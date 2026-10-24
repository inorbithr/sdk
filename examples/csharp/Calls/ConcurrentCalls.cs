// Concurrent use: one client shared by several calls in flight with Task.WhenAll. The client
// is safe for concurrent use, and callers that need a token at the same moment share one exchange.
using System;
using System.Linq;
using System.Threading.Tasks;
using InOrbit.Sdk;
using InOrbit.Sdk.Api;

internal static class ConcurrentCalls
{
    public static async Task RunAsync()
    {
        using var client = Client.FromEnv();
        var answers = await Task.WhenAll(client.MeAsync(), client.MeAsync(), client.MeAsync());
        Console.WriteLine($"{answers.Length} calls at once: {string.Join(", ", answers.Select(a => a.Value.Subject))}");
    }
}
