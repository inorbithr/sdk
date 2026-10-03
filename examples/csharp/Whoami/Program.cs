// Who the API thinks you are, and the latest radar digests, with the public surface.
// Run with INORBIT_TOKEN set (an API token from the console or `iohr token create`).
using System;
using InOrbit.Sdk;
using InOrbit.Sdk.Api;

using var client = Client.FromEnv();
try
{
    var me = (await client.MeAsync()).Value;
    Console.WriteLine($"subject {me.Subject}, scopes {string.Join(' ', me.Scopes)}");
    var page = (await client.Radar().ListDigestsAsync(new RadarListDigestsParams { Limit = 3 })).Value;
    foreach (var d in page.Digests)
    {
        Console.WriteLine($"{d.Week} {d.Language}: {d.Summary}");
    }
}
catch (ApiException e) when (e.Code == Code.Forbidden)
{
    Console.Error.WriteLine("the token lacks a scope this example needs (identity:read, radar:read)");
}
