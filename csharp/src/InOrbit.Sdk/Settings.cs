using System;
using System.Collections;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;
using System.Text.RegularExpressions;
using Tomlyn;
using Tomlyn.Model;

namespace InOrbit.Sdk;

/// <summary>The operating system whose file locations resolution follows (docs/config.md section 4.1).</summary>
public enum HostOs
{
    /// <summary>Linux and other Unix: <c>$XDG_CONFIG_HOME/iohr</c> or <c>~/.config/iohr</c>.</summary>
    Linux,

    /// <summary>macOS: <c>~/Library/Application Support/hr.InOrbit.iohr</c>.</summary>
    MacOS,

    /// <summary>Windows: <c>%APPDATA%\InOrbit\iohr\config</c>.</summary>
    Windows,
}

/// <summary>
/// What <see cref="Client.Load(ClientOptions?, LoadOptions?)"/> reads instead of the process: an
/// environment, an operating system, a home and a working directory. For tests, yours included,
/// so a configuration resolves without touching the process environment (docs/config.md section 9.2).
/// </summary>
public sealed record LoadOptions
{
    /// <summary>The environment (default: the process's). An empty value counts as unset.</summary>
    public IReadOnlyDictionary<string, string>? Environment { get; init; }

    /// <summary>Whose file locations apply (default: this machine's).</summary>
    public HostOs? Os { get; init; }

    /// <summary>The home directory (default: this machine's).</summary>
    public string? Home { get; init; }

    /// <summary>Resolve as if there were no home directory (a sandbox, a distroless container).</summary>
    public bool NoHome { get; init; }

    /// <summary>The directory relative paths from code and the environment resolve against (default: the current one).</summary>
    public string? WorkingDirectory { get; init; }
}

/// <summary>One problem a <see cref="ConfigException"/> found (docs/config.md section 2.5).</summary>
/// <param name="Setting">The setting, by its catalogue name (<c>timeout</c>), or <c>credential</c> when none was found.</param>
/// <param name="Source">Where the value came from (<c>env INORBIT_TIMEOUT</c>, <c>file &lt;path&gt; [sdk]</c>, <c>code</c>), or empty.</param>
/// <param name="Message">What is wrong and what to do; never a secret's value.</param>
public sealed record ConfigProblem(string Setting, string Source, string Message);

/// <summary>
/// A client's effective configuration (docs/config.md section 2.6): each setting with its value and
/// where it came from, the credential chosen and what was tried, the pipeline, and what was ignored.
/// Secrets are redacted.
/// </summary>
public sealed class ResolvedConfig
{
    private readonly JsonObject _doc;

    internal ResolvedConfig(JsonObject doc)
    {
        _doc = doc;
    }

    /// <summary>The configuration profile resolved for, if any.</summary>
    public string? Profile => _doc["profile"]?["name"]?.GetValue<string>();

    /// <summary>The <c>describe()</c> document, a fresh copy each time; stable within a major version.</summary>
    /// <returns>The document.</returns>
    public JsonObject Describe() => (JsonObject)_doc.DeepClone();

    /// <summary>The document as indented JSON, the same as <c>iohr sdk config</c> prints.</summary>
    /// <returns>The JSON text.</returns>
    public override string ToString() => _doc.ToJsonString(new JsonSerializerOptions { WriteIndented = true });
}

/// <summary>Configuration resolution (docs/config.md sections 2 to 5): each setting from code, the environment, the config file or its default, and the credential chain.</summary>
internal static partial class Settings
{
    /// <summary>The built-in pipeline, outermost first (docs/config.md section 7.2).</summary>
    internal static readonly string[] BuiltIns =
    [
        "request_id", "user_agent", "idempotency_key", "call_tracing", "deadline", "retry",
        "auth", "rate_limit", "attempt_tracing", "logging", "hooks", "timeout",
    ];

    internal const string Redacted = "<redacted>";

    internal static readonly string[] Sources = ["env", "workload", "file", "cli"];

    internal static readonly Setting[] Catalogue =
    [
        S("base_url", Ty.Url, "https://api.inorbit.hr"),
        S("token_url", Ty.Url, "https://auth.inorbit.hr/oauth2/token"),
        S("region", Ty.Reserved),
        Cred("key_id", Ty.Str, file: true),
        Cred("key_secret", Ty.Secret, file: false),
        Cred("key_secret_file", Ty.Path, file: true),
        S("scopes", Ty.Scopes),
        Cred("token", Ty.Secret, file: false),
        Cred("token_file", Ty.Path, file: true),
        S("credential_sources", Ty.List, Sources),
        S("cli_path", Ty.Path),
        Net("connect_timeout", Ty.Duration, "10s"),
        S("timeout", Ty.Duration, "30s"),
        S("total_timeout", Ty.Duration, "120s"),
        S("stream_idle_timeout", Ty.Duration, "45s"),
        S("max_retries", Ty.Int, 2),
        S("retry_base_delay", Ty.Duration, "500ms"),
        S("retry_max_delay", Ty.Duration, "8s"),
        S("retry_after_max", Ty.Duration, "60s"),
        S("retry_budget", Ty.Bool, true),
        S("streams", Ty.OneOf, "sse", ["sse", "socket"]),
        Net("proxy", Ty.Proxy),
        Net("no_proxy", Ty.List),
        Net("ca_bundle", Ty.Path),
        Net("system_trust", Ty.Bool, true),
        Net("client_cert", Ty.Path),
        Net("client_key", Ty.Path),
        Net("client_key_password", Ty.Secret) with { File = false },
        Net("pinned_keys", Ty.Pins),
        S("log", Ty.OneOf, "off", ["off", "error", "warn", "info", "debug"]),
        S("log_headers", Ty.Bool, false),
        S("log_allow_headers", Ty.List),
        S("tracing", Ty.Bool),
        S("metrics", Ty.Bool),
        S("rate_limit", Ty.OneOf, "observe", ["observe", "wait", "off"]),
        S("user_agent_suffix", Ty.Str),
    ];

    internal enum Ty
    {
        Duration,
        Int,
        Bool,
        Scopes,
        List,
        Url,
        Path,
        Secret,
        Str,
        Proxy,
        Pins,
        Reserved,
        OneOf,
    }

    internal enum From
    {
        Code,
        Env,
        File,
    }

