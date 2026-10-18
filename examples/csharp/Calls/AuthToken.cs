// Auth with a token you already have: an API token from the console or `iohr token create`.
// The SDK sends it as a Bearer on every call and never exchanges it.
using System;
using System.Threading.Tasks;
using InOrbit.Sdk;
using InOrbit.Sdk.Api;

internal static class AuthToken
{
    public static async Task RunAsync()
    {
        var baseUrl = Environment.GetEnvironmentVariable("INORBIT_BASE_URL");
        using var client = new Client<PublicProfile>(new ClientOptions
        {
            Token = Environment.GetEnvironmentVariable("INORBIT_TOKEN"),
            BaseUrl = string.IsNullOrEmpty(baseUrl) ? null : new Uri(baseUrl),
        });
        var me = (await client.MeAsync()).Value;
        Console.WriteLine($"{me.Subject} signed in with a token");
    }
}
