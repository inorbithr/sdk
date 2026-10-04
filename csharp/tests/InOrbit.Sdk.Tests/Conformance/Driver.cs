using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Linq;
using System.Net.Http;
using System.Text;
using System.Text.Json;
using System.Threading.Tasks;
using InOrbit.Sdk.Api;
using Xunit;
using Xunit.Abstractions;

namespace InOrbit.Sdk.Tests.Conformance;

/// <summary>
/// The driver for <c>conformance/cases</c>: starts the replay server, loads every case, runs its
/// action through the generated public surface, and compares the result and the server's verdict
/// with <c>expect</c> (<c>conformance/README.md</c>). Without the server binary
/// (<c>mise run conformance:server:build</c>) it skips, unless IOHR_TEST_REQUIRE_REPLAY is set.
/// </summary>
public sealed class Driver(ITestOutputHelper output)
{
    private const string Self = "csharp";

    [Fact]
    [Trait("Category", "Conformance")]
    public async Task Every_case_passes()
    {
        var root = Path.GetFullPath(Path.Combine(AppContext.BaseDirectory, "../../../../../.."));
        var bin = Path.Combine(root, "conformance/server/bin/replay" + (OperatingSystem.IsWindows() ? ".exe" : string.Empty));
        if (!File.Exists(bin))
        {
            var note = "conformance: no replay server at conformance/server/bin/replay; run `mise run conformance:server:build`";
            Assert.True(Environment.GetEnvironmentVariable("IOHR_TEST_REQUIRE_REPLAY") is null, note);
            output.WriteLine(note + "; skipping");
            return;
        }

        using var replay = Process.Start(new ProcessStartInfo(bin)
        {
            ArgumentList = { "--addr", "127.0.0.1:0", "--cases", Path.Combine(root, "conformance/cases") },
            RedirectStandardOutput = true,
            UseShellExecute = false,
        })!;
        try
        {
            var first = await replay.StandardOutput.ReadLineAsync().WaitAsync(TimeSpan.FromSeconds(10)) ?? string.Empty;
            const string prefix = "replay: listening on ";
            Assert.StartsWith(prefix, first, StringComparison.Ordinal);
            var url = first[prefix.Length..].Trim();
            using var http = new HttpClient { BaseAddress = new Uri(url) };
            var cases = JsonDocument.Parse(await http.GetStringAsync("/_cases")).RootElement.GetProperty("cases")
                .EnumerateArray().Select(c => c.GetString()!).ToList();
            Assert.NotEmpty(cases);
            var failed = new List<string>();
            foreach (var name in cases)
            {
                using var loaded = await http.PostAsync("/_case", new StringContent(JsonSerializer.Serialize(new { name }), Encoding.UTF8, "application/json"));
                if ((int)loaded.StatusCode == 501)
                {
                    output.WriteLine($"skip {name}: not implemented by the replay server yet");
                    continue;
                }

                Assert.True(loaded.IsSuccessStatusCode, $"{name}: loading answered {(int)loaded.StatusCode}");
                var c = JsonDocument.Parse(await loaded.Content.ReadAsStringAsync()).RootElement.GetProperty("case");
                var area = c.GetProperty("area").GetString();
                var pending = c.TryGetProperty("pending", out var p) && p.EnumerateArray().Any(x => x.GetString() == Self);
                if (pending)
                {
                    output.WriteLine($"skip {name}: pending for {Self}");
                    continue;
                }

                var problems = await RunAsync(c, url, http);
                if (problems.Count == 0)
                {
                    output.WriteLine($"pass {name}");
                }
                else
                {
                    failed.Add($"{name}:\n  {string.Join("\n  ", problems)}");
                }
            }

            Assert.True(failed.Count == 0, string.Join("\n", failed));
        }
        finally
        {
            replay.Kill(entireProcessTree: true);
        }
    }

