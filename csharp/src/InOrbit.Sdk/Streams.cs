using System;
using System.Buffers;
using System.Collections.Generic;
using System.IO;
using System.Net.WebSockets;
using System.Runtime.CompilerServices;
using System.Text;
using System.Text.Json;
using System.Threading;
using System.Threading.Channels;
using System.Threading.Tasks;

namespace InOrbit.Sdk;

/// <summary>Streams (design.md section 7): server-sent events, or calls on the one <c>/v1/ws</c> socket.</summary>
internal sealed partial class Transport
{
    /// <summary>The most one event's data may hold.</summary>
    internal const int MaxEvent = 1024 * 1024;

    private readonly StreamTransport _streams;
    private readonly TimeSpan _idle;
    private readonly object _socketGate = new();
    private SocketHub? _socket;

    internal IAsyncEnumerable<T> StreamAsync<T>(Operation op, CancellationToken cancellationToken)
    {
        if (_streams == StreamTransport.Sse)
        {
            return SseAsync<T>(op, cancellationToken);
        }

        if (op.Rpc is null)
        {
            throw new ConfigException($"{op} names no RPC (x-iohr-rpc), so it cannot open on the socket; use server-sent events");
        }

        SocketHub hub;
        lock (_socketGate)
        {
            hub = _socket ??= new SocketHub(this);
        }

        return hub.StreamAsync<T>(op, cancellationToken);
    }

    private async IAsyncEnumerable<T> SseAsync<T>(Operation op, [EnumeratorCancellation] CancellationToken cancellationToken)
    {
        var opened = await SendCoreAsync(op, stream: true, cancellationToken).ConfigureAwait(false);
        using var response = opened.Response!;
        var body = await response.Content.ReadAsStreamAsync(cancellationToken).ConfigureAwait(false);
        await using (body.ConfigureAwait(false))
        {
            var reader = new SseReader(body, _idle, BaseUrl.Host);
            while (await reader.NextAsync(cancellationToken).ConfigureAwait(false) is { } ev)
            {
                if (ev.Name == "error")
                {
                    throw StreamError(ev.Data, opened.Raw!);
                }

                if (ev.Name == "message")
                {
                    yield return Decode<T>(ev.Data, opened.Raw!);
                }
            }
        }
    }

    /// <summary>An error envelope that ended a stream, as the <see cref="ApiException"/> its code stands for.</summary>
    internal static ApiException StreamError(byte[] envelope, RawResponse opened)
    {
        var status = 500;
        try
        {
            using var doc = JsonDocument.Parse(envelope);
            if (doc.RootElement.ValueKind == JsonValueKind.Object && doc.RootElement.TryGetProperty("code", out var c) && c.ValueKind == JsonValueKind.String)
            {
                status = new Code(c.GetString() ?? string.Empty).HttpStatus ?? 500;
            }
        }
        catch (JsonException)
        {
            // Not JSON: the generic message for a 500 stands.
        }

        var raw = new RawResponse(status, new Dictionary<string, IReadOnlyList<string>>(StringComparer.Ordinal), envelope, opened.RequestId, opened.Attempts);
        return ApiException.From(raw);
    }

    internal static T Decode<T>(byte[] data, RawResponse opened)
    {
        try
        {
            return JsonSerializer.Deserialize<T>(data, Json.Options)!;
        }
        catch (Exception e) when (e is JsonException or NotSupportedException)
        {
            throw new DecodeException(e.Message, opened, e);
        }
    }

