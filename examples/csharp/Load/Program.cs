// A client configured by Client.Load (code, the environment, the iohr config file, the iohr
// login), a middleware of your own, and the effective configuration with every value's source.
// Run after `iohr login`, or with INORBIT_TOKEN set.
using System;
using System.Threading;
using System.Threading.Tasks;
using InOrbit.Sdk;
using InOrbit.Sdk.Api;

Client<PublicProfile> client;
try
{
    client = Client.Load(new ClientOptions
    {
        Timeout = TimeSpan.FromSeconds(10),
        Pipeline = p => p.AddPerRetry(new Team()),
    });
}
catch (ConfigException e)
{
    // Every problem, each with its setting and where the value came from.
    Console.Error.WriteLine(e.Message);
    return 1;
}

using (client)
{
    var described = client.Config.Describe();
    Console.WriteLine($"credential from {described["credential"]?["source"]}, pipeline {string.Join(" > ", described["pipeline"]!.AsArray())}");
    var (me, raw) = await client.MeAsync();
    Console.WriteLine($"subject {me.Subject}; request {raw.RequestId}; rate limit {raw.RateLimit}");
}

return 0;

/// <summary>Runs on every attempt, after <c>auth</c>: it sees the finished request.</summary>
internal sealed class Team : Middleware
{
    public override string Name => "team";

    public override ValueTask<SdkResponse> SendAsync(SdkRequest request, MiddlewareNext next, CancellationToken cancellationToken)
    {
        request.Headers["x-team"] = "payments";
        return next(request, cancellationToken);
    }
}
