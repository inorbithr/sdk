using System;

namespace @@MODULE@@;

/// <summary>@@NAME@@: an offline eval suite with a pass bar. Base class library only, no network.</summary>
public static class Program
{
    /// <summary>Runs the suite; the exit code is 0 met, 1 missed, 2 broken suite.</summary>
    public static int Main(string[] args) => Suite.Run(args, Console.Out, Console.Error);
}
