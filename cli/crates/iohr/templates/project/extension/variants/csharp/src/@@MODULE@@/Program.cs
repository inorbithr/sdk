using System;
using System.Collections;
using System.Collections.Generic;
using System.Threading;
using System.Threading.Tasks;

namespace @@MODULE@@;

/// <summary>@@ID@@: the demo project's private macOS service. See <see cref="Service"/>.</summary>
internal static class Program
{
    private static async Task<int> Main()
    {
        var env = new Dictionary<string, string>();
        foreach (DictionaryEntry e in Environment.GetEnvironmentVariables())
        {
            env[(string)e.Key] = e.Value?.ToString() ?? string.Empty;
        }

        using var stop = new CancellationTokenSource();
        Console.CancelKeyPress += (_, a) =>
        {
            a.Cancel = true;
            stop.Cancel();
        };
        using var term = System.Runtime.InteropServices.PosixSignalRegistration.Create(
            System.Runtime.InteropServices.PosixSignal.SIGTERM, _ => stop.Cancel());
        return await Service.Run(env, stop.Token).ConfigureAwait(false);
    }
}
