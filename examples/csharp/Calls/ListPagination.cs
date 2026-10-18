// A paged list. AllListMonitorsAsync yields every item and follows next_page_token for you;
// the second half reads one page by hand with PageSize.
// Run with INORBIT_ORG_ID and a credential with connections:read.
using System;
using System.Threading.Tasks;
using InOrbit.Sdk;
using InOrbit.Sdk.Api;

internal static class ListPagination
{
    public static async Task RunAsync()
    {
        using var client = Client.FromEnv();
        var org = Environment.GetEnvironmentVariable("INORBIT_ORG_ID") ?? "";

        var n = 0;
        await foreach (var m in client.Connections().AllListMonitorsAsync(org, new ConnectionsListMonitorsParams { PageSize = 2 }))
        {
            n++;
            Console.WriteLine($"{m.MonitorId} {m.Name}");
        }

        Console.WriteLine($"{n} monitors");

        var page = (await client.Connections().ListMonitorsAsync(org, new ConnectionsListMonitorsParams { PageSize = 1 })).Value;
        Console.WriteLine($"one page: {page.Monitors.Count} monitor, more after \"{page.NextPageToken}\"");
    }
}
