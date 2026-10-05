using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text.Json;
using System.Text.Json.Nodes;
using Xunit;

namespace InOrbit.Sdk.Tests.Conformance;

/// <summary>
/// The pure-function vectors of <c>conformance/vectors/</c> (docs/config.md section 9.2): configuration
/// resolution, config file paths, <c>no_proxy</c>, rate-limit headers and durations, each run against the
/// runtime without a server.
/// </summary>
public sealed class Vectors
{
    private const string Self = "csharp";

    private static IEnumerable<JsonObject> Load(string kind) =>
        Directory.GetFiles(Path.Combine(Support.Root, "conformance", "vectors", kind), "*.yaml")
            .Order(StringComparer.Ordinal)
            .Select(f => Support.Yaml(f)!.AsObject())
            .Where(v => !(v["pending"] is JsonArray p && p.Any(x => x!.GetValue<string>() == Self)));

    private static HostOs Os(JsonNode? os) => os?.GetValue<string>() switch
    {
        "macos" => HostOs.MacOS,
        "windows" => HostOs.Windows,
        _ => HostOs.Linux,
    };

    private static Dictionary<string, string> Env(JsonNode? env, Func<string, string>? map = null) =>
        (env as JsonObject ?? []).ToDictionary(kv => kv.Key, kv => (map ?? (x => x))(kv.Value?.GetValue<string>() ?? string.Empty), StringComparer.Ordinal);

    [Fact]
    public void Config()
    {
        var failures = new List<string>();
        var count = 0;
        foreach (var v in Load("config"))
        {
            count++;
            var dir = Directory.CreateTempSubdirectory("inorbit-vector-").FullName;
            try
            {
                if (RunConfig(v, dir) is { } problem)
                {
                    failures.Add($"{v["name"]}: {problem}");
                }
            }
            finally
            {
                Directory.Delete(dir, recursive: true);
            }
        }

        Assert.True(count > 0, "no config vectors ran");
        Assert.True(failures.Count == 0, string.Join("\n", failures));
    }

    private static string? RunConfig(JsonObject v, string dir)
    {
        var input = v["input"] as JsonObject ?? [];
        var env = Env(input["env"], x => x.Replace("{dir}", dir, StringComparison.Ordinal));
        var os = Os(input["os"]);
        var home = input["home"]?.GetValue<bool>() == true ? Path.Combine(dir, "home") : null;
        if (home is not null)
        {
            Directory.CreateDirectory(home);
        }

        foreach (var (name, content) in input["files"] as JsonObject ?? [])
        {
            var p = Path.Combine(dir, name);
            Directory.CreateDirectory(Path.GetDirectoryName(p)!);
            File.WriteAllText(p, content!.GetValue<string>());
        }

        var options = Support.Options(input["code"] as JsonObject);
        var file = Path.Combine(dir, "config.toml");
        if (input["config_file"]?.GetValue<string>() is { } text)
        {
            if (home is not null)
            {
                var located = new Dictionary<string, string>(env, StringComparer.Ordinal);
                located.Remove("INORBIT_CONFIG_FILE");
                var at = Settings.ConfigPath(os, located, home, null) ?? throw new InvalidOperationException("no default location");
                Directory.CreateDirectory(Path.GetDirectoryName(at.Path)!);
                File.WriteAllText(at.Path, text);
                file = at.Path;
            }
            else
            {
                File.WriteAllText(file, text);
                options = options with { ConfigFile = file };
            }
        }

        if (input["cli"]?.GetValue<string>() == "present")
        {
            env["PATH"] = Support.FakeIohr(dir);
        }

        var load = new LoadOptions { Environment = env, Os = os, Home = home, NoHome = home is null, WorkingDirectory = dir };
        // The expectation is JSON text: a Windows path goes in escaped, or its \ starts an escape.
        static string Esc(string path) => JsonSerializer.Serialize(path)[1..^1];
        string Sub(string s) => s.Replace("{file}", Esc(file), StringComparison.Ordinal).Replace("{dir}", Esc(dir), StringComparison.Ordinal);
        var want = JsonNode.Parse(Sub((v["expect"] ?? new JsonObject()).ToJsonString()))!.AsObject();
        JsonObject? doc = null;
        ConfigException? error = null;
        try
        {
            var res = Transport.Resolve(options, input["profile_type"]?.GetValue<string>(), load, explicitly: false);
            doc = res.Describe(Transport.PipelineNames(options)).Describe();
        }
        catch (ConfigException e)
        {
            error = e;
        }

        var shown = doc?.ToJsonString() ?? $"{error!.Message} {JsonSerializer.Serialize(error.Problems)}";
        foreach (var x in want["excludes"] as JsonArray ?? [])
        {
            if (shown.Contains(x!.GetValue<string>(), StringComparison.Ordinal))
            {
                return $"{x} appears in {shown}";
            }
        }

        if (want["error"] is JsonObject we)
        {
            if (error is null)
            {
                return $"want a ConfigException, got {shown}";
            }

            if (we["problems"] is JsonArray problems)
            {
                if (problems.Count != error.Problems.Count)
                {
                    return $"want {problems.Count} problems, got {JsonSerializer.Serialize(error.Problems)}";
                }

                for (var i = 0; i < problems.Count; i++)
                {
                    var p = problems[i]!.AsObject();
                    var have = error.Problems[i];
                    if (p["setting"]?.GetValue<string>() is { } setting && setting != have.Setting)
                    {
                        return $"problem {i}: setting {setting} != {have}";
                    }

                    if (p["source"]?.GetValue<string>() is { } source && Support.Slash(source) != Support.Slash(have.Source))
                    {
                        return $"problem {i}: source {source} != {have}";
                    }

                    if (p["message_contains"]?.GetValue<string>() is { } contains && !have.Message.Contains(contains, StringComparison.Ordinal))
                    {
                        return $"problem {i}: message lacks {contains}: {have}";
                    }
                }
            }

            foreach (var part in we["message_contains"] as JsonArray ?? [])
            {
                if (!error.Message.Contains(part!.GetValue<string>(), StringComparison.Ordinal))
                {
                    return $"the message lacks {part}:\n{error.Message}";
                }
            }

            return null;
        }

        if (doc is null)
        {
            return $"unexpected error: {shown}";
        }

        foreach (var key in new[] { "profile", "settings", "credential", "pipeline" })
        {
            if (want.ContainsKey(key) && Support.Subset(want[key], doc[key], key) is { } p)
            {
                return p;
            }
        }

        if (want.ContainsKey("config_file") && Support.Subset(want["config_file"], doc["config_file"], "config_file") is { } cf)
        {
            return cf;
        }

        var settings = doc["settings"]!.AsObject();
        foreach (var a in want["settings_absent"] as JsonArray ?? [])
        {
            if (settings.ContainsKey(a!.GetValue<string>()))
            {
                return $"{a} should not be in settings";
            }
        }

        var ignored = doc["ignored"]!.AsArray();
        foreach (var w in want["ignored"] as JsonArray ?? [])
        {
            if (!ignored.Any(h => Support.Subset(w, h) is null))
            {
                return $"ignored lacks {w!.ToJsonString()}: {ignored.ToJsonString()}";
            }
        }

        return null;
    }