    /// <summary>A parsed duration: digits, then <c>ms</c>, <c>s</c>, <c>m</c> or <c>h</c>, greater than zero.</summary>
    internal static TimeSpan? ParseDuration(string v)
    {
        var m = DurationPattern().Match(v);
        if (!m.Success || !long.TryParse(m.Groups[1].Value, NumberStyles.None, CultureInfo.InvariantCulture, out var n))
        {
            return null;
        }

        var unit = m.Groups[2].Value switch { "ms" => 1L, "s" => 1000L, "m" => 60_000L, _ => 3_600_000L };
        if (n <= 0 || n > long.MaxValue / unit / TimeSpan.TicksPerMillisecond)
        {
            return null;
        }

        return TimeSpan.FromMilliseconds(n * unit);
    }

    /// <summary>A duration as <c>describe()</c> shows it: <c>30s</c>, or <c>1500ms</c>.</summary>
    internal static string ShowDuration(TimeSpan d)
    {
        var ms = (long)Math.Ceiling(d.TotalMilliseconds);
        return ms % 1000 == 0 ? (ms / 1000).ToString(CultureInfo.InvariantCulture) + "s" : ms.ToString(CultureInfo.InvariantCulture) + "ms";
    }

    /// <summary>A profile name in the environment's form: <c>acme-ci</c> becomes <c>ACME_CI</c>.</summary>
    internal static string EnvName(string profile) => profile.ToUpperInvariant().Replace('-', '_');

    /// <summary>The OS this process runs on.</summary>
    internal static HostOs CurrentOs() => OperatingSystem.IsWindows() ? HostOs.Windows : OperatingSystem.IsMacOS() ? HostOs.MacOS : HostOs.Linux;

    /// <summary>The process environment; on Windows names compare without case, as there.</summary>
    internal static Dictionary<string, string> ProcessEnvironment()
    {
        var env = new Dictionary<string, string>(OperatingSystem.IsWindows() ? StringComparer.OrdinalIgnoreCase : StringComparer.Ordinal);
        foreach (DictionaryEntry e in System.Environment.GetEnvironmentVariables())
        {
            if (e.Key is string k && e.Value is string v)
            {
                env[k] = v;
            }
        }

        return env;
    }

    /// <summary>
    /// Where the config file is (docs/config.md section 4.1): the path, whether it was named (code or
    /// <c>INORBIT_CONFIG_FILE</c>) and the label of where that came from; <see langword="null"/> reads no file.
    /// </summary>
    internal static (string Path, bool Named, string Label)? ConfigPath(HostOs os, IReadOnlyDictionary<string, string> env, string? home, string? code)
    {
        var paths = new Paths(os);
        string? V(string k) => env.TryGetValue(k, out var x) && x.Length > 0 ? x : null;
        if (code is not null)
        {
            return code == "off" ? null : (code, true, "code");
        }

        if (V("INORBIT_CONFIG_FILE") is { } named)
        {
            return named == "off" ? null : (named, true, "env INORBIT_CONFIG_FILE");
        }

        if (V("IOHR_CONFIG_DIR") is { } dir)
        {
            return (paths.Join(dir, "config.toml"), false, "env IOHR_CONFIG_DIR");
        }

        switch (os)
        {
            case HostOs.MacOS:
                return home is null ? null : (paths.Join(home, "Library/Application Support/hr.InOrbit.iohr/config.toml"), false, "default");
            case HostOs.Windows:
                return V("APPDATA") is { } appdata ? (paths.Join(appdata, "InOrbit\\iohr\\config\\config.toml"), false, "default") : null;
            default:
                if (V("XDG_CONFIG_HOME") is { } xdg && paths.IsAbsolute(xdg))
                {
                    return (paths.Join(xdg, "iohr/config.toml"), false, "default");
                }

                return home is null ? null : (paths.Join(home, ".config/iohr/config.toml"), false, "default");
        }
    }

    /// <summary>Resolves a configuration, or throws a <see cref="ConfigException"/> listing every problem.</summary>
    internal static Resolution Resolve(ResolveInput input) => new Resolver(input).Run();

    private static Setting S(string name, Ty ty, object? fallback = null, string[]? oneOf = null) =>
        new(name, ty, File: true, Credential: false, Transport: false, fallback, oneOf ?? []);

    private static Setting Cred(string name, Ty ty, bool file) =>
        new(name, ty, file, Credential: true, Transport: false, Fallback: null, OneOf: []);

    private static Setting Net(string name, Ty ty, object? fallback = null) =>
        new(name, ty, File: true, Credential: false, Transport: true, fallback, []);

    [GeneratedRegex("^([0-9]+)(ms|s|m|h)$", RegexOptions.CultureInvariant)]
    private static partial Regex DurationPattern();

    [GeneratedRegex("^[a-z0-9][a-z0-9_-]{0,63}$", RegexOptions.CultureInvariant)]
    internal static partial Regex ProfilePattern();

    /// <summary>One setting of the catalogue (section 3).</summary>
    internal sealed record Setting(string Name, Ty Ty, bool File, bool Credential, bool Transport, object? Fallback, string[] OneOf);

    /// <summary>A value before parsing, and where it came from.</summary>
    internal readonly record struct Raw(From From, object? Value);

    /// <summary>Path rules for one OS: absolute, join, parent.</summary>
    internal sealed class Paths(HostOs os)
    {
        internal char Sep => os == HostOs.Windows ? '\\' : '/';

        internal bool IsAbsolute(string p)
        {
            if (p.StartsWith('/'))
            {
                return true;
            }

            var drive = p.Length >= 3 && char.IsAsciiLetter(p[0]) && p[1] == ':' && p[2] is '\\' or '/';
            return drive || (os == HostOs.Windows && p.StartsWith('\\'));
        }

        internal string Join(string dir, string rest) => dir.TrimEnd('/', '\\') + Sep + rest;

        internal string Parent(string path)
        {
            var i = Math.Max(path.LastIndexOf('/'), path.LastIndexOf('\\'));
            var p = i == 0 ? path[..1] : i < 0 ? "." : path[..i];
            return os == HostOs.Windows ? p.Replace('/', '\\') : p.Replace('\\', '/');
        }
    }
}

/// <summary>What <see cref="Settings.Resolve"/> is handed.</summary>
internal sealed record ResolveInput
{
    /// <summary>Options set in code, by catalogue name.</summary>
    public required IReadOnlyDictionary<string, object> Code { get; init; }

    /// <summary>Whether code supplied the HTTP client (an <c>HttpClient</c> or a handler).</summary>
    public bool HttpClient { get; init; }

