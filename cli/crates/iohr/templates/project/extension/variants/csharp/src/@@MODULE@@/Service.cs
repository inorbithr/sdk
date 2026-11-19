using System;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;
using Microsoft.AspNetCore.Builder;
using Microsoft.AspNetCore.Hosting.Server;
using Microsoft.AspNetCore.Hosting.Server.Features;
using Microsoft.AspNetCore.Http;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Logging;

namespace @@MODULE@@;

/// <summary>
/// @@ID@@: the demo project's private macOS service.
/// Every minute it writes $STATE/heartbeat.json (the health InOrbit reads) and asks iohr's token
/// channel for a token with the one scope its manifest declares. It also serves a tiny panel on
/// 127.0.0.1 that InOrbit Desktop can show as a pane: the page reads /api/heartbeat.
/// Run it once with IOHR_EXT_ONCE=1 as the self-test does. The .NET shared framework only.
/// </summary>
public static class Service
{
    public const string Id = "@@ID@@";
    public const string Version = "0.1.0";
    public static readonly IReadOnlyList<string> Scopes = ["identity:read"];

    private static void WriteJson(string state, string name, object value)
    {
        var tmp = Path.Combine(state, $".{name}.tmp");
        File.WriteAllText(tmp, JsonSerializer.Serialize(value));
        if (!OperatingSystem.IsWindows())
        {
            File.SetUnixFileMode(tmp, UnixFileMode.UserRead | UnixFileMode.UserWrite);
        }

        File.Move(tmp, Path.Combine(state, name), overwrite: true);
    }

    /// <summary>One heartbeat: asks for a token (kept in memory only, never printed), writes heartbeat.json.</summary>
    public static async Task<Dictionary<string, object>> Heartbeat(IReadOnlyDictionary<string, string> env, string state, int beats)
    {
        ArgumentNullException.ThrowIfNull(env);
        var tok = "granted";
        try
        {
            await Channel.Token(Scopes, env).ConfigureAwait(false);
        }
        catch (RefusedException e)
        {
            await Console.Error.WriteLineAsync($"token refused: {e.Message}").ConfigureAwait(false);
            tok = "refused";
        }

        var now = DateTime.UtcNow.ToString("yyyy-MM-dd'T'HH:mm:ss'Z'", CultureInfo.InvariantCulture);
        var beat = new Dictionary<string, object>
        {
            ["id"] = Id,
            ["version"] = Version,
            ["state"] = "ok",
            ["at"] = now,
            ["beats"] = beats,
            ["token"] = tok,
            ["dev"] = env.GetValueOrDefault("IOHR_EXT_DEV", string.Empty) == "1",
        };
        WriteJson(state, "heartbeat.json", beat);
        Console.WriteLine($"heartbeat {beats} at {now} (token {tok})");
        return beat;
    }

    private static string PanelFile(IReadOnlyDictionary<string, string> env)
    {
        var named = env.GetValueOrDefault("PANEL_FILE", string.Empty);
        if (named.Length > 0)
        {
            return named;
        }

        var beside = Path.Combine(AppContext.BaseDirectory, "panel", "index.html");
        return File.Exists(beside) ? beside : Path.Combine("panel", "index.html");
    }

    /// <summary>Listens on 127.0.0.1 only; the port is PANEL_PORT or one the system picks, written to $STATE/panel.json.</summary>
    public static async Task<WebApplication> Serve(
        IReadOnlyDictionary<string, string> env,
        string state,
        Func<IReadOnlyDictionary<string, object>> latest)
    {
        ArgumentNullException.ThrowIfNull(env);
        ArgumentNullException.ThrowIfNull(latest);
        var raw = env.GetValueOrDefault("PANEL_PORT", string.Empty);
        if (!int.TryParse(raw.Length == 0 ? "0" : raw, NumberStyles.None, CultureInfo.InvariantCulture, out var port))
        {
            throw new InvalidOperationException($"PANEL_PORT is not a number: {raw}");
        }

        var builder = WebApplication.CreateSlimBuilder();
        builder.Logging.ClearProviders();
        var app = builder.Build();
        app.Urls.Add($"http://127.0.0.1:{port}");
        app.MapGet("/api/heartbeat", () => Results.Json(latest()));
        app.MapGet("/", () =>
        {
            var page = PanelFile(env);
            return Results.Content(File.Exists(page) ? File.ReadAllText(page) : "<p>panel/index.html is missing</p>", "text/html");
        });
        await app.StartAsync().ConfigureAwait(false);
        var url = app.Services.GetRequiredService<IServer>().Features.Get<IServerAddressesFeature>()!.Addresses.First().TrimEnd('/') + "/";
        WriteJson(state, "panel.json", new Dictionary<string, string> { ["id"] = Id, ["url"] = url });
        Console.WriteLine($"panel at {url}");
        return app;
    }

    /// <summary>Runs until the token is cancelled (SIGINT or SIGTERM) or once, with IOHR_EXT_ONCE; returns the exit code.</summary>
    public static async Task<int> Run(IReadOnlyDictionary<string, string> env, CancellationToken stop)
    {
        ArgumentNullException.ThrowIfNull(env);
        var state = env.GetValueOrDefault("STATE", string.Empty);
        if (state.Length == 0 || env.GetValueOrDefault("CACHE", string.Empty).Length == 0)
        {
            await Console.Error.WriteLineAsync("STATE and CACHE are not set: run this with iohr ext dev").ConfigureAwait(false);
            return 2;
        }

        var every = int.TryParse(env.GetValueOrDefault("HEARTBEAT_EVERY", string.Empty), NumberStyles.None, CultureInfo.InvariantCulture, out var n) && n > 0 ? n : 60;
        var once = env.GetValueOrDefault("IOHR_EXT_ONCE", string.Empty).Length > 0;
        Console.WriteLine($"{Id} {Version} starting; state in {state}");
        IReadOnlyDictionary<string, object> latest = new Dictionary<string, object>();
        WebApplication? server = null;
        try
        {
            if (!once)
            {
                server = await Serve(env, state, () => latest).ConfigureAwait(false);
            }

            for (var beats = 1; ; beats++)
            {
                latest = await Heartbeat(env, state, beats).ConfigureAwait(false);
                if (once)
                {
                    return 0;
                }

                try
                {
                    await Task.Delay(TimeSpan.FromSeconds(every), stop).ConfigureAwait(false);
                }
                catch (OperationCanceledException)
                {
                    Console.WriteLine("stopping");
                    return 0;
                }
            }
        }
        catch (Exception e) when (e is IOException or InvalidOperationException)
        {
            await Console.Error.WriteLineAsync(e.Message).ConfigureAwait(false);
            return 1;
        }
        finally
        {
            if (server is not null)
            {
                await server.StopAsync(CancellationToken.None).ConfigureAwait(false);
                await server.DisposeAsync().ConfigureAwait(false);
            }
        }
    }
}
