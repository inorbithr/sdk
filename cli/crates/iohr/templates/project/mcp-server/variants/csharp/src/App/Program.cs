// A stdio MCP server: the host starts this process and speaks to it over stdin and stdout.
// Logs go to stderr, because stdout belongs to the protocol.
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Hosting;
using Microsoft.Extensions.Logging;
using App;

var builder = Host.CreateApplicationBuilder(args);
builder.Logging.ClearProviders();
builder.Logging.AddConsole(o => o.LogToStandardErrorThreshold = LogLevel.Trace);
builder.Services
    .AddMcpServer(o =>
    {
        o.ServerInfo = new() { Name = "@@NAME@@", Version = "0.1.0" };
        o.ServerInstructions = "Counts the words and characters in a text. It reads nothing else and changes nothing.";
    })
    .WithStdioServerTransport()
    .WithTools<WordTools>()
    .WithResources<AboutResource>();
await builder.Build().RunAsync();