    /// <summary>Whether code supplied a token provider.</summary>
    public bool TokenProvider { get; init; }

    /// <summary>Resolve for a typed (generated) profile of this name.</summary>
    public string? ProfileType { get; init; }

    /// <summary>Explicit construction: code and defaults only, no environment, file or chain.</summary>
    public bool Explicit { get; init; }

    public LoadOptions? Load { get; init; }
}

/// <summary>What the chain decided: the credential's shape and where its pieces are.</summary>
internal sealed record CredentialPlan(string Source, string Kind)
{
    public string? Token { get; init; }

    public string? Path { get; init; }

    public string? KeyId { get; init; }

    public string? KeySecret { get; init; }

    public string? KeySecretFile { get; init; }

    public string? Profile { get; init; }

    public string? Program { get; init; }
}

/// <summary>The proxy, with whether it was set on purpose (code, <c>INORBIT_PROXY</c>, the file) or came from the standard variables.</summary>
internal sealed record ProxyChoice(string Url, bool Explicit);

/// <summary>A resolved configuration: the values a client is built from, and its description.</summary>
internal sealed record Resolution(
    IReadOnlyDictionary<string, object> Values,
    JsonObject Description,
    CredentialPlan Credential,
    string? Profile,
    ProxyChoice? Proxy)
{
    internal T? Get<T>(string name) => Values.TryGetValue(name, out var v) && v is T t ? t : default;

    internal TimeSpan Duration(string name) => Values.TryGetValue(name, out var v) && v is TimeSpan t ? t : TimeSpan.Zero;

    /// <summary>The <c>describe()</c> document with the final pipeline.</summary>
    internal ResolvedConfig Describe(IEnumerable<string> pipeline)
    {
        var doc = (JsonObject)Description.DeepClone();
        doc["pipeline"] = new JsonArray(pipeline.Select(n => (JsonNode?)JsonValue.Create(n)).ToArray());
        doc.Remove("ignored", out var ignored);
        doc["ignored"] = ignored;
        return new ResolvedConfig(doc);
    }
}

/// <summary>One run of resolution.</summary>
internal sealed class Resolver
{
    private readonly ResolveInput _in;
    private readonly IReadOnlyDictionary<string, string> _env;
    private readonly HostOs _os;
    private readonly Settings.Paths _paths;
    private readonly string? _home;
    private readonly string _cwd;
    private readonly string? _prefix;
    private readonly List<ConfigProblem> _problems = [];
    private readonly Dictionary<string, (JsonNode? Value, string Source)> _settings = new(StringComparer.Ordinal);
    private readonly List<string> _settingOrder = [];
    private readonly Dictionary<string, object> _values = new(StringComparer.Ordinal);
    private readonly List<(string Key, string Source, string Reason)> _ignored = [];
    private readonly List<(TomlTable Table, string Label, bool Profile)> _layers = [];
    private string? _filePath;
    private string _fileDir = ".";
    private ProxyChoice? _proxy;

    internal Resolver(ResolveInput input)
    {
        _in = input;
        var load = input.Load ?? new LoadOptions();
        _env = input.Explicit ? new Dictionary<string, string>() : load.Environment ?? Settings.ProcessEnvironment();
        _os = load.Os ?? Settings.CurrentOs();
        _paths = new Settings.Paths(_os);
        _home = load.NoHome ? null : load.Home ?? MachineHome();
        _cwd = load.WorkingDirectory ?? System.Environment.CurrentDirectory;
        _prefix = input.ProfileType is null ? null : $"INORBIT_{Settings.EnvName(input.ProfileType)}_";
    }

    internal Resolution Run()
    {
        var doc = _in.Explicit ? [] : ReadFile();
        (string Name, string Source)? profile = null;
        if (_in.ProfileType is not null)
        {
            profile = (_in.ProfileType, "code");
        }
        else if (!_in.Explicit)
        {
            profile = ChooseProfile(doc);
        }

        var profiles = doc.TryGetValue("profiles", out var ps) && ps is TomlTable pt ? pt : null;
        TomlTable? table = null;
        if (profile is { } p && profiles is not null && profiles.TryGetValue(p.Name, out var t) && t is TomlTable tt)
        {
            table = tt;
            _layers.Add((tt, Label($"profiles.{p.Name}"), true));
        }

        if (doc.TryGetValue("sdk", out var sdk) && sdk is TomlTable st)
        {
            _layers.Add((st, Label("sdk"), false));
        }

        _fileDir = _filePath is null ? _cwd : _paths.Parent(_filePath);
        CheckFileKeys();
        ResolveSettings();
        var credential = _in.Explicit ? ExplicitCredential() : Chain(profile?.Name, table);
        CrossChecks();
        if (_problems.Count > 0)
        {
            throw ConfigException.Of([.. _problems.OrderBy(x => Order(x.Setting))]);
        }

        if (credential is null)
        {
            throw new ConfigException("no credentials");
        }

        var settings = new JsonObject();
        foreach (var name in _settingOrder)
        {
            if (_settings.TryGetValue(name, out var s))
            {
                settings[name] = new JsonObject { ["value"] = s.Value?.DeepClone(), ["source"] = s.Source };
            }
        }

        var description = new JsonObject
        {
            ["profile"] = profile is { } chosen ? new JsonObject { ["name"] = chosen.Name, ["source"] = chosen.Source } : null,
            ["config_file"] = _filePath,
            ["settings"] = settings,
            ["credential"] = credential.Value.Described,
            ["ignored"] = new JsonArray(_ignored.Select(i => (JsonNode?)new JsonObject { ["key"] = i.Key, ["source"] = i.Source, ["reason"] = i.Reason }).ToArray()),
        };
        return new Resolution(_values, description, credential.Value.Plan, profile?.Name, _proxy);
    }

    private static string? MachineHome()
    {
        var h = System.Environment.GetFolderPath(System.Environment.SpecialFolder.UserProfile);
        return string.IsNullOrEmpty(h) ? null : h;
    }

    private static int Order(string setting)
    {
        if (setting == "profile")
        {
            return 0;
        }

        if (setting == "config_file")
        {
            return 1;
        }

        if (setting == "credential")
        {
            return int.MaxValue;
        }

        var i = System.Array.FindIndex(Settings.Catalogue, x => x.Name == setting);
        return i < 0 ? int.MaxValue - 1 : i + 2;
    }

