// Auth with a token provider of your own, here a stand-in for a secret store. The SDK asks it
// before every attempt and calls InvalidateAsync when the API refused the last token.
// Never log the token.
using System;
using System.Threading;
using System.Threading.Tasks;
using InOrbit.Sdk;
using InOrbit.Sdk.Api;

internal static class AuthCustomToken
{
    public static async Task RunAsync()
    {
        var baseUrl = Environment.GetEnvironmentVariable("INORBIT_BASE_URL");
        using var client = new Client<PublicProfile>(new ClientOptions
        {
            TokenProvider = new Vault(),
            BaseUrl = string.IsNullOrEmpty(baseUrl) ? null : new Uri(baseUrl),
        });
        var me = (await client.MeAsync()).Value;
        Console.WriteLine($"{me.Subject} signed in with a custom provider");
    }

    private sealed class Vault : ITokenProvider
    {
        public ValueTask<Token> GetTokenAsync(CancellationToken cancellationToken) =>
            ValueTask.FromResult(new Token("custom-token-1")); // pass an expiry when you know it
    }
}