    /// <summary>Opens <c>/v1/ws</c> with a fresh token: the socket, or why not.</summary>
    internal async Task<(ClientWebSocket? Socket, int Status, InOrbitException? Error)> ConnectAsync(CancellationToken cancellationToken)
    {
        Token token;
        try
        {
            token = await _provider.GetTokenAsync(cancellationToken).ConfigureAwait(false);
        }
        catch (InOrbitException e)
        {
            return (null, 0, e);
        }
        catch (Exception e) when (e is not OperationCanceledException)
        {
            return (null, 0, new AuthException("the token provider failed: " + e.Message, string.Empty, e));
        }

        var scheme = BaseUrl.Scheme == Uri.UriSchemeHttps ? "wss" : "ws";
        var uri = new Uri($"{scheme}://{BaseUrl.Authority}/v1/ws");
        var ws = new ClientWebSocket();
        ws.Options.CollectHttpResponseDetails = true;
        ws.Options.SetRequestHeader("Authorization", "Bearer " + token.Access);
        ws.Options.SetRequestHeader("User-Agent", _userAgent);
        ws.Options.KeepAliveInterval = TimeSpan.FromSeconds(15);
        SocketKeepAlive.Bound(ws.Options, _idle);
        using var timer = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        timer.CancelAfter(_timeout);
        try
        {
            await ws.ConnectAsync(uri, timer.Token).ConfigureAwait(false);
            return (ws, 101, null);
        }
        catch (Exception e) when (e is WebSocketException or OperationCanceledException && !cancellationToken.IsCancellationRequested)
        {
            var status = (int)ws.HttpStatusCode;
            var headers = new Dictionary<string, IReadOnlyList<string>>(StringComparer.Ordinal);
            if (ws.HttpResponseHeaders is { } h)
            {
                foreach (var (name, values) in h)
                {
                    headers[name.ToLowerInvariant()] = [.. values];
                }
            }

            ws.Dispose();
            if (status == 401)
            {
                await _provider.InvalidateAsync().ConfigureAwait(false);
                return (null, 401, ApiException.From(new RawResponse(401, headers, [], Retry.RequestId(), 1)));
            }

            if (status is >= 400 and < 600)
            {
                return (null, status, ApiException.From(new RawResponse(status, headers, [], Retry.RequestId(), 1)));
            }

            return (null, 0, e is OperationCanceledException
                ? new RequestTimeoutException(BaseUrl.Host, _timeout)
                : new ConnectionException(BaseUrl.Host, e.Message, e));
        }
    }

    internal int MaxRetries => _maxRetries;

    internal TimeSpan IdleTimeout => _idle;
}

/// <summary>Sets <c>KeepAliveTimeout</c> where the runtime has it (.NET 9 and later): pings unanswered that long close the socket.</summary>
internal static class SocketKeepAlive
{
    private static readonly System.Reflection.PropertyInfo? Timeout =
        typeof(ClientWebSocketOptions).GetProperty("KeepAliveTimeout");

    internal static void Bound(ClientWebSocketOptions options, TimeSpan idle) => Timeout?.SetValue(options, idle);
}

/// <summary>One server-sent event: its name and its data.</summary>
internal sealed record SseEvent(string Name, byte[] Data);

/// <summary>The WHATWG event-stream rules over a body: lines end with LF, CRLF or CR; comments keep it alive; data lines join.</summary>
internal sealed class SseReader(Stream body, TimeSpan idle, string host)
{
    private readonly byte[] _chunk = new byte[16 * 1024];
    private readonly ArrayBufferWriter<byte> _line = new();
    private readonly ArrayBufferWriter<byte> _data = new();
    private int _pos;
    private int _len;
    private bool _afterCr;
    private string _name = "message";
    private bool _hasData;

    /// <summary>The next event, or <see langword="null"/> when the body ends.</summary>
    internal async Task<SseEvent?> NextAsync(CancellationToken cancellationToken)
    {
        while (true)
        {
            while (_pos < _len)
            {
                var b = _chunk[_pos++];
                if (_afterCr && b == (byte)'\n')
                {
                    _afterCr = false;
                    continue;
                }

                _afterCr = b == (byte)'\r';
                if (b is (byte)'\n' or (byte)'\r')
                {
                    if (Line() is { } ev)
                    {
                        return ev;
                    }

                    continue;
                }

                if (_line.WrittenCount >= Transport.MaxEvent)
                {
                    throw new TooLargeException("an event of the stream is larger than 1 MiB; refusing to read it");
                }

                _line.GetSpan(1)[0] = b;
                _line.Advance(1);
            }

            using var timer = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
            timer.CancelAfter(idle);
            try
            {
                _len = await body.ReadAsync(_chunk, timer.Token).ConfigureAwait(false);
            }
            catch (OperationCanceledException) when (!cancellationToken.IsCancellationRequested)
            {
                throw new RequestTimeoutException(host, idle, "the stream was silent for");
            }
            catch (IOException e)
            {
                throw new ConnectionException(host, e.Message, e);
            }

            _pos = 0;
            if (_len == 0)
            {
                return null;
            }
        }
    }