    private static string TomlType(object? v) => v switch
    {
        string => "string",
        long or int => "integer",
        double => "float",
        bool => "boolean",
        TomlDateTime => "datetime",
        TomlArray => "array",
        _ => "table",
    };

    private static bool Readable(string path)
    {
        try
        {
            using var f = File.OpenRead(path);
            return true;
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException or ArgumentException or NotSupportedException)
        {
            return false;
        }
    }

    private static string RedactUserinfo(string raw)
    {
        var i = raw.IndexOf("://", StringComparison.Ordinal);
        if (i < 0)
        {
            return raw;
        }

        var rest = raw[(i + 3)..];
        var end = rest.IndexOfAny(['/', '?', '#']);
        var authority = end < 0 ? rest : rest[..end];
        var at = authority.LastIndexOf('@');
        return at < 0 ? raw : $"{raw[..i]}://{Settings.Redacted}@{rest[(at + 1)..]}";
    }

    private static bool HasUserinfo(string raw) => Uri.TryCreate(raw, UriKind.Absolute, out var u) && u.UserInfo.Length > 0;

    private static bool IsLoopbackHost(string host)
    {
        var h = host.Trim('[', ']').ToLowerInvariant();
        return h == "localhost" || (System.Net.IPAddress.TryParse(h, out var ip) && System.Net.IPAddress.IsLoopback(ip));
    }

    private string? Var(string k) => _env.TryGetValue(k, out var v) && v.Length > 0 ? v : null;

    private void Problem(string setting, string source, string message) => _problems.Add(new(setting, source, message));

    private string Label(string suffix)
    {
        if (_filePath is null)
        {
            return string.Empty;
        }

        return suffix.Length == 0 ? $"file {_filePath}" : $"file {_filePath} [{suffix}]";
    }

    private TomlTable ReadFile()
    {
        var code = _in.Code.TryGetValue("config_file", out var c) ? c as string : null;
        var located = Settings.ConfigPath(_os, _env, _home, code);
        if (located is not { } l)
        {
            return [];
        }

        var abs = _paths.IsAbsolute(l.Path) ? l.Path : _paths.Join(_cwd, l.Path);
        FileInfo info;
        try
        {
            info = new FileInfo(abs);
        }
        catch (Exception e) when (e is ArgumentException or NotSupportedException or PathTooLongException or UnauthorizedAccessException)
        {
            if (l.Named)
            {
                Problem("config_file", l.Label, $"there is no readable file at {abs}");
            }

            return [];
        }

        if (!info.Exists)
        {
            if (l.Named)
            {
                Problem("config_file", l.Label, $"there is no readable file at {abs}");
            }

            return [];
        }

        if (info.Length > 1024 * 1024)
        {
            Problem("config_file", l.Label, $"{abs} is larger than 1 MiB");
            return [];
        }

        string text;
        try
        {
            text = new UTF8Encoding(false, true).GetString(File.ReadAllBytes(abs));
        }
        catch (DecoderFallbackException)
        {
            Problem("config_file", l.Label, $"{abs} is not valid TOML: it is not UTF-8");
            return [];
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException)
        {
            Problem("config_file", l.Label, $"there is no readable file at {abs}");
            return [];
        }

        try
        {
            var parsed = TomlSerializer.Deserialize<TomlTable>(text) ?? [];
            _filePath = abs;
            return parsed;
        }
        catch (TomlException e)
        {
            // The first line only: the parser's excerpt could quote a secret.
            var first = e.Message.Split('\n')[0];
            Problem("config_file", l.Label, $"{abs} is not valid TOML: {first}");
            return [];
        }
    }

    private (string Name, string Source)? ChooseProfile(TomlTable doc)
    {
        (string Name, string Source)? chosen = null;
        if (_in.Code.TryGetValue("profile", out var c) && c is string code && code.Length > 0)
        {
            chosen = (code, "code");
        }
        else if (Var("INORBIT_PROFILE") is { } env)
        {
            chosen = (env, "env INORBIT_PROFILE");
        }
        else if (doc.TryGetValue("default", out var d) && d is string def)
        {
            chosen = (def, Label(string.Empty));
        }

        if (chosen is not { } p)
        {
            return null;
        }

        if (!Settings.ProfilePattern().IsMatch(p.Name))
        {
            Problem("profile", p.Source, $"{Quote(p.Name)} is not a profile name: 1 to 64 lower-case letters, digits, '-' or '_', starting with a letter or digit");
            return null;
        }

        var profiles = doc.TryGetValue("profiles", out var ps) && ps is TomlTable pt ? pt : null;
        if (profiles is null || !profiles.ContainsKey(p.Name))
        {
            var where = _filePath is null ? "no config file was read" : $"{_filePath} has no [profiles.{p.Name}]";
            Problem("profile", p.Source, $"there is no profile {Quote(p.Name)}: {where}; `iohr profile list` shows the profiles");
            return null;
        }

        return p;
    }

    private static string Quote(string s) => JsonSerializer.Serialize(s);

    private void CheckFileKeys()
    {
        foreach (var (table, label, isProfile) in _layers)
        {
            foreach (var (k, v) in table)
            {
                if (isProfile && System.Array.IndexOf(CliKeys, k) >= 0)
                {
                    continue;
                }

                var setting = System.Array.Find(Settings.Catalogue, x => x.Name == k);
                if (setting is null)
                {
                    _ignored.Add((k, label, k is "profile" or "config_file" ? "not read from the config file" : "unknown key"));
                }
                else if (!setting.File)
                {
                    var wayOut = k switch
                    {
                        "key_secret" => "key_secret_file, the environment, or iohr login",
                        "token" => "token_file, the environment, or iohr login",
                        _ => "the environment or code",
                    };
                    Problem(k, label, $"secrets are not allowed in the config file; use {wayOut}");
                }
                else if (setting.Ty == Settings.Ty.Proxy && v is string s && HasUserinfo(s))
                {
                    Problem(k, label, "a proxy URL with a user name or password holds a secret, which is not allowed in the config file; set it in INORBIT_PROXY or in code");
                }
            }
        }
    }

    private static readonly string[] CliKeys = ["kind", "account", "storage", "issuer", "client_id"];

    private List<string> EnvNames(Settings.Setting setting)
    {
        var upper = setting.Name.ToUpperInvariant();
        var names = new List<string>();
        if (_prefix is not null)
        {
            names.Add(_prefix + upper);
        }

        if (!(setting.Credential && _prefix is not null))
        {
            names.Add("INORBIT_" + upper);
        }

        return names;
    }

