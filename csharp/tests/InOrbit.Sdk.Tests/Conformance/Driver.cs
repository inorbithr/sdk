using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Linq;
using System.Net.Http;
using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;
using System.Threading.Tasks;
using InOrbit.Sdk.Api;
using Microsoft.Extensions.Logging;
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

    private static string ReplayBin { get; } = Path.Combine(Support.Root, "conformance/server/bin/replay" + (OperatingSystem.IsWindows() ? ".exe" : string.Empty));

    [Fact]
    [Trait("Category", "Conformance")]
    public async Task Every_case_passes()
    {
        if (!File.Exists(ReplayBin))
        {
            var note = "conformance: no replay server at conformance/server/bin/replay; run `mise run conformance:server:build`";
            Assert.True(Environment.GetEnvironmentVariable("IOHR_TEST_REQUIRE_REPLAY") is null, note);
            output.WriteLine(note + "; skipping");
            return;
        }

        using var replay = Process.Start(new ProcessStartInfo(ReplayBin)
        {
            ArgumentList = { "--addr", "127.0.0.1:0", "--cases", Path.Combine(Support.Root, "conformance/cases") },
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
            var passed = 0;
            foreach (var name in cases)
            {
                using var loaded = await http.PostAsync("/_case", new StringContent(JsonSerializer.Serialize(new { name }), Encoding.UTF8, "application/json"));
                if ((int)loaded.StatusCode == 501)
                {
                    output.WriteLine($"skip {name}: not implemented by the replay server yet");
                    continue;
                }

                Assert.True(loaded.IsSuccessStatusCode, $"{name}: loading answered {(int)loaded.StatusCode}");
                var answer = JsonNode.Parse(await loaded.Content.ReadAsStringAsync())!.AsObject();
                var c = answer["case"]!.AsObject();
                if (c["pending"] is JsonArray pending && pending.Any(x => x!.GetValue<string>() == Self))
                {
                    output.WriteLine($"skip {name}: pending for {Self}");
                    continue;
                }

                List<string> problems;
                try
                {
                    problems = await RunAsync(answer, c, http);
                }
                catch (InOrbitException e)
                {
                    problems = [$"building the client: {e.Message}"];
                }

                if (problems.Count == 0)
                {
                    passed++;
                    output.WriteLine($"pass {name}");
                }
                else
                {
                    failed.Add($"{name}:\n  {string.Join("\n  ", problems)}");
                }
            }

            output.WriteLine($"conformance: {passed} passed, {failed.Count} failed");
            Assert.True(failed.Count == 0, string.Join("\n", failed));
        }
        finally
        {
            Stop(replay);
        }
    }

    /// <summary>Stops the server with SIGTERM, so it removes its certificate directory; SIGKILL only if it lingers.</summary>
    private static void Stop(Process replay)
    {
        if (!OperatingSystem.IsWindows() && !replay.HasExited)
        {
            using var term = Process.Start(new ProcessStartInfo("kill") { ArgumentList = { "-TERM", replay.Id.ToString(System.Globalization.CultureInfo.InvariantCulture) }, UseShellExecute = false });
            term?.WaitForExit(5000);
            if (replay.WaitForExit(5000))
            {
                return;
            }
        }

        replay.Kill(entireProcessTree: true);
    }

    private static string Substitute(string text, string replay, string dir) => text.Replace("{replay}", replay, StringComparison.Ordinal).Replace("{dir}", dir, StringComparison.Ordinal);

    private static async Task<List<string>> RunAsync(JsonObject answer, JsonObject c, HttpClient http)
    {
        var o = c["client"] as JsonObject ?? [];
        var baseUrl = answer["base_url"]!.GetValue<string>();
        var dir = Directory.CreateTempSubdirectory("inorbit-case-").FullName;
        try
        {
            using var built = Build(answer, o, baseUrl, dir);
            var action = c["action"]!.AsObject();
            var args = action["args"] as JsonObject ?? [];
            var expect = c["expect"]!.AsObject();
            if (action["op"]!.GetValue<string>() == "events.stream_events")
            {
                return await StreamAsync(built.Client, action, args, expect, http);
            }

            var client = built.Client;
            if (action["options"] is JsonObject opts)
            {
                client = client.WithOptions(new CallOptions
                {
                    IdempotencyKey = opts["idempotency_key"]?.GetValue<string>(),
                    Traceparent = opts["traceparent"]?.GetValue<string>(),
                    Timeout = opts["timeout_ms"] is { } ms ? TimeSpan.FromMilliseconds(ms.GetValue<long>()) : null,
                });
            }

            var results = new List<object>();
            async Task<object> Run()
            {
                try
                {
                    return await CallAsync(client, action["op"]!.GetValue<string>(), args);
                }
                catch (InOrbitException e)
                {
                    return e;
                }
            }

            using (built.Root?.Start())
            {
                if (action["concurrent"] is { } n)
                {
                    results.AddRange(await Task.WhenAll(Enumerable.Range(0, (int)n.GetValue<long>()).Select(_ => Run())));
                }
                else
                {
                    var repeat = action["repeat"] is { } r ? (int)r.GetValue<long>() : 1;
                    var rewrite = action["rewrite"] as JsonObject;
                    for (var i = 0; i < repeat; i++)
                    {
                        results.Add(await Run());
                        if (rewrite is not null && i + 1 == rewrite["after"]!.GetValue<long>())
                        {
                            foreach (var (file, content) in rewrite["files"]!.AsObject())
                            {
                                await File.WriteAllTextAsync(Path.Combine(dir, file), content!.GetValue<string>());
                            }
                        }
                    }
                }
            }

            var problems = await VerdictAsync(http, expect);
            var wantOk = expect["ok"];
            var wantError = expect["error"] as JsonObject;
            foreach (var result in results)
            {
                switch (result)
                {
                    case RawResponse raw when wantOk is not null:
                        var body = raw.Text();
                        if (Support.Subset(wantOk, JsonNode.Parse(body)) is { } p)
                        {
                            problems.Add($"ok: want a superset of {wantOk.ToJsonString()}, got {body} ({p})");
                        }

                        break;
                    case RawResponse raw when wantError is not null:
                        problems.Add($"want an error, got HTTP {raw.Status}");
                        break;
                    case InOrbitException e when wantError is not null:
                        problems.AddRange(CheckError(e, wantError));
                        break;
                    case InOrbitException e when wantOk is not null:
                        problems.Add($"want ok, got {e.Message}");
                        break;
                }
            }

            problems.AddRange(CheckM6(expect, built, results.LastOrDefault()));
            return problems;
        }
        finally
        {
            try
            {
                Directory.Delete(dir, recursive: true);
            }
            catch (IOException)
            {
                // A file still held open on Windows: the temporary directory goes later.
            }
        }
    }

    /// <summary>The client a case describes, and what it records: probes, logs, spans.</summary>
    private static Built Build(JsonObject answer, JsonObject o, string baseUrl, string dir)
    {
        string? Str(string n) => o[n]?.GetValue<string>();
        long? Long(string n) => o[n]?.GetValue<long>();
        foreach (var (name, content) in o["files"] as JsonObject ?? [])
        {
            var p = Path.Combine(dir, name);
            Directory.CreateDirectory(Path.GetDirectoryName(p)!);
            File.WriteAllText(p, Substitute(content!.GetValue<string>(), baseUrl, dir));
        }

        var built = new Built();
        var load = o["load"]?.GetValue<bool>() == true;
        var transport = Str("transport");
        var tracing = o["tracing"]?.GetValue<bool>();
        var options = new ClientOptions
        {
            MaxRetries = Long("max_retries") is { } mr ? (int)mr : load ? null : 2,
            Timeout = Long("timeout_ms") is { } ms ? TimeSpan.FromMilliseconds(ms) : null,
            Streams = Str("streams") switch { "socket" => StreamTransport.Socket, "sse" => StreamTransport.Sse, _ => null },
            StreamIdleTimeout = Long("stream_idle_timeout_ms") is { } im ? TimeSpan.FromMilliseconds(im) : null,
            Log = Str("log") switch
            {
                null => null,
                "debug" => LogLevel.Debug,
                "info" => LogLevel.Information,
                "warn" => LogLevel.Warning,
                "error" => LogLevel.Error,
                _ => LogLevel.None,
            },
            LoggerFactory = Str("log") is null ? null : built.Logs,
            LogHeaders = o["log_headers"]?.GetValue<bool>(),
            LogAllowHeaders = (o["log_allow_headers"] as JsonArray)?.Select(x => x!.GetValue<string>()).ToArray(),
            RateLimit = Str("rate_limit") switch { "wait" => RateLimitMode.Wait, "off" => RateLimitMode.Off, "observe" => RateLimitMode.Observe, _ => null },
            TotalTimeout = Long("total_timeout_ms") is { } tt ? TimeSpan.FromMilliseconds(tt) : null,
            RetryBudgetCapacity = Long("retry_budget_capacity") is { } cap ? (int)cap : null,
            Tracing = tracing,
            CaBundle = transport is "https" or "proxy" or "mtls" ? answer["ca_file"]!.GetValue<string>() : null,
            ClientCert = transport == "mtls" ? answer["client_cert_file"]!.GetValue<string>() : null,
            ClientKey = transport == "mtls" ? answer["client_key_file"]!.GetValue<string>() : null,
            Proxy = transport == "proxy" ? answer["proxy_url"]!.GetValue<string>() : null,
            NoProxy = Str("no_proxy") is { } np ? np.Split(',') : null,
            CredentialSources = (o["credential_sources"] as JsonArray)?.Select(x => x!.GetValue<string>()).ToArray(),
            CliPath = o["cli"]?.GetValue<bool>() == true ? ReplayBin : null,
            Pipeline = o["pipeline"] is JsonObject pipeline ? p => Edit(p, pipeline, built.Probes) : null,
        };
        if (tracing == true)
        {
            built.Listen();
        }

        if (load)
        {
            var env = (o["env"] as JsonObject ?? []).ToDictionary(kv => kv.Key, kv => Substitute(kv.Value!.GetValue<string>(), baseUrl, dir), StringComparer.Ordinal);
            string? configFile = null;
            if (Str("config_file") is { } text)
            {
                configFile = Path.Combine(dir, "config.toml");
                File.WriteAllText(configFile, Substitute(text, baseUrl, dir));
            }

            built.Client = Client.Load(
                options with { ConfigFile = configFile, Profile = Str("profile") },
                new LoadOptions { Environment = env, NoHome = true, WorkingDirectory = dir });
        }
        else
        {
            built.Client = new Client<PublicProfile>(options with
            {
                BaseUrl = new Uri(baseUrl),
                TokenUrl = new Uri(baseUrl + "/oauth2/token"),
                KeyId = Str("key_id") ?? "ak_test",
                KeySecret = Str("key_secret") ?? "s3cr3t",
                Scopes = (o["scopes"] as JsonArray)?.Select(x => x!.GetValue<string>()).ToArray() ?? ["identity:read"],
            });
        }

        return built;
    }

    /// <summary>Adds the case's probes and removes the built-ins it names.</summary>
    private static void Edit(Pipeline p, JsonObject pipeline, Dictionary<string, List<Dictionary<string, string>>> probes)
    {
        foreach (var add in pipeline["add"] as JsonArray ?? [])
        {
            var name = add!["name"]!.GetValue<string>();
            var seen = new List<Dictionary<string, string>>();
            probes[name] = seen;
            var probe = Middleware.Create(name, (req, next, ct) =>
            {
                lock (seen)
                {
                    seen.Add(new Dictionary<string, string>(req.Headers, StringComparer.OrdinalIgnoreCase));
                }

                return next(req, ct);
            });
            if (add["before"]?.GetValue<string>() is { } before)
            {
                p.InsertBefore(before, probe);
            }
            else if (add["after"]?.GetValue<string>() is { } after)
            {
                p.InsertAfter(after, probe);
            }
            else if (add["stage"]?.GetValue<string>() == "per_call")
            {
                p.AddPerCall(probe);
            }
            else
            {
                p.AddPerRetry(probe);
            }
        }

        foreach (var remove in pipeline["remove"] as JsonArray ?? [])
        {
            p.Remove(remove!.GetValue<string>());
        }
    }

    /// <summary>What the M6 expectations add: probes, logs, spans, the rate limit, the key, the config.</summary>
    private static List<string> CheckM6(JsonObject expect, Built built, object? last)
    {
        var problems = new List<string>();
        foreach (var (name, want) in expect["probes"] as JsonObject ?? [])
        {
            var seen = built.Probes.TryGetValue(name, out var s) ? s : [];
            if (want!["count"] is { } count && seen.Count != count.GetValue<long>())
            {
                problems.Add($"probe {name}: ran {seen.Count} times, want {count}");
            }

            var captures = new Dictionary<string, string>(StringComparer.Ordinal);
            var i = 0;
            foreach (var headers in want["seen"] as JsonArray ?? [])
            {
                foreach (var (h, v) in headers!.AsObject())
                {
                    var got = i < seen.Count && seen[i].TryGetValue(h, out var g) ? g : null;
                    if (!Support.Matches(v!.GetValue<string>(), got, captures))
                    {
                        problems.Add($"probe {name} request {i + 1}: {h} is {got ?? "absent"}, want {v}");
                    }
                }

                i++;
            }
        }

        if (expect["logs"] is JsonObject logs)
        {
            var records = built.Logs.Records;
            var text = string.Join("\n", records.Select(r => r.ToJsonString()));
            foreach (var w in logs["contains"] as JsonArray ?? [])
            {
                if (!records.Any(r => Support.Subset(w, r) is null))
                {
                    problems.Add($"logs: no record holds {w!.ToJsonString()}: {text}");
                }
            }

            foreach (var x in logs["excludes"] as JsonArray ?? [])
            {
                if (text.Contains(x!.GetValue<string>(), StringComparison.Ordinal))
                {
                    problems.Add($"logs: {x} appears in {text}");
                }
            }
        }

        if (expect["spans"] is JsonArray spans)
        {
            var got = built.Spans();
            if (got.Count != spans.Count)
            {
                problems.Add($"spans: want {spans.Count}, got {got.Count}: {string.Join("; ", got.Select(Describe))}");
            }
            else
            {
                for (var i = 0; i < spans.Count; i++)
                {
                    var w = spans[i]!.AsObject();
                    var a = got[i];
                    var kind = w["kind"]?.GetValue<string>() switch { "internal" => ActivityKind.Internal, "client" => ActivityKind.Client, _ => a.Kind };
                    var attributes = new JsonObject(a.TagObjects.Select(t => KeyValuePair.Create(t.Key, t.Value is null ? null : JsonSerializer.SerializeToNode(t.Value, t.Value.GetType()))));
                    if (a.DisplayName != w["name"]!.GetValue<string>() || a.Kind != kind || Support.Subset(w["attributes"] ?? new JsonObject(), attributes) is not null)
                    {
                        problems.Add($"span {i}: want {w.ToJsonString()}, got {Describe(a)}");
                    }
                }
            }
        }

        var raw = last as RawResponse;
        if (expect.ContainsKey("rate_limit"))
        {
            var r = raw?.RateLimit;
            JsonObject? got = null;
            if (r is not null)
            {
                got = [];
                if (r.Limit is { } l)
                {
                    got["limit"] = l;
                }

                if (r.Remaining is { } rem)
                {
                    got["remaining"] = rem;
                }

                if (r.Reset is { } reset)
                {
                    got["reset_ms"] = (long)reset.TotalMilliseconds;
                }
            }

            if (Support.Subset(expect["rate_limit"], got) is { } p)
            {
                problems.Add($"rate_limit: {p}");
            }
        }

        if (expect["idempotency_key"]?.GetValue<string>() is { } key)
        {
            var k = raw?.IdempotencyKey;
            if (key == "*" ? string.IsNullOrEmpty(k) : k != key)
            {
                problems.Add($"idempotency_key: want {key}, got {k ?? "none"}");
            }
        }

        if (expect["config"] is JsonObject config && Support.Subset(config, built.Client.Config.Describe()) is { } cp)
        {
            problems.Add($"config: {cp} in {built.Client.Config}");
        }

        return problems;
    }

    private static string Describe(Activity a) =>
        $"{a.DisplayName} ({a.Kind}) {{{string.Join(", ", a.TagObjects.Select(t => $"{t.Key}={t.Value}"))}}}";

    /// <summary>The server's verdict and the counts the case expects.</summary>
    private static async Task<List<string>> VerdictAsync(HttpClient http, JsonObject expect)
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
            if (expect[key] is { } want && (!verdict.TryGetProperty(key, out var got) || got.GetInt64() != want.GetValue<long>()))
            {
                problems.Add($"{key}: want {want}, got {(verdict.TryGetProperty(key, out var g) ? g.GetRawText() : "none")}");
            }
        }

        return problems;
    }

    /// <summary>A stream case: read the events (stopping after <c>take</c>), then compare the items and the error.</summary>
    private static async Task<List<string>> StreamAsync(Client<PublicProfile> client, JsonObject action, JsonObject args, JsonObject expect, HttpClient http)
    {
        var take = action["take"] is { } t ? t.GetValue<long>() : long.MaxValue;
        var types = args["types"]?.GetValue<string>();
        var items = new List<JsonNode?>();
        InOrbitException? error = null;
        try
        {
            await foreach (var ev in client.Events().StreamEventsAsync(types is null ? null : new EventsStreamEventsParams { Types = types }))
            {
                items.Add(JsonSerializer.SerializeToNode(ev));
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
        if (expect["items"] is JsonArray want)
        {
            if (want.Count != items.Count)
            {
                problems.Add($"items: want {want.Count}, got {items.Count}{(error is null ? string.Empty : $" then {error.Message}")}");
            }
            else
            {
                for (var i = 0; i < want.Count; i++)
                {
                    if (Support.Subset(want[i], items[i]) is { } p)
                    {
                        problems.Add($"item: want a superset of {want[i]!.ToJsonString()}, got {items[i]?.ToJsonString()} ({p})");
                    }
                }
            }
        }

        if (expect["error"] is JsonObject we)
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
    private static async Task<object> CallAsync(Client<PublicProfile> client, string op, JsonObject args)
    {
        string Arg(string n) => args[n]?.GetValue<string>() ?? string.Empty;
        string? Opt(string n) => args[n]?.GetValue<string>();
        string[]? List(string n) => (args[n] as JsonArray)?.Select(x => x!.GetValue<string>()).ToArray();
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
                EventTypes = List("event_types"),
            })).Raw,
            "events.update_endpoint" => (await client.Events().UpdateEndpointAsync(Arg("endpoint_id"), new UpdateEndpointRequest
            {
                Url = Opt("url"),
                Description = Opt("description"),
                Enabled = args["enabled"]?.GetValue<bool>(),
                EventTypes = List("event_types"),
            })).Raw,
            "events.delete_endpoint" => (await client.Events().DeleteEndpointAsync(Arg("endpoint_id"))).Raw,
            _ => throw new InvalidOperationException($"the conformance schema names an op this driver does not know: {op}"),
        };
    }

    private static IEnumerable<string> CheckError(InOrbitException e, JsonObject want)
    {
        string? Str(string n) => want[n] is { } v ? (v.GetValueKind() == JsonValueKind.String ? v.GetValue<string>() : v.ToJsonString()) : null;
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

            if (want["status"] is { } st && api.Status != st.GetValue<long>())
            {
                yield return $"error status: want {st}, got {api.Status}";
            }
        }
        else if (Str("code") is not null || want.ContainsKey("status"))
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

    /// <summary>One case's client and what it recorded.</summary>
    private sealed class Built : IDisposable
    {
        private readonly List<Activity> _spans = [];
        private ActivityListener? _listener;

        internal Client<PublicProfile> Client { get; set; } = null!;

        internal CaptureLogs Logs { get; } = new();

        internal Dictionary<string, List<Dictionary<string, string>>> Probes { get; } = new(StringComparer.Ordinal);

        /// <summary>The case's own root activity, so its spans are told apart from other tests' (an in-memory exporter).</summary>
        internal Activity? Root { get; private set; }

        internal void Listen()
        {
            Root = new Activity("conformance").SetIdFormat(ActivityIdFormat.W3C);
            _listener = new ActivityListener
            {
                ShouldListenTo = s => s.Name == "InOrbit.Sdk",
                Sample = (ref ActivityCreationOptions<ActivityContext> _) => ActivitySamplingResult.AllDataAndRecorded,
                ActivityStarted = a =>
                {
                    lock (_spans)
                    {
                        _spans.Add(a);
                    }
                },
            };
            ActivitySource.AddActivityListener(_listener);
        }

        /// <summary>This case's spans, in start order.</summary>
        internal List<Activity> Spans()
        {
            var trace = Root?.TraceId.ToHexString();
            lock (_spans)
            {
                return _spans.Where(a => a.TraceId.ToHexString() == trace).ToList();
            }
        }

        public void Dispose()
        {
            Client?.Dispose();
            _listener?.Dispose();
        }
    }
}
