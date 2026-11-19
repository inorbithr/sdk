using System;
using System.Collections.Generic;
using System.Text.Json;
using System.Text.Json.Serialization;
using InOrbit.Sdk.Api;
using Xunit;

namespace InOrbit.Sdk.Tests;

public class RuntimeTests
{
    [Fact]
    public void Credentials_never_print_their_secret()
    {
        Assert.DoesNotContain("s3cr3t", new StaticToken("s3cr3t").ToString(), StringComparison.Ordinal);
        using var key = new ClientCredentials("ak_test", "s3cr3t", ["identity:read"]);
        Assert.DoesNotContain("s3cr3t", key.ToString(), StringComparison.Ordinal);
        Assert.Contains("ak_test", key.ToString(), StringComparison.Ordinal);
        Assert.DoesNotContain("s3cr3t", new Token("s3cr3t").ToString(), StringComparison.Ordinal);
        var options = new ClientOptions { Token = "s3cr3t" };
        Assert.DoesNotContain("s3cr3t", options.ToString(), StringComparison.Ordinal);
        Assert.DoesNotContain("s3cr3t", new ClientOptions { KeyId = "ak_test", KeySecret = "s3cr3t" }.ToString(), StringComparison.Ordinal);
    }

    [Fact]
    public void Credentials_never_serialise_their_secret()
    {
        var shown = new List<string>
        {
            JsonSerializer.Serialize(new Token("tok-s3cr3t", DateTimeOffset.UnixEpoch)),
            JsonSerializer.Serialize(new ClientCredentialsOptions { KeyId = "ak_test", KeySecret = "s3cr3t", Scopes = ["identity:read"] }),
            JsonSerializer.Serialize(new ClientOptions
            {
                Token = "tok-s3cr3t", KeyId = "ak_test", KeySecret = "s3cr3t", ClientKeyPassword = "pw-s3cr3t",
                Proxy = "http://ana:pw-s3cr3t@proxy.example:3128",
            }),
            JsonSerializer.Serialize(new Dictionary<string, object> { ["token"] = new Token("tok-s3cr3t") }),
        };
        foreach (var json in shown)
        {
            Assert.DoesNotContain("s3cr3t", json, StringComparison.Ordinal);
            Assert.Contains("redacted", json, StringComparison.Ordinal);
        }

        using var doc = JsonDocument.Parse(shown[2]);
        Assert.Equal("<redacted>", doc.RootElement.GetProperty("Token").GetString());
        Assert.Equal("<redacted>", doc.RootElement.GetProperty("KeySecret").GetString());
        Assert.Equal("<redacted>", doc.RootElement.GetProperty("ClientKeyPassword").GetString());
        Assert.Equal("http://<redacted>@proxy.example:3128", doc.RootElement.GetProperty("Proxy").GetString());
        Assert.Equal("ak_test", doc.RootElement.GetProperty("KeyId").GetString());
        Assert.Equal("<redacted>", JsonDocument.Parse(shown[0]).RootElement.GetProperty("Access").GetString());

        // A log line through Microsoft.Extensions.Logging formats a token with ToString.
        var logger = new CapturingLogger();
#pragma warning disable CA1848, CA1873 // logged the way a caller's own code would
        Microsoft.Extensions.Logging.LoggerExtensions.LogInformation(logger, "token {Token} options {Options}", new Token("tok-s3cr3t"), new ClientCredentialsOptions { KeyId = "ak_test", KeySecret = "s3cr3t", Scopes = ["identity:read"] });
#pragma warning restore CA1848, CA1873
        Assert.NotEmpty(logger.Lines);
        Assert.All(logger.Lines, line => Assert.DoesNotContain("s3cr3t", line, StringComparison.Ordinal));
    }

    private sealed class CapturingLogger : Microsoft.Extensions.Logging.ILogger
    {
        internal List<string> Lines { get; } = [];

        public IDisposable? BeginScope<TState>(TState state)
            where TState : notnull => null;

        public bool IsEnabled(Microsoft.Extensions.Logging.LogLevel logLevel) => true;

        public void Log<TState>(Microsoft.Extensions.Logging.LogLevel logLevel, Microsoft.Extensions.Logging.EventId eventId, TState state, Exception? exception, Func<TState, Exception?, string> formatter)
        {
            Lines.Add(formatter(state, exception));
            if (state is IEnumerable<KeyValuePair<string, object?>> pairs)
            {
                foreach (var (_, value) in pairs)
                {
                    Lines.Add(value?.ToString() ?? string.Empty);
                    Lines.Add(JsonSerializer.Serialize(value));
                }
            }
        }
    }