    private (Settings.Raw Raw, string Source)? Find(Settings.Setting setting)
    {
        if (_in.Code.TryGetValue(setting.Name, out var code))
        {
            return (new(Settings.From.Code, code), "code");
        }

        foreach (var n in EnvNames(setting))
        {
            if (Var(n) is { } v)
            {
                return (new(Settings.From.Env, v), $"env {n}");
            }
        }

        if (setting.File)
        {
            foreach (var (table, label, _) in _layers)
            {
                if (table.TryGetValue(setting.Name, out var v))
                {
                    return (new(Settings.From.File, v), label);
                }
            }
        }

        string[] after = setting.Name switch
        {
            "proxy" => ["https_proxy", "HTTPS_PROXY"],
            "no_proxy" => ["no_proxy", "NO_PROXY"],
            _ => [],
        };
        foreach (var n in after)
        {
            if (Var(n) is { } v)
            {
                return (new(Settings.From.Env, v), $"env {n}");
            }
        }

        return null;
    }

    private void Set(string name, JsonNode? shown, object value, string source)
    {
        Show(name, shown, source);
        _values[name] = value;
    }

    private void Show(string name, JsonNode? shown, string source)
    {
        if (!_settings.ContainsKey(name))
        {
            _settingOrder.Add(name);
        }

        _settings[name] = (shown, source);
    }

    private void Unshow(string name)
    {
        _settings.Remove(name);
        _settingOrder.Remove(name);
    }

    private void ResolveSettings()
    {
        foreach (var setting in Settings.Catalogue)
        {
            if (setting.Credential)
            {
                continue;
            }

            var found = Find(setting);
            if (found is not { } f)
            {
                if (setting.Fallback is not null && !(setting.Transport && _in.HttpClient))
                {
                    var (shown, value) = Fallback(setting);
                    Set(setting.Name, shown, value, "default");
                }

                continue;
            }

            if (setting.Transport && _in.HttpClient)
            {
                if (f.Source == "code")
                {
                    Problem(setting.Name, "code", "configure this on your HTTP client (HttpClient or HttpMessageHandler), or leave http_client out");
                }
                else
                {
                    _ignored.Add((setting.Name, f.Source, "the caller's HTTP client decides this"));
                }

                continue;
            }

            try
            {
                var (shown, value) = Parse(setting, f.Raw);
                Set(setting.Name, shown, value, f.Source);
                if (setting.Name == "proxy")
                {
                    _proxy = new ProxyChoice((string)value, !f.Source.StartsWith("env ", StringComparison.Ordinal) || f.Source == "env INORBIT_PROXY");
                }
            }
            catch (FormatException e)
            {
                Problem(setting.Name, f.Source, e.Message);
            }
        }
    }

    private static (JsonNode? Shown, object Value) Fallback(Settings.Setting setting) => setting.Fallback switch
    {
        string s when setting.Ty == Settings.Ty.Duration => (s, Settings.ParseDuration(s)!.Value),
        string s => (s, s),
        int i => (i, i),
        bool b => (b, b),
        string[] l => (new JsonArray(l.Select(x => (JsonNode?)x).ToArray()), l),
        _ => throw new InvalidOperationException(setting.Name),
    };

    private string PathOf(string p, bool fromFile)
    {
        if (p.StartsWith("~/", StringComparison.Ordinal))
        {
            if (_home is null)
            {
                throw new FormatException($"{p} starts with ~/ but there is no home directory");
            }

            return _paths.Join(_home, p[2..]);
        }

        return _paths.IsAbsolute(p) ? p : _paths.Join(fromFile ? _fileDir : _cwd, p);
    }

    private static JsonArray JArray(IEnumerable<string> l) => new(l.Select(x => (JsonNode?)x).ToArray());

