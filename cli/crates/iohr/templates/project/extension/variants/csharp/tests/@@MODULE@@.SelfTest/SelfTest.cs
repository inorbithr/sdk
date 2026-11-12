using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Net.Http;
using System.Net.Sockets;
using System.Text;
using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;

namespace @@MODULE@@.SelfTest;

/// <summary>
/// The self-test: `dotnet run --project tests/@@MODULE@@.SelfTest` runs one heartbeat with a STATE
/// and CACHE of its own (or the folders `iohr ext dev --test` and InOrbit Desktop's smoke test
/// give), then checks what it wrote and where. No package, so there is nothing to download.
/// </summary>
internal static class Program
{
    private static int failed;

    private static void Check(bool ok, string what)
    {
        if (!ok)
        {
            failed++;
        }

        Console.WriteLine($"{(ok ? "ok  " : "FAIL")} {what}");
    }

    private static Dictionary<string, string> Env(string state, string cache, params (string, string)[] more)
    {
        var e = new Dictionary<string, string> { ["STATE"] = state, ["CACHE"] = cache };
        foreach (var (k, v) in more)
        {
            e[k] = v;
        }

        return e;
    }

    private static async Task<int> Main()
    {
        var state = Directory.CreateTempSubdirectory("state-").FullName;
        var cache = Directory.CreateTempSubdirectory("cache-").FullName;
        var before = Directory.GetFileSystemEntries(".").Length;

        Check(await Service.Run(Env(state, cache, ("IOHR_EXT_ONCE", "1")), CancellationToken.None) == 0, "one heartbeat exits 0");
        using var beat = JsonDocument.Parse(File.ReadAllText(Path.Combine(state, "heartbeat.json")));
        var root = beat.RootElement;
        Check(
            root.GetProperty("id").GetString() == "@@ID@@" && root.GetProperty("state").GetString() == "ok" && root.GetProperty("beats").GetInt32() == 1,
            "the heartbeat says who and ok");

        // Granted when you are signed in and the channel is up, refused when not: either way it answered.
        Check(root.GetProperty("token").GetString() is "granted" or "refused", "the token ask was answered");
        Check(Directory.GetFileSystemEntries(".").Length == before, "nothing was written into the project folder");
        Check(await Service.Run(new Dictionary<string, string>(), CancellationToken.None) == 2, "no STATE and CACHE is an error, exit 2");

        var refused = false;
        try
        {
            await Channel.Token(["tokens:write"], new Dictionary<string, string>());
        }
        catch (RefusedException)
        {
            refused = true;
        }

        Check(refused, "without the channel the token ask is refused");

        // A fake channel: the one scope is granted, any other is refused.
        var socket = Path.Combine(Directory.CreateTempSubdirectory("chan-").FullName, "t.sock");
        using (var server = new Socket(AddressFamily.Unix, SocketType.Stream, ProtocolType.Unspecified))
        {
            server.Bind(new UnixDomainSocketEndPoint(socket));
            server.Listen(2);
            var fake = Task.Run(async () =>
            {
                for (var i = 0; i < 2; i++)
                {
                    using var c = await server.AcceptAsync();
                    var buffer = new byte[4096];
                    var n = 0;

                    // Read until the whole request (head and JSON body) has arrived.
                    while (n < buffer.Length && !Encoding.UTF8.GetString(buffer, 0, n).Contains("]}", StringComparison.Ordinal))
                    {
                        var got = await c.ReceiveAsync(buffer.AsMemory(n));
                        if (got == 0)
                        {
                            break;
                        }

                        n += got;
                    }

                    var ok = Encoding.UTF8.GetString(buffer, 0, n).Contains("identity:read", StringComparison.Ordinal);
                    var body = ok ? "{\"access_token\":\"t\",\"expires_at\":\"x\"}" : "{\"error\":\"scope\",\"message\":\"not declared\"}";
                    await c.SendAsync(Encoding.UTF8.GetBytes($"HTTP/1.1 {(ok ? "200 OK" : "403 Forbidden")}\r\ncontent-length: {body.Length}\r\n\r\n{body}"));
                }
            });
            var env = new Dictionary<string, string> { ["IOHR_EXT_TOKEN_SOCKET"] = socket };
            Check(await Channel.Token(Service.Scopes, env) == "t", "a declared scope is granted by the channel");
            var why = string.Empty;
            try
            {
                await Channel.Token(["tokens:write"], env);
            }
            catch (RefusedException e)
            {
                why = e.Message;
            }

            Check(why.Contains("403", StringComparison.Ordinal), "an undeclared scope is refused by the channel");
            await fake;
        }

        var latest = JsonSerializer.Deserialize<Dictionary<string, JsonElement>>(beat.RootElement.GetRawText())!
            .ToDictionary(p => p.Key, p => (object)p.Value);
        await using var panel = await Service.Serve(Env(state, cache, ("PANEL_FILE", "panel/index.html")), state, () => latest);
        var url = JsonElement.Parse(File.ReadAllText(Path.Combine(state, "panel.json"))).GetProperty("url").GetString()!;
        using var http = new HttpClient();
        var api = await http.GetAsync(new Uri(url + "api/heartbeat"));
        Check(api.IsSuccessStatusCode && (await api.Content.ReadAsStringAsync()).Contains("\"beats\":1", StringComparison.Ordinal), "the panel serves the heartbeat as JSON");
        Check((await http.GetStringAsync(new Uri(url))).Contains("/api/heartbeat", StringComparison.Ordinal), "the panel page reads /api/heartbeat");
        Check((await http.GetAsync(new Uri(url + "nope"))).StatusCode == System.Net.HttpStatusCode.NotFound, "anything else is a 404");
        await panel.StopAsync();

        Console.WriteLine(failed == 0 ? "selftest passed" : $"{failed} check(s) failed");
        return failed == 0 ? 0 : 1;
    }
}
