// One unary GET: who the API thinks you are. The answer carries the typed value and the raw
// response (request id, attempts, rate limit).
// Run with INORBIT_TOKEN set (or INORBIT_KEY_ID, INORBIT_KEY_SECRET and INORBIT_SCOPES).
using System;
using System.Threading.Tasks;
using InOrbit.Sdk;
using InOrbit.Sdk.Api;

internal static class UnaryGet
{
    public static async Task RunAsync()
    {
        using var client = Client.FromEnv();
        var (me, raw) = await client.MeAsync();
        Console.WriteLine($"subject {me.Subject}, scopes {string.Join(' ', me.Scopes)}, request {raw.RequestId}");
    }
}