    [Fact]
    public void Internal_plans_never_print_a_secret()
    {
        var plan = new CredentialPlan("env", "client_credentials") { KeyId = "ak_test", KeySecret = "s3cr3t", Token = "tok-s3cr3t" };
        var proxy = new ProxyChoice("http://ana:pw-s3cr3t@proxy.example:3128", true);
        foreach (var text in new[] { plan.ToString(), proxy.ToString(), (plan with { Kind = "static_token" }).ToString() })
        {
            Assert.DoesNotContain("s3cr3t", text, StringComparison.Ordinal);
            Assert.Contains("<redacted>", text, StringComparison.Ordinal);
        }

        Assert.Contains("ak_test", plan.ToString(), StringComparison.Ordinal);
        Assert.Contains("proxy.example:3128", proxy.ToString(), StringComparison.Ordinal);
    }

    [Theory]
    [InlineData("http://127.0.0.1:8080", true)]
    [InlineData("http://127.1.2.3", true)]
    [InlineData("http://[::1]:8080", true)]
    [InlineData("http://localhost:3000", true)]
    [InlineData("http://127.example.com", false)]
    [InlineData("http://127.0.0.1.example.com", false)]
    public void Plain_http_goes_only_to_this_machine(string url, bool allowed)
    {
        var ok = Record.Exception(() => Urls.Check("the base URL", new Uri(url), true)) is null;
        Assert.Equal(allowed, ok);
    }

    [Fact]
    public void Every_known_code_has_its_status_and_an_unknown_one_is_kept()
    {
        Assert.Equal(19, new[]
        {
            Code.BadRequest, Code.FailedPrecondition, Code.Unauthenticated, Code.Forbidden, Code.NotFound,
            Code.MethodNotAllowed, Code.AlreadyExists, Code.Conflict, Code.PayloadTooLarge, Code.UnsupportedMediaType,
            Code.Unprocessable, Code.UpgradeRequired,
            Code.RateLimited, Code.QuotaExceeded, Code.Cancelled, Code.Internal, Code.Unimplemented, Code.Unavailable,
            Code.Timeout,
        }.Length);
        Assert.Equal(429, Code.QuotaExceeded.HttpStatus);
        Assert.Equal(422, Code.Unprocessable.HttpStatus);
        Assert.Equal(Code.Unprocessable, Code.ForStatus(422));
        Assert.Equal(426, Code.UpgradeRequired.HttpStatus);
        Assert.Equal(Code.UpgradeRequired, Code.ForStatus(426));
        var newer = new Code("teapot_refused");
        Assert.False(newer.IsKnown);
        Assert.Equal("teapot_refused", newer.ToString());
        Assert.Equal(Code.Forbidden, Code.ForStatus(403));
        Assert.Equal("http_418", Code.ForStatus(418).Value);
        Assert.Equal(Code.Internal, Code.ForStatus(502));
    }

    [Fact]
    public void A_path_parameter_is_one_segment()
    {
        Assert.Equal("a%2Fb%20c", Codegen.PathSegment("a/b c"));
        Assert.Equal("ok-._~", Codegen.PathSegment("ok-._~"));
        Assert.Equal("%C5%A1", Codegen.PathSegment("š"));
    }

    [Fact]
    public void A_64_bit_integer_travels_as_a_decimal_string()
    {
        var read = JsonSerializer.Deserialize<WithInt64>("""{"n":"9007199254740993","list":["1",2]}""", Json.Options)!;
        Assert.Equal(9_007_199_254_740_993L, read.N);
        Assert.Equal([1L, 2L], read.List);
        var written = JsonSerializer.Serialize(new WithInt64 { N = 9_007_199_254_740_993L, List = [3L] }, Json.Options);
        Assert.Equal("""{"n":"9007199254740993","list":["3"]}""", written);
    }

    [Fact]
    public void A_string_enum_keeps_a_value_it_does_not_know()
    {
        var known = JsonSerializer.Deserialize<Colour>("\"red\"", Json.Options);
        Assert.Equal(Colour.Red, known);
        var newer = JsonSerializer.Deserialize<Colour>("\"ultraviolet\"", Json.Options);
        Assert.Equal("ultraviolet", newer.Value);
        Assert.Equal("\"ultraviolet\"", JsonSerializer.Serialize(newer, Json.Options));
    }