    private static async Task<List<string>> RunAsync(JsonElement c, string url, HttpClient http)
    {
        var o = c.TryGetProperty("client", out var co) ? co : default;
        string? Str(JsonElement e, string n) => e.ValueKind == JsonValueKind.Object && e.TryGetProperty(n, out var v) ? v.GetString() : null;
        int? Int(JsonElement e, string n) => e.ValueKind == JsonValueKind.Object && e.TryGetProperty(n, out var v) ? v.GetInt32() : null;
        var scopes = o.ValueKind == JsonValueKind.Object && o.TryGetProperty("scopes", out var s)
            ? s.EnumerateArray().Select(x => x.GetString()!).ToArray()
            : ["identity:read"];
        var timeout = Int(o, "timeout_ms");
        var idle = Int(o, "stream_idle_timeout_ms");
        using var client = new Client<PublicProfile>(new ClientOptions
        {
            BaseUrl = new Uri(url),
            TokenUrl = new Uri(url + "/oauth2/token"),
            KeyId = Str(o, "key_id") ?? "ak_test",
            KeySecret = Str(o, "key_secret") ?? "s3cr3t",
            Scopes = scopes,
            MaxRetries = Int(o, "max_retries") ?? 2,
            Timeout = timeout is { } ms ? TimeSpan.FromMilliseconds(ms) : null,
            Streams = Str(o, "streams") == "socket" ? StreamTransport.Socket : StreamTransport.Sse,
            StreamIdleTimeout = idle is { } im ? TimeSpan.FromMilliseconds(im) : null,
        });
        var action = c.GetProperty("action");
        var args = action.TryGetProperty("args", out var a) ? a : default;
        var expect = c.GetProperty("expect");
        if (action.GetProperty("op").GetString() == "events.stream_events")
        {
            return await StreamAsync(client, action, args, expect, http);
        }

        var results = new List<object>();
        async Task<object> Run()
        {
            try
            {
                return await CallAsync(client, action.GetProperty("op").GetString()!, args);
            }
            catch (InOrbitException e)
            {
                return e;
            }
        }

        if (action.TryGetProperty("concurrent", out var n))
        {
            results.AddRange(await Task.WhenAll(Enumerable.Range(0, n.GetInt32()).Select(_ => Run())));
        }
        else
        {
            var repeat = action.TryGetProperty("repeat", out var r) ? r.GetInt32() : 1;
            for (var i = 0; i < repeat; i++)
            {
                results.Add(await Run());
            }
        }

        var problems = await VerdictAsync(http, expect);

        var wantOk = expect.TryGetProperty("ok", out var ok) ? ok : (JsonElement?)null;
        var wantError = expect.TryGetProperty("error", out var err) ? err : (JsonElement?)null;
        foreach (var result in results)
        {
            switch (result)
            {
                case RawResponse raw when wantOk is { } w:
                    var body = raw.Json();
                    if (body is not { } got || !Subset(w, got))
                    {
                        problems.Add($"ok: want a superset of {w.GetRawText()}, got {raw.Text()}");
                    }

                    break;
                case RawResponse raw when wantError is not null:
                    problems.Add($"want an error, got HTTP {raw.Status}");
                    break;
                case InOrbitException e when wantError is { } w:
                    problems.AddRange(CheckError(e, w));
                    break;
                case InOrbitException e when wantOk is not null:
                    problems.Add($"want ok, got {e.Message}");
                    break;
            }
        }

        return problems;
    }

    /// <summary>The server's verdict and the counts the case expects.</summary>
    private static async Task<List<string>> VerdictAsync(HttpClient http, JsonElement expect)
    {
        var problems = new List<string>();
        JsonElement verdict = default;
        // A socket's last steps may still be running when the stream ends: wait for the server to see them.
        for (var i = 0; i < 50; i++)
        {
            verdict = JsonDocument.Parse(await http.GetStringAsync("/_result")).RootElement;
            if (verdict.GetProperty("status").GetString() != "incomplete")
            {
                break;
            }

            await Task.Delay(20);
        }

        if (verdict.GetProperty("status").GetString() != "pass")
        {
            problems.Add($"server: {verdict.GetRawText()}");
        }

        foreach (var key in new[] { "attempts", "token_exchanges" })
        {
            if (expect.TryGetProperty(key, out var want) && (!verdict.TryGetProperty(key, out var got) || got.GetInt32() != want.GetInt32()))
            {
                problems.Add($"{key}: want {want.GetInt32()}, got {(verdict.TryGetProperty(key, out var g) ? g.GetRawText() : "none")}");
            }
        }

        return problems;
    }

