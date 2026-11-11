using System;
using System.Collections.Generic;
using System.IO;
using System.Net.Sockets;
using System.Text;
using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;

namespace @@MODULE@@;

/// <summary>What <see cref="Channel.Token"/> throws when iohr refuses, or its channel cannot be reached.</summary>
public sealed class RefusedException : Exception
{
    public RefusedException(string message)
        : base(message)
    {
    }
}

/// <summary>
/// Tokens from iohr's extension channel (the SDK's ADR 0012; PRD 0084 slice 1).
/// iohr passes IOHR_EXT_TOKEN_SOCKET (a 0600 Unix socket). The program sends
/// <c>POST /token {"scopes": [...]}</c> and gets <c>{"access_token", "expires_at"}</c> for scopes
/// its manifest declares, and a refusal for any other. It is the only credential this project has:
/// never read IOHR_TOKEN or run `iohr auth token`.
/// </summary>
public static class Channel
{
    private const int MaxAnswer = 64 * 1024;
    private static readonly TimeSpan Wait = TimeSpan.FromSeconds(10);

    /// <summary>Returns an access token for the scopes; the exception message says iohr's reason.</summary>
    public static async Task<string> Token(IReadOnlyList<string> scopes, IReadOnlyDictionary<string, string> env)
    {
        ArgumentNullException.ThrowIfNull(scopes);
        ArgumentNullException.ThrowIfNull(env);
        var path = env.GetValueOrDefault("IOHR_EXT_TOKEN_SOCKET", string.Empty).Trim();
        if (path.Length == 0)
        {
            throw new RefusedException("IOHR_EXT_TOKEN_SOCKET is not set: run this with iohr ext dev");
        }

        var raw = new MemoryStream();
        try
        {
            using var cts = new CancellationTokenSource(Wait);
            using var socket = new Socket(AddressFamily.Unix, SocketType.Stream, ProtocolType.Unspecified);
            await socket.ConnectAsync(new UnixDomainSocketEndPoint(path), cts.Token).ConfigureAwait(false);
            var body = JsonSerializer.SerializeToUtf8Bytes(new Dictionary<string, string[]> { ["scopes"] = [.. scopes] });
            var head = Encoding.UTF8.GetBytes(
                $"POST /token HTTP/1.1\r\nhost: iohr\r\ncontent-type: application/json\r\ncontent-length: {body.Length}\r\nconnection: close\r\n\r\n");
            await socket.SendAsync(head, cts.Token).ConfigureAwait(false);
            await socket.SendAsync(body, cts.Token).ConfigureAwait(false);
            var buffer = new byte[8192];
            while (raw.Length < MaxAnswer)
            {
                var n = await socket.ReceiveAsync(buffer, cts.Token).ConfigureAwait(false);
                if (n == 0)
                {
                    break;
                }

                raw.Write(buffer, 0, n);
            }
        }
        catch (Exception e) when (e is SocketException or IOException or OperationCanceledException)
        {
            throw new RefusedException($"cannot reach iohr's token channel: {e.Message}");
        }

        var text = Encoding.UTF8.GetString(raw.GetBuffer(), 0, (int)Math.Min(raw.Length, MaxAnswer));
        var cut = text.IndexOf("\r\n\r\n", StringComparison.Ordinal);
        var top = cut < 0 ? text : text[..cut];
        var payload = cut < 0 ? string.Empty : text[(cut + 4)..];
        var fields = top.Split(' ');
        var status = fields.Length > 1 ? fields[1] : "no answer";
        JsonElement answer = default;
        try
        {
            using var doc = JsonDocument.Parse(payload);
            answer = doc.RootElement.Clone();
        }
        catch (JsonException)
        {
            // Not JSON: the status below says what happened.
        }

        string Field(string key) =>
            answer.ValueKind == JsonValueKind.Object && answer.TryGetProperty(key, out var v) && v.ValueKind == JsonValueKind.String
                ? v.GetString() ?? string.Empty
                : string.Empty;

        if (status != "200")
        {
            throw new RefusedException($"{status} {Field("error")}: {Field("message")}");
        }

        var access = Field("access_token");
        return access.Length == 0 ? throw new RefusedException("iohr's answer holds no token") : access;
    }
}