    /// <summary>Handles one line; a blank one dispatches the event gathered so far.</summary>
    private SseEvent? Line()
    {
        var line = _line.WrittenSpan;
        try
        {
            if (line.IsEmpty)
            {
                if (!_hasData)
                {
                    _name = "message";
                    return null;
                }

                var ev = new SseEvent(_name, _data.WrittenSpan.ToArray());
                _data.Clear();
                _hasData = false;
                _name = "message";
                return ev;
            }

            if (line[0] == (byte)':')
            {
                return null;
            }

            var colon = line.IndexOf((byte)':');
            var field = colon < 0 ? line : line[..colon];
            var value = colon < 0 ? [] : line[(colon + 1)..];
            if (!value.IsEmpty && value[0] == (byte)' ')
            {
                value = value[1..];
            }

            if (field.SequenceEqual("data"u8))
            {
                if (_hasData)
                {
                    _data.Write("\n"u8);
                }

                if (_data.WrittenCount + value.Length > Transport.MaxEvent)
                {
                    throw new TooLargeException("an event of the stream is larger than 1 MiB; refusing to read it");
                }

                _data.Write(value);
                _hasData = true;
            }
            else if (field.SequenceEqual("event"u8))
            {
                _name = value.IsEmpty ? "message" : Encoding.UTF8.GetString(value);
            }

            return null;
        }
        finally
        {
            _line.Clear();
        }
    }
}

/// <summary>
/// The client's one <c>/v1/ws</c> socket: opened by the first stream, closed after the last, every
/// stream a call on it. A close, a socket-level error other than <c>unauthenticated</c> or a dead
/// connection reconnects and issues again every call not yet ended, within the retry budget.
/// </summary>
internal sealed class SocketHub(Transport transport) : IDisposable
{
    private const int Queue = 64;

    private readonly object _gate = new();
    private readonly Dictionary<string, Call> _calls = new(StringComparer.Ordinal);
    private readonly SemaphoreSlim _send = new(1, 1);
    private ClientWebSocket? _ws;
    private Task? _loop;
    private CancellationTokenSource? _stop;
    private long _next;

    internal async IAsyncEnumerable<T> StreamAsync<T>(Operation op, [EnumeratorCancellation] CancellationToken cancellationToken)
    {
        var call = new Call(
            Interlocked.Increment(ref _next).ToString(System.Globalization.CultureInfo.InvariantCulture),
            op.Rpc!,
            op.CallBody?.ToArray() ?? "{}"u8.ToArray());
        var opened = new RawResponse(101, new Dictionary<string, IReadOnlyList<string>>(StringComparer.Ordinal), [], call.Id, 1);
        await RegisterAsync(call).ConfigureAwait(false);
        try
        {
            await foreach (var item in call.Items.Reader.ReadAllAsync(cancellationToken).ConfigureAwait(false))
            {
                if (item.Error is { } error)
                {
                    throw error;
                }

                yield return Transport.Decode<T>(item.Data!, opened);
            }
        }
        finally
        {
            await UnregisterAsync(call).ConfigureAwait(false);
        }
    }

    public void Dispose()
    {
        CancellationTokenSource? stop;
        ClientWebSocket? ws;
        lock (_gate)
        {
            stop = _stop;
            ws = _ws;
            _stop = null;
            _ws = null;
        }

        stop?.Cancel();
        ws?.Abort();
        ws?.Dispose();
        stop?.Dispose();
    }

    private async Task RegisterAsync(Call call)
    {
        ClientWebSocket? ws;
        lock (_gate)
        {
            _calls[call.Id] = call;
            ws = _ws;
            if (_loop is null)
            {
                _stop = new CancellationTokenSource();
                _loop = Task.Run(() => RunAsync(_stop.Token), CancellationToken.None);
                return;
            }
        }

        if (ws is not null)
        {
            await SendCallAsync(ws, call).ConfigureAwait(false);
        }
    }

    private async Task UnregisterAsync(Call call)
    {
        ClientWebSocket? ws;
        bool last;
        bool wasOpen;
        lock (_gate)
        {
            wasOpen = _calls.Remove(call.Id) && !call.Ended;
            last = _calls.Count == 0;
            ws = _ws;
        }

        if (wasOpen && ws is not null && call.Sent)
        {
            // Stopped by the caller: the server ends the call; what still comes for it is ignored.
            await SendAsync(ws, Frame(w =>
            {
                w.WriteString("type", "cancel");
                w.WriteString("id", call.Id);
            })).ConfigureAwait(false);
        }

        if (last)
        {
            CancellationTokenSource? stop;
            lock (_gate)
            {
                if (_calls.Count != 0)
                {
                    return;
                }

                stop = _stop;
            }

            if (ws is not null)
            {
                try
                {
                    using var timer = new CancellationTokenSource(TimeSpan.FromSeconds(2));
                    await ws.CloseOutputAsync(WebSocketCloseStatus.NormalClosure, "done", timer.Token).ConfigureAwait(false);
                }
                catch (Exception e) when (e is WebSocketException or OperationCanceledException or ObjectDisposedException)
                {
                    // Gone already.
                }
            }

            stop?.Cancel();
        }
    }

