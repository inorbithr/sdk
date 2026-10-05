using System;
using System.Collections.Generic;
using System.Linq;
using System.Net;
using System.Net.Http;
using System.Text.RegularExpressions;
using System.Threading;
using System.Threading.Tasks;
using InOrbit.Sdk.Api;
using Xunit;

namespace InOrbit.Sdk.Tests;

/// <summary>Configuration and the pipeline (docs/config.md) where the conformance cases do not reach.</summary>
public class M6Tests
{
    private static readonly LoadOptions Nothing = new() { Environment = new Dictionary<string, string>(), NoHome = true };

    [Fact]
    public void The_built_in_pipeline_is_in_order_and_edited_by_name()
    {
        var names = Client.LoadConfig(new ClientOptions { Token = "t" }, Nothing).Describe()["pipeline"]!.AsArray().Select(n => n!.GetValue<string>());
        Assert.Equal(["request_id", "user_agent", "idempotency_key", "call_tracing", "deadline", "retry", "auth", "rate_limit", "attempt_tracing", "logging", "hooks", "timeout"], names);

        var probe = Middleware.Create("probe", (r, n, c) => n(r, c));
        using var client = Client.Load(new ClientOptions { Token = "t", Pipeline = p => p.AddPerCall(probe).Replace("rate_limit", Middleware.Create("mine", (r, n, c) => n(r, c))).Remove("logging") }, Nothing);
        var edited = client.Config.Describe()["pipeline"]!.AsArray().Select(n => n!.GetValue<string>()).ToList();
        Assert.Equal(edited.IndexOf("retry") - 1, edited.IndexOf("probe"));
        Assert.Contains("rate_limit", edited);
        Assert.DoesNotContain("logging", edited);
    }

    [Theory]
    [InlineData("retry")]
    [InlineData("auth")]
    [InlineData("timeout")]
    public void Retry_auth_and_timeout_cannot_be_removed(string name)
    {
        var e = Assert.Throws<ConfigException>(() => Client.Load(new ClientOptions { Token = "t", Pipeline = p => p.Remove(name) }, Nothing));
        Assert.Contains("only replaced", e.Message, StringComparison.Ordinal);
    }

    [Fact]
    public void A_name_is_unique_and_must_exist()
    {
        var probe = Middleware.Create("request_id", (r, n, c) => n(r, c));
        Assert.Throws<ConfigException>(() => Client.Load(new ClientOptions { Token = "t", Pipeline = p => p.AddPerRetry(probe) }, Nothing));
        Assert.Throws<ConfigException>(() => Client.Load(new ClientOptions { Token = "t", Pipeline = p => p.InsertAfter("nope", Middleware.Create("x", (r, n, c) => n(r, c))) }, Nothing));
    }

    [Fact]
    public void The_user_agent_uses_the_shared_vocabulary()
    {
        Assert.Matches(new Regex(@"^inorbithr-sdk-csharp/\d+\.\d+\.\d+ dotnet/\d+\.\d+\.\d+ (linux|macos|windows|freebsd|android|ios|other)/(x86_64|aarch64|x86|arm|riscv64|other) app/1$"), Transport.UserAgent("app/1"));
    }

    [Fact]
    public async Task A_caller_key_on_an_operation_without_the_mark_is_refused()
    {
        using var client = new Client<PublicProfile>(new ClientOptions { Token = "t", HttpMessageHandler = new Answer(_ => new HttpResponseMessage(HttpStatusCode.OK)) });
        var e = await Assert.ThrowsAsync<ConfigException>(() => client.WithOptions(new CallOptions { IdempotencyKey = "k" }).MeAsync(TestContext.Current.CancellationToken));
        Assert.Contains("does not take an idempotency key", e.Message, StringComparison.Ordinal);
    }

    [Fact]
    public async Task A_delegating_handler_sits_in_the_pipeline()
    {
        var seen = new List<string?>();
        using var client = new Client<PublicProfile>(new ClientOptions
        {
            Token = "t",
            HttpMessageHandler = new Answer(r =>
            {
                seen.Add(r.Headers.TryGetValues("x-team", out var v) ? v.Single() : null);
                return new HttpResponseMessage(HttpStatusCode.OK) { Content = new StringContent("""{"subject":"ak_1"}""") };
            }),
            Pipeline = p => p.AddPerRetry(Middleware.FromHandler("team", new Team())),
        });
        var me = await client.MeAsync(TestContext.Current.CancellationToken);
        Assert.Equal("ak_1", me.Value.Subject);
        Assert.Equal(["payments"], seen);
    }

