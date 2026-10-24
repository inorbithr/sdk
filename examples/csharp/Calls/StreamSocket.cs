// The same event stream over one WebSocket (/v1/ws) instead of SSE: set Streams to
// StreamTransport.Socket. Every stream of the client shares the connection. Needs events:read.
using System;
using System.Threading.Tasks;
using InOrbit.Sdk;
using InOrbit.Sdk.Api;

internal static class StreamSocket
{
    public static async Task RunAsync()
    {
        using var client = Client.Load(new ClientOptions { Streams = StreamTransport.Socket });
        var n = 0;
        await foreach (var e in client.Events().StreamEventsAsync())
        {
            n++;
            Console.WriteLine($"{e.OccurredAt} {e.Type} {e.Id}");
        }

        Console.WriteLine($"{n} events");
    }
}