    /// <summary>A stream case: read the events (stopping after <c>take</c>), then compare the items and the error.</summary>
    private static async Task<List<string>> StreamAsync(Client<PublicProfile> client, JsonElement action, JsonElement args, JsonElement expect, HttpClient http)
    {
        var take = action.TryGetProperty("take", out var t) ? t.GetInt32() : int.MaxValue;
        var types = args.ValueKind == JsonValueKind.Object && args.TryGetProperty("types", out var ty) ? ty.GetString() : null;
        var items = new List<JsonElement>();
        InOrbitException? error = null;
        try
        {
            await foreach (var ev in client.Events().StreamEventsAsync(types is null ? null : new EventsStreamEventsParams { Types = types }))
            {
                items.Add(JsonSerializer.SerializeToElement(ev));
                if (items.Count >= take)
                {
                    break;
                }
            }
        }
        catch (InOrbitException e)
        {
            error = e;
        }

        var problems = await VerdictAsync(http, expect);
        if (expect.TryGetProperty("items", out var want))
        {
            if (want.GetArrayLength() != items.Count)
            {
                problems.Add($"items: want {want.GetArrayLength()}, got {items.Count}{(error is null ? string.Empty : $" then {error.Message}")}");
            }
            else
            {
                foreach (var (w, g) in want.EnumerateArray().Zip(items))
                {
                    if (!Subset(w, g))
                    {
                        problems.Add($"item: want a superset of {w.GetRawText()}, got {g.GetRawText()}");
                    }
                }
            }
        }

        if (expect.TryGetProperty("error", out var we))
        {
            if (error is null)
            {
                problems.Add("want an error, the stream ended cleanly");
            }
            else
            {
                problems.AddRange(CheckError(error, we));
            }
        }
        else if (error is not null)
        {
            problems.Add($"want a clean end, got {error.Message}");
        }

        return problems;
    }

    /// <summary>The case's action, through the generated public surface.</summary>
    private static async Task<object> CallAsync(Client<PublicProfile> client, string op, JsonElement args)
    {
        string Arg(string n) => args.ValueKind == JsonValueKind.Object && args.TryGetProperty(n, out var v) ? v.GetString() ?? string.Empty : string.Empty;
        string? Opt(string n) => args.ValueKind == JsonValueKind.Object && args.TryGetProperty(n, out var v) ? v.GetString() : null;
        return op switch
        {
            "me" => (await client.MeAsync()).Raw,
            "accounts.get_me" => (await client.Accounts().GetMeAsync()).Raw,
            "accounts.get_usage" => (await client.Accounts().GetUsageAsync(Arg("org_id"), new AccountsGetUsageParams { From = Opt("from"), To = Opt("to") })).Raw,
            "radar.get_digest" => (await client.Radar().GetDigestAsync(Arg("id"))).Raw,
            "events.create_endpoint" => (await client.Events().CreateEndpointAsync(new CreateEndpointRequest
            {
                AccountId = Opt("account_id"),
                Url = Opt("url"),
                Description = Opt("description"),
                EventTypes = args.TryGetProperty("event_types", out var types) ? types.EnumerateArray().Select(x => x.GetString()!).ToArray() : null,
            })).Raw,
            "events.delete_endpoint" => (await client.Events().DeleteEndpointAsync(Arg("endpoint_id"))).Raw,
            _ => throw new InvalidOperationException($"the conformance schema names an op this driver does not know: {op}"),
        };
    }

    private static IEnumerable<string> CheckError(InOrbitException e, JsonElement want)
    {
        string? Str(string n) => want.TryGetProperty(n, out var v) ? v.ToString() : null;
        if (Str("kind") is { } kind && e.Kind != kind)
        {
            yield return $"error kind: want {kind}, got {e.Kind} ({e.Message})";
        }

        if (e is ApiException api)
        {
            if (Str("code") is { } code && api.Code.Value != code)
            {
                yield return $"error code: want {code}, got {api.Code}";
            }

            if (want.TryGetProperty("status", out var st) && api.Status != st.GetInt32())
            {
                yield return $"error status: want {st.GetInt32()}, got {api.Status}";
            }
        }
        else if (Str("code") is not null || want.TryGetProperty("status", out _))
        {
            yield return $"error: want an API error, got {e.Message}";
        }

        if (Str("message_contains") is { } contains && !e.Message.Contains(contains, StringComparison.Ordinal))
        {
            yield return $"message: want it to contain \"{contains}\", got \"{e.Message}\"";
        }

        if (Str("message_excludes") is { } excludes && e.Message.Contains(excludes, StringComparison.Ordinal))
        {
            yield return $"message: must not contain \"{excludes}\", got \"{e.Message}\"";
        }
    }

    private static bool Subset(JsonElement want, JsonElement got) => want.ValueKind switch
    {
        JsonValueKind.Object => got.ValueKind == JsonValueKind.Object &&
            want.EnumerateObject().All(p => got.TryGetProperty(p.Name, out var g) && Subset(p.Value, g)),
        JsonValueKind.Array => got.ValueKind == JsonValueKind.Array && want.GetArrayLength() == got.GetArrayLength() &&
            want.EnumerateArray().Zip(got.EnumerateArray()).All(x => Subset(x.First, x.Second)),
        _ => want.GetRawText() == got.GetRawText() ||
            (want.ValueKind == JsonValueKind.String && got.ValueKind == JsonValueKind.String && want.GetString() == got.GetString()),
    };
}
