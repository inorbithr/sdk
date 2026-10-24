// Configuration by Client.Load: code, the environment, the iohr config file and the iohr
// login. Config.Describe() says where each value came from, without printing a secret.
using System;
using System.Threading.Tasks;
using InOrbit.Sdk;
using InOrbit.Sdk.Api;

internal static class ConfigLoad
{
    public static async Task RunAsync()
    {
        Client<PublicProfile> client;
        try
        {
            client = Client.Load(new ClientOptions { Timeout = TimeSpan.FromSeconds(10) });
        }
        catch (ConfigException e)
        {
            Console.Error.WriteLine(e.Message); // every problem, with its setting
            Environment.Exit(1);
            return;
        }

        using (client)
        {
            Console.WriteLine($"credential from {client.Config.Describe()["credential"]?["source"]}");
            var me = (await client.MeAsync()).Value;
            Console.WriteLine($"{me.Subject}, a {me.Kind}");
        }
    }
}
