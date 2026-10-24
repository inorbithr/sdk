// The SDK retries for you. The API answers 429 with Retry-After; the SDK waits that long and
// tries again, and the answer says how many attempts it took. Tune it with MaxRetries.
using System;
using System.Threading.Tasks;
using InOrbit.Sdk;
using InOrbit.Sdk.Api;

internal static class Retries
{
    public static async Task RunAsync()
    {
        using var client = Client.Load(new ClientOptions { MaxRetries = 3 });
        var (me, raw) = await client.MeAsync();
        Console.WriteLine($"{me.Subject} after {raw.Attempts} attempts");
    }
}