    /// <summary>The value as <c>describe()</c> shows it, and as the client uses it; a <see cref="FormatException"/> says what is wrong.</summary>
    private (JsonNode? Shown, object Value) Parse(Settings.Setting setting, Settings.Raw raw)
    {
        var fromFile = raw.From == Settings.From.File;
        string Text(string what) => raw.Value switch
        {
            string s => s,
            Uri u => u.OriginalString,
            _ => throw new FormatException(fromFile ? $"must be {what}, not {TomlType(raw.Value)}" : $"must be {what}"),
        };
        string[] List(bool comma)
        {
            if (raw.Value is string s)
            {
                return comma
                    ? s.Split(',').Select(x => x.Trim()).Where(x => x.Length > 0).ToArray()
                    : s.Split((char[]?)null, StringSplitOptions.RemoveEmptyEntries);
            }

            if (raw.Value is IEnumerable<string> strings)
            {
                return strings.ToArray();
            }

            if (raw.Value is TomlArray a)
            {
                if (a.All(x => x is string))
                {
                    return a.Cast<string>().ToArray();
                }

                throw new FormatException("must be an array of strings");
            }

            throw new FormatException(fromFile ? $"must be an array of strings, not {TomlType(raw.Value)}" : "must be a list of strings");
        }

        switch (setting.Ty)
        {
            case Settings.Ty.OneOf:
                {
                    var v = Text("a string");
                    if (System.Array.IndexOf(setting.OneOf, v) < 0)
                    {
                        throw new FormatException($"{Quote(v)} is not one of {string.Join(", ", setting.OneOf)}");
                    }

                    return (v, v);
                }

            case Settings.Ty.Duration:
                {
                    if (raw.From == Settings.From.Code)
                    {
                        if (raw.Value is not TimeSpan d || d <= TimeSpan.Zero || d == System.Threading.Timeout.InfiniteTimeSpan)
                        {
                            throw new FormatException("must be a duration greater than zero");
                        }

                        var ms = TimeSpan.FromMilliseconds(Math.Ceiling(d.TotalMilliseconds));
                        return (Settings.ShowDuration(ms), ms);
                    }

                    var v = Text("a duration string such as \"30s\"");
                    if (Settings.ParseDuration(v) is not { } parsed)
                    {
                        throw new FormatException(Settings.ParseDuration(v + "s") is not null
                            ? $"{Quote(v)} is not a duration; write it with a unit, such as 30s"
                            : $"{Quote(v)} is not a duration greater than zero: digits, then ms, s, m or h, such as 30s");
                    }

                    return (Settings.ShowDuration(parsed), parsed);
                }

            case Settings.Ty.Int:
                {
                    if (raw.Value is string s && raw.From == Settings.From.Env)
                    {
                        if (!int.TryParse(s, NumberStyles.None, CultureInfo.InvariantCulture, out var n))
                        {
                            throw new FormatException($"{Quote(s)} is not a whole number of 0 or more");
                        }

                        return (n, n);
                    }

                    switch (raw.Value)
                    {
                        case int i when i >= 0:
                            return (i, i);
                        case long l when l is >= 0 and <= int.MaxValue:
                            return ((int)l, (int)l);
                        case not (int or long) when fromFile:
                            throw new FormatException($"must be an integer, not {TomlType(raw.Value)}");
                        default:
                            throw new FormatException("must be a whole number of 0 or more");
                    }
                }

            case Settings.Ty.Bool:
                {
                    if (raw.From == Settings.From.Env && raw.Value is string s)
                    {
                        switch (s.ToLowerInvariant())
                        {
                            case "true" or "1":
                                return (true, true);
                            case "false" or "0":
                                return (false, false);
                            default:
                                throw new FormatException($"{Quote(s)} is not true, false, 1 or 0");
                        }
                    }

                    if (raw.Value is bool b)
                    {
                        return (b, b);
                    }

                    throw new FormatException(fromFile ? $"must be a boolean, not {TomlType(raw.Value)}" : "must be a boolean");
                }

            case Settings.Ty.Scopes:
                {
                    var l = List(comma: false);
                    return (JArray(l), l);
                }

            case Settings.Ty.List:
                {
                    var l = List(comma: true);
                    if (setting.Name == "credential_sources" && l.FirstOrDefault(x => System.Array.IndexOf(Settings.Sources, x) < 0) is { } bad)
                    {
                        throw new FormatException($"{Quote(bad)} is not a credential source; use env, workload, file or cli");
                    }

                    if (setting.Name == "no_proxy" && l.FirstOrDefault(x => NoProxy.ParseEntry(x) is null) is { } wrong)
                    {
                        throw new FormatException($"{Quote(wrong)} is not a no_proxy entry: a host, .domain, host:port, an IP address or a CIDR range");
                    }

                    if (setting.Name == "log_allow_headers")
                    {
                        l = l.Select(x => x.ToLowerInvariant()).ToArray();
                    }

                    return (JArray(l), l);
                }

            case Settings.Ty.Url:
                {
                    var v = Text("a URL string");
                    if (!Uri.TryCreate(v, UriKind.Absolute, out var u) || (u.Scheme != Uri.UriSchemeHttp && u.Scheme != Uri.UriSchemeHttps && !v.Contains("://", StringComparison.Ordinal)))
                    {
                        throw new FormatException($"{Quote(v)} is not an absolute URL");
                    }

                    if (!(u.Scheme == Uri.UriSchemeHttps || (u.Scheme == Uri.UriSchemeHttp && IsLoopbackHost(u.Host))))
                    {
                        throw new FormatException($"{Quote(v)} must use https (plain http is allowed only for localhost and loopback addresses)");
                    }

                    if (u.UserInfo.Length > 0 || u.Fragment.Length > 0)
                    {
                        throw new FormatException($"{Quote(v)} must not carry credentials or a fragment");
                    }

                    if (setting.Name == "base_url" && (u.AbsolutePath != "/" || u.Query.Length > 0))
                    {
                        throw new FormatException($"{Quote(v)} is an origin only, such as https://api.inorbit.hr");
                    }

                    return (v, v);
                }

            case Settings.Ty.Path:
                {
                    var p = PathOf(Text("a path string"), fromFile);
                    return (p, p);
                }

            case Settings.Ty.Secret:
                return raw.Value is string secret ? (Settings.Redacted, secret) : throw new FormatException("must be a string");

            case Settings.Ty.Str:
                {
                    var v = Text("a string");
                    if (setting.Name == "user_agent_suffix" && (v.Length > 128 || v.Any(ch => ch is < ' ' or > '~')))
                    {
                        throw new FormatException("must be product tokens (such as myapp/1.2), at most 128 printable ASCII characters");
                    }

                    return (v, v);
                }

            case Settings.Ty.Proxy:
                {
                    var v = Text("a URL string");
                    if (v == "off")
                    {
                        return ("off", "off");
                    }

                    var shown = RedactUserinfo(v);
                    if (!Uri.TryCreate(v, UriKind.Absolute, out var u))
                    {
                        throw new FormatException($"{Quote(shown)} is not an absolute URL");
                    }

                    if (u.Scheme != Uri.UriSchemeHttp && u.Scheme != Uri.UriSchemeHttps)
                    {
                        throw new FormatException($"{Quote(shown)} must be an http:// or https:// proxy URL, or off");
                    }

                    return (shown, v);
                }

            case Settings.Ty.Pins:
                {
                    var l = List(comma: true);
                    if (l.Length < 2)
                    {
                        throw new FormatException("pin at least two keys (the current one and a backup)");
                    }

                    foreach (var p in l)
                    {
                        var buffer = new byte[48];
                        if (!Convert.TryFromBase64String(p, buffer, out var n) || n != 32)
                        {
                            throw new FormatException($"{Quote(p)} is not a base64 SHA-256 of a public key");
                        }
                    }

                    return (JArray(l), l);
                }

            default:
                throw new FormatException("region is reserved until the API offers regions; remove it");
        }
    }

    private bool Allowed(string source) => !_values.TryGetValue("credential_sources", out var l) || l is not string[] list || System.Array.IndexOf(list, source) >= 0;

    private string? CodeString(string k) => _in.Code.TryGetValue(k, out var v) && v is string s && s.Length > 0 ? s : null;

    private (CredentialPlan Plan, JsonObject Described)? ExplicitCredential()
    {
        CredentialPlan plan;
        if (_in.TokenProvider)
        {
            plan = new("code", "custom");
        }
        else if (CodeString("token") is { } token)
        {
            plan = new("code", "static_token") { Token = token };
            Show("token", Settings.Redacted, "code");
        }
        else if (CodeString("token_file") is { } tokenFile)
        {
            plan = new("code", "token_file") { Path = tokenFile };
            Show("token_file", tokenFile, "code");
        }
        else
        {
            var keyId = CodeString("key_id") ?? string.Empty;
            Show("key_id", keyId, "code");
            if (CodeString("key_secret") is { } secret)
            {
                plan = new("code", "client_credentials") { KeyId = keyId, KeySecret = secret };
                Show("key_secret", Settings.Redacted, "code");
            }
            else
            {
                var file = CodeString("key_secret_file") ?? string.Empty;
                plan = new("code", "client_credentials") { KeyId = keyId, KeySecretFile = file };
                Show("key_secret_file", file, "code");
            }
        }

        ScopesFor(plan.Kind);
        return (plan, Described(plan, [new JsonObject { ["source"] = "code", ["result"] = "used" }]));
    }