    [Fact]
    public void ConfigPath()
    {
        foreach (var v in Load("config-path"))
        {
            foreach (var c in v["checks"]!.AsArray().Select(x => x!.AsObject()))
            {
                var got = Settings.ConfigPath(Os(c["os"]), Env(c["env"]), c["home"]?.GetValue<string>(), c["code"]?["config_file"]?.GetValue<string>());
                Assert.True(c["expect"]?.GetValue<string>() == got?.Path, $"{v["name"]}: {c["summary"]}: want {c["expect"]}, got {got?.Path}");
            }
        }
    }

    [Fact]
    public void Durations()
    {
        foreach (var v in Load("durations"))
        {
            foreach (var c in v["checks"]!.AsArray().Select(x => x!.AsObject()))
            {
                var value = c["value"]!.ToString();
                var got = Settings.ParseDuration(value) is { } d ? ((long)d.TotalMilliseconds).ToString(System.Globalization.CultureInfo.InvariantCulture) : "error";
                Assert.True(c["expect"]!.ToString() == got, $"{v["name"]}: {value}: want {c["expect"]}, got {got}");
            }
        }
    }

    [Fact]
    public void NoProxy()
    {
        foreach (var v in Load("no-proxy"))
        {
            foreach (var c in v["checks"]!.AsArray().Select(x => x!.AsObject()))
            {
                var what = $"{v["name"]}: {c["summary"]}";
                var env = Env(c["env"]);
                env["INORBIT_TOKEN"] = "t";
                env["INORBIT_CONFIG_FILE"] = "off";
                var wantError = c["expect"] is JsonObject;
                Resolution res;
                try
                {
                    res = Transport.Resolve(Support.Options(c["code"] as JsonObject), null, new LoadOptions { Environment = env, Os = HostOs.Linux, NoHome = true, WorkingDirectory = "/" }, explicitly: false);
                }
                catch (ConfigException e)
                {
                    Assert.True(wantError, $"{what}: {e.Message}");
                    continue;
                }

                Assert.False(wantError, $"{what}: want a ConfigException");
                var noProxy = Sdk.NoProxy.Parse(res.Get<string[]>("no_proxy") ?? []);
                var got = Sdk.NoProxy.For(new Uri(c["url"]!.GetValue<string>()), res.Proxy, noProxy)?.GetLeftPart(UriPartial.Authority);
                Assert.True(c["expect"]?.GetValue<string>() == got, $"{what}: want {c["expect"]}, got {got}");
            }
        }
    }

    [Fact]
    public void RateLimitHeaders()
    {
        foreach (var v in Load("rate-limit"))
        {
            foreach (var c in v["checks"]!.AsArray().Select(x => x!.AsObject()))
            {
                var headers = (c["headers"] as JsonObject ?? []).ToDictionary(kv => kv.Key.ToLowerInvariant(), kv => (IReadOnlyList<string>)[kv.Value!.GetValue<string>()], StringComparer.Ordinal);
                var s = RateLimit.Read(headers);
                JsonObject? got = null;
                if (s is not null)
                {
                    got = [];
                    if (s.Limit is { } l)
                    {
                        got["limit"] = l;
                    }

                    if (s.Remaining is { } r)
                    {
                        got["remaining"] = r;
                    }

                    if (s.Reset is { } reset)
                    {
                        got["reset_ms"] = (long)reset.TotalMilliseconds;
                    }

                    if (s.Policy is { } p)
                    {
                        var policy = new JsonObject { ["name"] = p.Name };
                        if (p.Quota is { } q)
                        {
                            policy["quota"] = q;
                        }

                        if (p.Window is { } w)
                        {
                            policy["window_ms"] = (long)w.TotalMilliseconds;
                        }

                        got["policy"] = policy;
                    }
                }

                var want = c["expect"];
                Assert.True(JsonNode.DeepEquals(want, got), $"{v["name"]}: {c["summary"]}: want {want?.ToJsonString() ?? "null"}, got {got?.ToJsonString() ?? "null"}");
            }
        }
    }
}
