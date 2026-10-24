// A server-sent-events stream: the account's events, as they happen. Each event is typed;
// the loop ends when the server ends the stream. Needs events:read.
using System;
using System.Threading.Tasks;
using InOrbit.Sdk;
using InOrbit.Sdk.Api;

internal static class StreamSse
{
    public static async Task RunAsync()
    {
        using var client = Client.FromEnv(); // streams open as SSE by default
        var n = 0;
        await foreach (var e in client.Events().StreamEventsAsync())
        {
            n++;
            Console.WriteLine($"{e.OccurredAt} {e.Type} {e.Id}");
        }

        Console.WriteLine($"{n} events");
    }
}