    private static JsonObject Described(CredentialPlan plan, List<JsonNode?> tried) =>
        new() { ["source"] = plan.Source, ["kind"] = plan.Kind, ["tried"] = new JsonArray([.. tried]) };

    /// <summary>The chain (section 5.1); <see langword="null"/> when a problem stopped it.</summary>
    private (CredentialPlan Plan, JsonObject Described)? Chain(string? profile, TomlTable? table)
    {
        var tried = new List<JsonNode?>();
        void Skip(string source, string reason) => tried.Add(new JsonObject { ["source"] = source, ["result"] = "skipped", ["reason"] = reason });
        void Used(string source) => tried.Add(new JsonObject { ["source"] = source, ["result"] = "used" });
        var p = _prefix ?? "INORBIT_";
        CredentialPlan? plan = null;

        // 1. Code.
        if (_in.TokenProvider)
        {
            plan = new("code", "custom");
        }
        else if (CodeString("token") is { } token)
        {
            plan = new("code", "static_token") { Token = token };
            Show("token", Settings.Redacted, "code");
        }
        else if (CodeString("token_file") is { } tokenFile)
        {
            var path = SafePath(tokenFile, false);
            if (!Readable(path))
            {
                Problem("token_file", "code", $"cannot read {path}");
                return null;
            }

            plan = new("code", "token_file") { Path = path };
            Show("token_file", path, "code");
        }
        else if (CodeString("key_id") is { } keyId)
        {
            var secret = CodeString("key_secret");
            var secretFile = CodeString("key_secret_file");
            if (secret is not null && secretFile is not null)
            {
                Problem("key_secret", "code", "KeySecret and KeySecretFile are both set; set one");
                return null;
            }

            if (secret is null && secretFile is null)
            {
                Problem("key_secret", "code", "KeyId is set without KeySecret or KeySecretFile");
                return null;
            }

            if (!_values.ContainsKey("scopes"))
            {
                Problem("scopes", "code", "a key needs scopes: set Scopes");
                return null;
            }

            Show("key_id", keyId, "code");
            if (secret is not null)
            {
                plan = new("code", "client_credentials") { KeyId = keyId, KeySecret = secret };
                Show("key_secret", Settings.Redacted, "code");
            }
            else
            {
                var path = SafePath(secretFile!, false);
                if (!Readable(path))
                {
                    Problem("key_secret_file", "code", $"cannot read {path}");
                    return null;
                }

                plan = new("code", "client_credentials") { KeyId = keyId, KeySecretFile = path };
                Show("key_secret_file", path, "code");
            }
        }

        if (plan is not null)
        {
            Used("code");
        }
        else
        {
            Skip("code", "none set");
        }

        // 2. The environment.
        if (plan is null)
        {
            string N(string x) => p + x;
            string Src(string x) => "env " + p + x;
            var token = Var(N("TOKEN"));
            var tokenFile = Var(N("TOKEN_FILE"));
            var keyId = Var(N("KEY_ID"));
            var secret = Var(N("KEY_SECRET"));
            var secretFile = Var(N("KEY_SECRET_FILE"));
            if (!Allowed("env"))
            {
                Skip("env", "not in credential_sources");
            }
            else if (token is null && tokenFile is null && keyId is null)
            {
                Skip("env", $"{N("TOKEN")}, {N("TOKEN_FILE")} and {N("KEY_ID")} are not set");
            }
            else
            {
                var set = new[] { ("TOKEN", token), ("TOKEN_FILE", tokenFile), ("KEY_ID", keyId) }.Where(x => x.Item2 is not null).Select(x => x.Item1).ToList();
                if (set.Count > 1)
                {
                    Problem(set[0].ToLowerInvariant(), Src(set[0]), $"{string.Join(" and ", set.Select(N))} are both set; set one credential");
                    return null;
                }

                if (token is not null)
                {
                    Show("token", Settings.Redacted, Src("TOKEN"));
                    plan = new("env", "static_token") { Token = token };
                }
                else if (tokenFile is not null)
                {
                    var path = SafePath(tokenFile, false);
                    if (!Readable(path))
                    {
                        Problem("token_file", Src("TOKEN_FILE"), $"cannot read {path}");
                        return null;
                    }

                    Show("token_file", path, Src("TOKEN_FILE"));
                    plan = new("env", "token_file") { Path = path };
                }
                else if (keyId is not null)
                {
                    if (secret is not null && secretFile is not null)
                    {
                        Problem("key_secret", Src("KEY_SECRET"), $"{N("KEY_SECRET")} and {N("KEY_SECRET_FILE")} are both set; set one");
                        return null;
                    }

                    if (secret is null && secretFile is null)
                    {
                        Problem("key_secret", Src("KEY_ID"), $"{N("KEY_ID")} is set without {N("KEY_SECRET")} or {N("KEY_SECRET_FILE")}");
                        return null;
                    }

                    if (!_values.ContainsKey("scopes"))
                    {
                        Problem("scopes", Src("KEY_ID"), $"a key needs scopes: set {p}SCOPES");
                        return null;
                    }

                    Show("key_id", keyId, Src("KEY_ID"));
                    if (secret is not null)
                    {
                        Show("key_secret", Settings.Redacted, Src("KEY_SECRET"));
                        plan = new("env", "client_credentials") { KeyId = keyId, KeySecret = secret };
                    }
                    else
                    {
                        var path = SafePath(secretFile!, false);
                        if (!Readable(path))
                        {
                            Problem("key_secret_file", Src("KEY_SECRET_FILE"), $"cannot read {path}");
                            return null;
                        }

                        Show("key_secret_file", path, Src("KEY_SECRET_FILE"));
                        plan = new("env", "client_credentials") { KeyId = keyId, KeySecretFile = path };
                    }
                }

                if (plan is not null)
                {
                    Used("env");
                }
            }
        }

        // 3. Workload identity: reserved until the platform exchanges outside tokens.
        if (plan is null)
        {
            Skip("workload", Allowed("workload") ? "not offered by the platform yet" : "not in credential_sources");
        }

        // 4. The config file's profile table.
        if (plan is null)
        {
            var label = _layers.Count > 0 ? _layers[0].Label : string.Empty;
            if (!Allowed("file"))
            {
                Skip("file", "not in credential_sources");
            }
            else if (_filePath is null)
            {
                Skip("file", "no config file was read");
            }
            else if (profile is null)
            {
                Skip("file", "no profile chosen");
            }
            else if (table is not null && (table.ContainsKey("token_file") || table.ContainsKey("key_id")))
            {
                string? Str(string k) => table.TryGetValue(k, out var v) && v is string s ? s : null;
                if (table.ContainsKey("token_file") && table.ContainsKey("key_id"))
                {
                    Problem("token_file", label, "token_file and key_id are both set; set one credential");
                    return null;
                }

                if (Str("token_file") is { } tf)
                {
                    var path = SafePath(tf, true);
                    if (!Readable(path))
                    {
                        Problem("token_file", label, $"cannot read {path}");
                        return null;
                    }

                    Show("token_file", path, label);
                    plan = new("file", "token_file") { Path = path };
                }
                else if (Str("key_id") is { } fileKey)
                {
                    if (table.ContainsKey("key_secret"))
                    {
                        return null; // Already reported: secrets are not allowed in the file.
                    }

                    if (Str("key_secret_file") is not { } f)
                    {
                        Problem("key_secret", label, "key_id is set without key_secret_file");
                        return null;
                    }

                    if (!_values.ContainsKey("scopes"))
                    {
                        Problem("scopes", label, "a key needs scopes: set scopes in the profile's table");
                        return null;
                    }

                    var path = SafePath(f, true);
                    if (!Readable(path))
                    {
                        Problem("key_secret_file", label, $"cannot read {path}");
                        return null;
                    }

                    Show("key_id", fileKey, label);
                    Show("key_secret_file", path, label);
                    plan = new("file", "client_credentials") { KeyId = fileKey, KeySecretFile = path };
                }
                else
                {
                    Problem(table.ContainsKey("token_file") ? "token_file" : "key_id", label, "must be a string");
                    return null;
                }

                Used("file");
            }
            else
            {
                Skip("file", $"profile {profile} sets no token_file or key_id");
            }
        }

        // 5. The iohr login.
        if (plan is null)
        {
            var program = _values.TryGetValue("cli_path", out var cp) ? (string)cp : "iohr";
            if (!Allowed("cli"))
            {
                Skip("cli", "not in credential_sources");
            }
            else if (profile is null)
            {
                Skip("cli", "skipped, no profile chosen");
            }
            else if (table is null || !table.ContainsKey("kind"))
            {
                Skip("cli", $"profile {profile} was not made by iohr login");
            }
            else if (Program.Find(program, _env) is not { } found)
            {
                Skip("cli", program == "iohr" ? "iohr not found on PATH" : $"iohr not found at {program}");
            }
            else
            {
                Used("cli");
                plan = new("cli", "cli") { Profile = profile, Program = found };
            }
        }

        if (plan is null)
        {
            var lines = string.Concat(tried.Select(t => $"\n  {t!["source"]}: {t["reason"]}"));
            Problem("credential", string.Empty, $"no credentials found for profile {Quote(profile ?? "default")}; tried:{lines}\nSet {p}KEY_ID, {p}KEY_SECRET and {p}SCOPES, or {p}TOKEN, or run `iohr login`.");
            return null;
        }

        ScopesFor(plan.Kind);
        return (plan, Described(plan, tried));
    }