    /// <summary>The socket's life: connect, issue every open call, read until the socket ends, again while calls remain.</summary>
    private async Task RunAsync(CancellationToken stop)
    {
        var failures = 0;
        var refreshed = false;
        try
        {
            while (!stop.IsCancellationRequested)
            {
                lock (_gate)
                {
                    if (_calls.Count == 0)
                    {
                        return;
                    }
                }

                var (ws, status, error) = await transport.ConnectAsync(stop).ConfigureAwait(false);
                if (ws is null)
                {
                    if (status == 401 && !refreshed)
                    {
                        refreshed = true;
                        continue;
                    }

                    var retryable = status is 0 or 429 or 503 or 504 && error is not AuthException;
                    if (!retryable || failures >= transport.MaxRetries)
                    {
                        FailAll(error!);
                        return;
                    }

                    var wait = error is ApiException api && api.RetryAfterSeconds() is { } s ? TimeSpan.FromSeconds(Math.Min(s, 60)) : Retry.Backoff(failures);
                    failures++;
                    await Task.Delay(wait, stop).ConfigureAwait(false);
                    continue;
                }

                refreshed = false;
                List<Call> open;
                lock (_gate)
                {
                    _ws = ws;
                    open = [.. _calls.Values];
                }

                foreach (var call in open)
                {
                    await SendCallAsync(ws, call).ConfigureAwait(false);
                }

                var (end, heard) = await ReadAsync(ws, stop).ConfigureAwait(false);
                lock (_gate)
                {
                    _ws = null;
                }

                ws.Abort();
                ws.Dispose();
                if (end is not null)
                {
                    FailAll(end);
                    return;
                }

                if (heard)
                {
                    failures = 0;
                }
                else if (failures++ >= transport.MaxRetries)
                {
                    FailAll(new ConnectionException(transport.BaseUrl.Host, "the socket kept closing"));
                    return;
                }
                else
                {
                    await Task.Delay(Retry.Backoff(failures), stop).ConfigureAwait(false);
                }
            }
        }
        catch (OperationCanceledException) when (stop.IsCancellationRequested)
        {
            // Closed by the last stream, or by the client.
        }
        finally
        {
            lock (_gate)
            {
                _loop = null;
                _ws = null;
                _stop?.Dispose();
                _stop = null;
                if (_calls.Count > 0)
                {
                    // A stream registered while the loop was ending: start over for it.
                    _stop = new CancellationTokenSource();
                    _loop = Task.Run(() => RunAsync(_stop.Token), CancellationToken.None);
                }
            }
        }
    }

    /// <summary>Reads frames until the socket ends: the error that ends every call (a revoked key), or <see langword="null"/> to reconnect; and whether any data came.</summary>
    private async Task<(InOrbitException? End, bool Heard)> ReadAsync(ClientWebSocket ws, CancellationToken stop)
    {
        var buffer = new byte[16 * 1024];
        using var message = new MemoryStream();
        var heard = false;
        while (true)
        {
            message.SetLength(0);
            WebSocketReceiveResult result;
            try
            {
                do
                {
                    result = await ws.ReceiveAsync(buffer, stop).ConfigureAwait(false);
                    if (result.MessageType == WebSocketMessageType.Close)
                    {
                        return (null, heard);
                    }

                    if (message.Length + result.Count > Transport.MaxEvent)
                    {
                        return (null, heard);
                    }

                    message.Write(buffer, 0, result.Count);
                }
                while (!result.EndOfMessage);
            }
            catch (Exception e) when (e is WebSocketException or IOException)
            {
                return (null, heard);
            }

            var end = await DispatchAsync(message.ToArray(), stop).ConfigureAwait(false);
            if (end.Ends)
            {
                return (end.Error, heard);
            }

            heard |= end.Data;
        }
    }