    [Fact]
    public void Details_are_typed_and_an_unknown_one_is_kept()
    {
        using var doc = JsonDocument.Parse("""[{"type":"field","field":"url","description":"not https"},{"type":"retry","after_seconds":"7"},{"type":"later","x":1}]""");
        var details = new List<Detail>();
        foreach (var d in doc.RootElement.EnumerateArray())
        {
            details.Add(Detail.Read(d));
        }

        Assert.Equal(new FieldDetail("url", "not https"), details[0]);
        Assert.Equal(new RetryDetail(7), details[1]);
        Assert.IsType<UnknownDetail>(details[2]);
    }

    [Fact]
    public void A_client_needs_a_credential_and_https()
    {
        var none = Assert.Throws<ConfigException>(() => new Client<PublicProfile>(new ClientOptions()));
        Assert.Contains("INORBIT_TOKEN", none.Message, StringComparison.Ordinal);
        var noScopes = Assert.Throws<ConfigException>(() => new Client<PublicProfile>(new ClientOptions { KeyId = "ak_1", KeySecret = "s" }));
        Assert.Contains("SCOPES", noScopes.Message, StringComparison.Ordinal);
        Assert.Throws<ConfigException>(() => new Client<PublicProfile>(new ClientOptions { Token = "t", BaseUrl = new Uri("http://api.example.com") }));
        Assert.Throws<ConfigException>(() => new Client<PublicProfile>(new ClientOptions { Token = "t", BaseUrl = new Uri("https://api.example.com/v1") }));
        using var local = new Client<PublicProfile>(new ClientOptions { Token = "t", BaseUrl = new Uri("http://127.0.0.1:8080") });
        Assert.Equal("127.0.0.1", local.BaseUrl.Host);
    }

    [Fact]
    public void A_named_profile_reads_only_its_own_variables()
    {
        var before = Environment.GetEnvironmentVariable("INORBIT_TOKEN");
        Environment.SetEnvironmentVariable("INORBIT_TOKEN", "bare");
        try
        {
            var e = Assert.Throws<ConfigException>(() => Client.FromEnv<NamedProfile>());
            Assert.Contains("INORBIT_ACME_CI_TOKEN", e.Message, StringComparison.Ordinal);
            using var bare = Client.FromEnv();
            Assert.Equal("api.inorbit.hr", bare.BaseUrl.Host);
        }
        finally
        {
            Environment.SetEnvironmentVariable("INORBIT_TOKEN", before);
        }
    }

    [Fact]
    public void A_raw_answer_never_prints_its_body()
    {
        var raw = new RawResponse(200, new Dictionary<string, IReadOnlyList<string>>(), "{\"secret\":\"x\"}"u8.ToArray(), "iohr-1", 1);
        Assert.DoesNotContain("secret", raw.ToString(), StringComparison.Ordinal);
    }

    private sealed record WithInt64
    {
        [JsonPropertyName("n")]
        [JsonNumberHandling(JsonNumberHandling.AllowReadingFromString | JsonNumberHandling.WriteAsString)]
        public long N { get; init; }

        [JsonPropertyName("list")]
        [JsonNumberHandling(JsonNumberHandling.AllowReadingFromString | JsonNumberHandling.WriteAsString)]
        public IReadOnlyList<long> List { get; init; } = [];
    }

    [JsonConverter(typeof(StringValueConverter<Colour>))]
    private readonly record struct Colour(string Value) : IStringValue<Colour>
    {
        public static Colour Red { get; } = new("red");

        static Colour IStringValue<Colour>.From(string value) => new(value);
    }

    private sealed class NamedProfile : IProfile
    {
        public static string Name => "acme-ci";

        public static string Env => "ACME_CI";
    }

    [Fact]
    public void A_timestamp_reads_empty_as_no_value()
    {
        Assert.Null(Timestamps.Parse(""));
        Assert.Equal(new DateTimeOffset(2026, 10, 4, 8, 0, 0, 500, TimeSpan.Zero), Timestamps.Parse("2026-10-04T08:00:00.5Z"));
        Assert.Throws<FormatException>(() => Timestamps.Parse("yesterday"));
        Assert.Throws<FormatException>(() => Timestamps.Parse("2026-10-04"));
    }
}