    private string SafePath(string p, bool fromFile)
    {
        try
        {
            return PathOf(p, fromFile);
        }
        catch (FormatException)
        {
            return p;
        }
    }

    /// <summary><c>scopes</c> next to a credential that carries its own is listed as ignored.</summary>
    private void ScopesFor(string kind)
    {
        if (kind is "client_credentials" or "custom")
        {
            return;
        }

        if (_settings.TryGetValue("scopes", out var scopes))
        {
            Unshow("scopes");
            _values.Remove("scopes");
            _ignored.Add(("scopes", scopes.Source, "not used by this credential"));
        }
    }

    private void CrossChecks()
    {
        (JsonNode? Value, string Source)? Get(string k) => _settings.TryGetValue(k, out var v) ? v : null;
        if (Get("system_trust") is { } trust && trust.Value?.GetValue<bool>() == false && Get("ca_bundle") is null)
        {
            Problem("system_trust", trust.Source, "system_trust = false needs a ca_bundle to trust instead");
        }

        var cert = Get("client_cert");
        var key = Get("client_key");
        if (cert is { } c && key is null)
        {
            Problem("client_key", c.Source, "client_cert needs client_key");
        }
        else if (cert is null && key is { } k)
        {
            Problem("client_cert", k.Source, "client_key needs client_cert");
        }

        foreach (var name in new[] { "ca_bundle", "client_cert", "client_key" })
        {
            if (Get(name) is { } v && v.Value?.GetValue<string>() is { } path && !Readable(path))
            {
                Problem(name, v.Source, $"cannot read {path}");
            }
        }
    }
}

/// <summary>Finds the program the <c>cli</c> source runs.</summary>
internal static class Program
{
    /// <summary>
    /// The full path of <paramref name="program"/>: a path to a file, or a name found on <c>PATH</c>
    /// (read from <paramref name="env"/> with this machine's separator and, on Windows, its extensions);
    /// <see langword="null"/> when it is not there.
    /// </summary>
    internal static string? Find(string program, IReadOnlyDictionary<string, string> env)
    {
        if (program.Contains('/', StringComparison.Ordinal) || program.Contains('\\', StringComparison.Ordinal))
        {
            return File.Exists(program) ? program : null;
        }

        var path = env.TryGetValue("PATH", out var p) ? p : env.TryGetValue("Path", out var w) ? w : null;
        if (string.IsNullOrEmpty(path))
        {
            return null;
        }

        string[] exts = OperatingSystem.IsWindows() ? ["", ".exe", ".cmd", ".bat"] : [""];
        foreach (var dir in path.Split(Path.PathSeparator, StringSplitOptions.RemoveEmptyEntries))
        {
            foreach (var ext in exts)
            {
                var candidate = Path.Combine(dir, program + ext);
                if (File.Exists(candidate))
                {
                    return candidate;
                }
            }
        }

        return null;
    }
}