    private readonly record struct Dispatched(bool Ends, InOrbitException? Error, bool Data);

    private async Task<Dispatched> DispatchAsync(byte[] frame, CancellationToken stop)
    {
        JsonElement root;
        try
        {
            using var doc = JsonDocument.Parse(frame);
            root = doc.RootElement.Clone();
        }
        catch (JsonException)
        {
            return default;
        }

        string? Str(string n) => root.ValueKind == JsonValueKind.Object && root.TryGetProperty(n, out var v) && v.ValueKind == JsonValueKind.String ? v.GetString() : null;
        var type = Str("type");
        var id = Str("id");
        var opened = new RawResponse(101, new Dictionary<string, IReadOnlyList<string>>(StringComparer.Ordinal), [], id ?? string.Empty, 1);
        if (id is null)
        {
            if (type == "error" && Str("code") == "unauthenticated")
            {
                return new Dispatched(true, Transport.StreamError(frame, opened), false);
            }

            // Any other socket-level error: the server is ending the socket; reconnect.
            return type == "error" ? new Dispatched(true, null, false) : default;
        }

        Call? call;
        lock (_gate)
        {
            _calls.TryGetValue(id, out call);
        }

        if (call is null || call.Ended)
        {
            return default;
        }

        switch (type)
        {
            case "data" when root.TryGetProperty("body", out var body):
                // Bounded: a full queue holds back reading the socket until the caller reads.
                await call.Items.Writer.WriteAsync(new Item(Encoding.UTF8.GetBytes(body.GetRawText()), null), stop).ConfigureAwait(false);
                return new Dispatched(false, null, true);
            case "end":
                call.Ended = true;
                call.Items.Writer.TryComplete();
                return default;
            case "error":
                call.Ended = true;
                await call.Items.Writer.WriteAsync(new Item(null, Transport.StreamError(frame, opened)), stop).ConfigureAwait(false);
                call.Items.Writer.TryComplete();
                return default;
            default:
                return default;
        }
    }

    private void FailAll(InOrbitException error)
    {
        List<Call> calls;
        lock (_gate)
        {
            // Ended calls leave the table at once, so nothing reconnects for them.
            calls = [.. _calls.Values];
            _calls.Clear();
        }

        foreach (var call in calls)
        {
            call.Ended = true;
            call.Items.Writer.TryWrite(new Item(null, error));
            call.Items.Writer.TryComplete();
        }
    }

    private async Task SendCallAsync(ClientWebSocket ws, Call call)
    {
        if (call.Ended)
        {
            return;
        }

        call.Sent = true;
        await SendAsync(ws, Frame(w =>
        {
            w.WriteString("type", "call");
            w.WriteString("id", call.Id);
            w.WriteString("method", call.Rpc);
            w.WritePropertyName("body");
            using var body = JsonDocument.Parse(call.Body);
            body.RootElement.WriteTo(w);
        })).ConfigureAwait(false);
    }

    private async Task SendAsync(ClientWebSocket ws, byte[] frame)
    {
        await _send.WaitAsync().ConfigureAwait(false);
        try
        {
            using var timer = new CancellationTokenSource(TimeSpan.FromSeconds(10));
            await ws.SendAsync(frame, WebSocketMessageType.Text, true, timer.Token).ConfigureAwait(false);
        }
        catch (Exception e) when (e is WebSocketException or OperationCanceledException or ObjectDisposedException)
        {
            // The socket is going; the read loop reconnects and issues the call again.
        }
        finally
        {
            _send.Release();
        }
    }

    private static byte[] Frame(Action<Utf8JsonWriter> write)
    {
        var buffer = new ArrayBufferWriter<byte>();
        using (var w = new Utf8JsonWriter(buffer))
        {
            w.WriteStartObject();
            write(w);
            w.WriteEndObject();
        }

        return buffer.WrittenSpan.ToArray();
    }

    private sealed record Item(byte[]? Data, InOrbitException? Error);

    private sealed class Call(string id, string rpc, byte[] body)
    {
        internal string Id { get; } = id;

        internal string Rpc { get; } = rpc;

        internal byte[] Body { get; } = body;

        internal Channel<Item> Items { get; } = Channel.CreateBounded<Item>(new BoundedChannelOptions(Queue) { SingleReader = true, SingleWriter = false });

        internal bool Sent { get; set; }

        internal bool Ended { get; set; }
    }
}
