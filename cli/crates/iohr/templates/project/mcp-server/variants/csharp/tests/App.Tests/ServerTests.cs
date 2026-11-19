// The passes-on-init test: the server is started the way a host starts it (a child process over
// stdio) and spoken to by the official client. It checks that the 2026-07-28 revision connects
// (server/discover, no handshake), that a client of the older revision still gets its
// initialize handshake, the tool list and its order, a tool call and its structured result, a
// refused bad argument, and the rules every tool must meet.
using System.Text.Json;
using ModelContextProtocol;
using ModelContextProtocol.Client;
using ModelContextProtocol.Protocol;
using Xunit;

namespace App.Tests;

public class ServerTests
{
    private static string ServerDll => Path.Combine(AppContext.BaseDirectory, "@@NAME@@.dll");

    private static Task<McpClient> Connect(string? version = null) =>
        McpClient.CreateAsync(
            new StdioClientTransport(new StdioClientTransportOptions
            {
                Name = "@@NAME@@",
                Command = "dotnet",
                Arguments = [ServerDll],
            }),
            new McpClientOptions { ProtocolVersion = version },
            cancellationToken: new CancellationTokenSource(TimeSpan.FromSeconds(60)).Token);

    [Fact]
    public async Task DiscoverAnswersOnThe20260728Revision()
    {
        await using var client = await Connect();
        Assert.Equal("@@NAME@@", client.ServerInfo?.Name);
        Assert.Equal("2026-07-28", client.NegotiatedProtocolVersion);
    }

    [Fact]
    public async Task AClientOfTheOlderRevisionStillGetsItsHandshake()
    {
        // initialize was removed in 2026-07-28; the SDK still answers it for 2025-11-25 hosts.
        await using var client = await Connect("2025-11-25");
        Assert.Equal("@@NAME@@", client.ServerInfo?.Name);
        Assert.Equal("2025-11-25", client.NegotiatedProtocolVersion);
    }

    [Fact]
    public async Task ToolsListNamesTheToolAndComesInTheSameOrderTwice()
    {
        await using var client = await Connect();
        var first = await client.ListToolsAsync();
        var second = await client.ListToolsAsync();
        Assert.Equal(["word_count"], first.Select(t => t.Name));
        Assert.Equal(first.Select(t => t.Name), second.Select(t => t.Name));
    }

    [Fact]
    public async Task AToolCallReturnsTheStructuredResult()
    {
        await using var client = await Connect();
        var res = await client.CallToolAsync("word_count", new Dictionary<string, object?> { ["text"] = "one two  three" });
        Assert.NotEqual(true, res.IsError);
        Assert.Equal(3, res.StructuredContent!.Value.GetProperty("words").GetInt32());
        Assert.Equal(14, res.StructuredContent!.Value.GetProperty("characters").GetInt32());
    }

    [Fact]
    public async Task ABadArgumentIsRefusedAndTheToolDoesNotRun()
    {
        await using var client = await Connect();
        var bad = new Dictionary<string, Dictionary<string, object?>>
        {
            ["too long"] = new() { ["text"] = new string('x', 10_001) },
            ["missing"] = new(),
            ["extra"] = new() { ["text"] = "a", ["other"] = 1 },
        };
        foreach (var (name, args) in bad)
        {
            try
            {
                var res = await client.CallToolAsync("word_count", args);
                Assert.True(res.IsError == true, $"{name}: was not refused");
            }
            catch (McpException)
            {
                // a protocol error is a refusal too
            }
        }
    }

    [Fact]
    public async Task TheResourceIsListedAndCanBeRead()
    {
        await using var client = await Connect();
        var listed = await client.ListResourcesAsync();
        Assert.Equal(["about://server"], listed.Select(r => r.Uri));
        var read = await client.ReadResourceAsync("about://server");
        Assert.Contains("counts words", ((TextResourceContents)read.Contents[0]).Text);
    }

    [Fact]
    public async Task EveryToolHasADescriptionAnAnnotationAndBoundedInputs()
    {
        await using var client = await Connect();
        foreach (var tool in await client.ListToolsAsync())
        {
            var p = tool.ProtocolTool;
            Assert.False(string.IsNullOrEmpty(p.Description), $"{p.Name}: no description");
            var a = p.Annotations;
            Assert.True(a is not null && (a.ReadOnlyHint == true || a.DestructiveHint is not null),
                $"{p.Name}: say whether it only reads (ReadOnly) or whether it destroys (Destructive)");
            foreach (var prop in p.InputSchema.GetProperty("properties").EnumerateObject())
            {
                var type = prop.Value.GetProperty("type").GetString();
                var has = (string k) => prop.Value.TryGetProperty(k, out _);
                if (type == "string")
                {
                    Assert.True(has("maxLength") || has("enum") || has("pattern"), $"{p.Name}.{prop.Name}: bound the string (MaxLength, enum or pattern)");
                }
                else if (type is "integer" or "number")
                {
                    Assert.True(has("minimum") || has("maximum"), $"{p.Name}.{prop.Name}: bound the number (Range)");
                }
            }
        }
    }
}
