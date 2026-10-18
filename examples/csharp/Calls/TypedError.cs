// Typed error handling. A refused call throws ApiException with the HTTP status, a stable
// code to branch on, the problem text and the request id to quote to support.
using System;
using System.Threading.Tasks;
using InOrbit.Sdk;
using InOrbit.Sdk.Api;

internal static class TypedError
{
    public static async Task RunAsync()
    {
        using var client = Client.FromEnv();
        try
        {
            await client.MeAsync();
        }
        catch (ApiException e) when (e.Code == Code.Forbidden)
        {
            Console.WriteLine($"refused: status {e.Status}, code {e.Code}, request {e.Raw.RequestId}");
        }
    }
}
