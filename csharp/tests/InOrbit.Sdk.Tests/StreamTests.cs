using System;
using System.IO;
using System.Text;
using System.Threading;
using System.Threading.Tasks;
using Xunit;

namespace InOrbit.Sdk.Tests;

public class StreamTests
{
    private static SseReader Reader(string body, TimeSpan? idle = null) =>
        new(new MemoryStream(Encoding.UTF8.GetBytes(body)), idle ?? TimeSpan.FromSeconds(5), "api.test");

    [Fact]
    public async Task Data_lines_join_and_every_line_ending_counts()
    {
        var reader = Reader(": open\r\n\r\nid: 1\rdata: {\"a\":\ndata:  1}\n\nevent: error\r\ndata: x\r\n\r\ndata: tail without a blank line");
        var first = await reader.NextAsync(CancellationToken.None);
        Assert.Equal("message", first!.Name);
        Assert.Equal("{\"a\":\n 1}", Encoding.UTF8.GetString(first.Data));
        var second = await reader.NextAsync(CancellationToken.None);
        Assert.Equal("error", second!.Name);
        Assert.Equal("x", Encoding.UTF8.GetString(second.Data));
        Assert.Null(await reader.NextAsync(CancellationToken.None));
    }

    [Fact]
    public async Task An_event_over_one_mebibyte_is_refused()
    {
        var reader = Reader("data: " + new string('a', Transport.MaxEvent + 1) + "\n\n");
        var e = await Assert.ThrowsAsync<TooLargeException>(() => reader.NextAsync(CancellationToken.None));
        Assert.Equal("too_large", e.Kind);
    }

    [Fact]
    public async Task Silence_past_the_idle_timeout_is_a_timeout()
    {
        var reader = new SseReader(new Silent(), TimeSpan.FromMilliseconds(50), "api.test");
        var e = await Assert.ThrowsAsync<RequestTimeoutException>(() => reader.NextAsync(CancellationToken.None));
        Assert.Equal("timeout", e.Kind);
    }

    [Fact]
    public void An_error_envelope_takes_its_status_from_its_code()
    {
        var opened = new RawResponse(200, new System.Collections.Generic.Dictionary<string, System.Collections.Generic.IReadOnlyList<string>>(), [], "iohr-1", 1);
        var e = Transport.StreamError(Encoding.UTF8.GetBytes("{\"code\":\"forbidden\",\"error\":\"no\",\"details\":[]}"), opened);
        Assert.Equal(403, e.Status);
        Assert.Equal(Code.Forbidden, e.Code);
    }

    [Fact]
    public void A_socket_without_an_rpc_is_refused_and_a_bad_idle_timeout_too()
    {
        Assert.Throws<ConfigException>(() => new Client<PublicProfile>(new ClientOptions { Token = "t", StreamIdleTimeout = TimeSpan.Zero }));
        using var client = new Client<PublicProfile>(new ClientOptions { Token = "t", Streams = StreamTransport.Socket });
        Assert.Throws<ConfigException>(() => client.StreamAsync<object>(new Operation(Method.Get, "/v1/x/events"), TestContext.Current.CancellationToken));
    }

    /// <summary>A body that never sends a byte.</summary>
    private sealed class Silent : Stream
    {
        public override bool CanRead => true;

        public override bool CanSeek => false;

        public override bool CanWrite => false;

        public override long Length => throw new NotSupportedException();

        public override long Position { get => throw new NotSupportedException(); set => throw new NotSupportedException(); }

        public override void Flush()
        {
        }

        public override int Read(byte[] buffer, int offset, int count) => throw new NotSupportedException();

        public override async ValueTask<int> ReadAsync(Memory<byte> buffer, CancellationToken cancellationToken = default)
        {
            await Task.Delay(Timeout.Infinite, cancellationToken);
            return 0;
        }

        public override long Seek(long offset, SeekOrigin origin) => throw new NotSupportedException();

        public override void SetLength(long value) => throw new NotSupportedException();

        public override void Write(byte[] buffer, int offset, int count) => throw new NotSupportedException();
    }
}