    [Fact]
    public async Task A_refused_static_token_from_load_is_an_auth_error_and_retries_fire_on_retry()
    {
        var calls = 0;
        var hook = new Retries();
        using var client = Client.Load(
            new ClientOptions
            {
                BaseUrl = new Uri("http://127.0.0.1:1"),
                Hooks = [hook],
                RetryBaseDelay = TimeSpan.FromMilliseconds(1),
                HttpMessageHandler = new Answer(_ => new HttpResponseMessage(++calls == 1 ? HttpStatusCode.ServiceUnavailable : HttpStatusCode.Unauthorized)),
            },
            new LoadOptions { Environment = new Dictionary<string, string> { ["INORBIT_TOKEN"] = "t" }, NoHome = true });
        var e = await Assert.ThrowsAsync<AuthException>(() => client.MeAsync(TestContext.Current.CancellationToken));
        Assert.Contains("refused the token", e.Message, StringComparison.Ordinal);
        Assert.StartsWith("iohr-", e.RequestId, StringComparison.Ordinal);
        Assert.Equal(["503"], hook.Reasons);
    }

    [Fact]
    public async Task One_refresh_however_many_wait()
    {
        var source = new Slow();
        var cached = new CachedToken(source);
        var tokens = await Task.WhenAll(Enumerable.Range(0, 8).Select(_ => cached.GetTokenAsync(CancellationToken.None).AsTask()));
        Assert.Equal(1, source.Calls);
        Assert.All(tokens, t => Assert.Equal("t1", t.Access));
        Assert.DoesNotContain("t1", cached.ToString(), StringComparison.Ordinal);
    }

    [Fact]
    public void Describe_never_shows_a_secret()
    {
        var config = Client.LoadConfig(
            null,
            new LoadOptions
            {
                Environment = new Dictionary<string, string>
                {
                    ["INORBIT_KEY_ID"] = "ak_1",
                    ["INORBIT_KEY_SECRET"] = "s3cr3t",
                    ["INORBIT_SCOPES"] = "identity:read",
                    ["INORBIT_PROXY"] = "http://ana:pw@proxy.example:3128",
                },
                NoHome = true,
            });
        var text = config.ToString();
        Assert.DoesNotContain("s3cr3t", text, StringComparison.Ordinal);
        Assert.DoesNotContain("pw@", text, StringComparison.Ordinal);
        Assert.Contains("ak_1", text, StringComparison.Ordinal);
    }

    [Fact]
    public void Explicit_construction_reads_no_environment()
    {
        using var client = new Client<PublicProfile>(new ClientOptions { Token = "t" });
        var doc = client.Config.Describe();
        Assert.Equal("code", doc["credential"]!["source"]!.GetValue<string>());
        Assert.Null(doc["config_file"]);
        Assert.Equal("120s", doc["settings"]!["total_timeout"]!["value"]!.GetValue<string>());
    }

    private sealed class Answer(Func<HttpRequestMessage, HttpResponseMessage> answer) : HttpMessageHandler
    {
        protected override Task<HttpResponseMessage> SendAsync(HttpRequestMessage request, CancellationToken cancellationToken) =>
            Task.FromResult(answer(request));
    }

    private sealed class Team : DelegatingHandler
    {
        protected override Task<HttpResponseMessage> SendAsync(HttpRequestMessage request, CancellationToken cancellationToken)
        {
            request.Headers.Add("x-team", "payments");
            return base.SendAsync(request, cancellationToken);
        }
    }

    private sealed class Retries : IHook
    {
        public List<string> Reasons { get; } = [];

        public void OnRetry(Attempt attempt, string reason, TimeSpan delay) => Reasons.Add(reason);
    }

    private sealed class Slow : ITokenProvider
    {
        private int _calls;

        public int Calls => _calls;

        public async ValueTask<Token> GetTokenAsync(CancellationToken cancellationToken)
        {
            Interlocked.Increment(ref _calls);
            await Task.Delay(50, cancellationToken);
            return new Token("t1", DateTimeOffset.UtcNow.AddMinutes(15));
        }
    }
}
