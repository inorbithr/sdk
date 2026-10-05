package hr.inorbit.sdk;

import com.fasterxml.jackson.databind.JsonNode;
import hr.inorbit.sdk.errors.ConnectionException;
import hr.inorbit.sdk.errors.DecodeException;
import hr.inorbit.sdk.errors.TimeoutException;
import hr.inorbit.sdk.errors.TooLargeException;
import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.nio.ByteBuffer;
import java.nio.charset.StandardCharsets;
import java.time.Duration;
import java.util.List;
import java.util.concurrent.BlockingQueue;
import java.util.concurrent.Flow;
import java.util.concurrent.LinkedBlockingQueue;
import java.util.concurrent.TimeUnit;

/**
 * One stream over server-sent events, parsed by the WHATWG event-stream rules (design.md
 * section 7): lines end with LF, CRLF or CR; a comment is a keep-alive; {@code data} lines join
 * with a newline; a blank line dispatches; an {@code error} event is the envelope and ends the
 * stream. Nothing at all for the idle timeout fails with a timeout. The body is read one chunk
 * at a time (a {@link Flow} subscriber requesting the next only when this one is used), so what
 * waits in memory is bounded by the caller's pace.
 */
final class SseSource<T> implements EventStream.Source<T> {

    private final Client client;
    private final Operation op;
    private final Class<T> type;
    private final Duration idle;
    private final String host;

    private final Body subscriber = new Body();
    private final java.util.ArrayDeque<ByteBuffer> pending = new java.util.ArrayDeque<>();
    private ByteBuffer current;
    private boolean opened;
    private final byte[] chunk = new byte[8192];
    private int pos;
    private int len;
    private boolean skipLf;
    private final ByteArrayOutputStream line = new ByteArrayOutputStream();
    private final StringBuilder data = new StringBuilder();
    private boolean hasData;
    private String event = "";
    private volatile boolean closed;
    private String requestId = "";
    private int attempts;
    private Runnable onEnd = () -> {};

    SseSource(Client client, Operation op, Class<T> type, Duration idle) {
        this.client = client;
        this.op = op;
        this.type = type;
        this.idle = idle;
        this.host = client.baseUrl().getHost();
    }

    @Override
    public T next() {
        if (closed) {
            return null;
        }
        if (!opened) {
            opened = true;
            Client.Opened o = client.openSse(op);
            requestId = o.requestId();
            attempts = o.attempts();
            onEnd = o.onEnd();
            o.body().subscribe(subscriber);
        }
        while (true) {
            String l = readLine();
            if (l == null) {
                // A partly read event at the end of the body is dropped, as the rules say.
                return null;
            }
            if (l.isEmpty()) {
                T item = dispatch();
                if (item != null) {
                    return item;
                }
                continue;
            }
            if (l.charAt(0) == ':') {
                continue;
            }
            int colon = l.indexOf(':');
            String field = colon < 0 ? l : l.substring(0, colon);
            String value = colon < 0 ? "" : l.substring(colon + 1);
            if (value.startsWith(" ")) {
                value = value.substring(1);
            }
            switch (field) {
                case "data" -> {
                    if (hasData) {
                        data.append('\n');
                    }
                    data.append(value);
                    hasData = true;
                    if (data.length() > Streams.MAX_EVENT) {
                        throw new TooLargeException();
                    }
                }
                case "event" -> event = value;
                default -> {
                    // id, retry and unknown fields are not used.
                }
            }
        }
    }

    /** The event a blank line ends, or {@code null} when it carried no data. */
    private T dispatch() {
        String name = event;
        event = "";
        if (!hasData) {
            return null;
        }
        String text = data.toString();
        data.setLength(0);
        hasData = false;
        if (name.equals("error")) {
            JsonNode envelope;
            try {
                envelope = Json.MAPPER.readTree(text);
            } catch (IOException e) {
                throw new DecodeException("the stream's error event is not JSON", null, e);
            }
            throw Streams.error(envelope, requestId, attempts);
        }
        try {
            return Json.MAPPER.readValue(text, type);
        } catch (IOException e) {
            throw new DecodeException("an event is not a " + type.getSimpleName() + ": " + e.getMessage(), null, e);
        }
    }

    /** One line without its end; {@code null} at the end of the body. */
    private String readLine() {
        line.reset();
        while (true) {
            if (pos == len) {
                if (!fill()) {
                    if (line.size() > 0) {
                        return line.toString(StandardCharsets.UTF_8);
                    }
                    return null;
                }
            }
            byte b = chunk[pos++];
            if (skipLf) {
                skipLf = false;
                if (b == '\n') {
                    continue;
                }
            }
            if (b == '\n') {
                return line.toString(StandardCharsets.UTF_8);
            }
            if (b == '\r') {
                skipLf = true;
                return line.toString(StandardCharsets.UTF_8);
            }
            line.write(b);
            if (line.size() > Streams.MAX_EVENT) {
                throw new TooLargeException();
            }
        }
    }

    /** Reads more of the body, waiting at most the idle timeout; false at its end. */
    private boolean fill() {
        while (true) {
            if (current != null && current.hasRemaining()) {
                len = Math.min(current.remaining(), chunk.length);
                current.get(chunk, 0, len);
                pos = 0;
                return true;
            }
            if (!pending.isEmpty()) {
                current = pending.removeFirst();
                continue;
            }
            Object got;
            try {
                got = subscriber.items.poll(idle.toNanos(), TimeUnit.NANOSECONDS);
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
                throw new ConnectionException(host, "interrupted", e);
            }
            if (got == null) {
                throw new TimeoutException(host, idle.toSeconds(), null);
            }
            if (got == Body.END) {
                return false;
            }
            if (got instanceof Throwable t) {
                if (closed) {
                    return false;
                }
                throw new ConnectionException(host, "the stream broke: " + t.getMessage(), t);
            }
            @SuppressWarnings("unchecked")
            List<ByteBuffer> buffers = (List<ByteBuffer>) got;
            pending.addAll(buffers);
            subscriber.more();
        }
    }

    /** The body's chunks, one request at a time: what waits here is bounded. */
    static final class Body implements Flow.Subscriber<List<ByteBuffer>> {
        static final Object END = new Object();
        final BlockingQueue<Object> items = new LinkedBlockingQueue<>();
        private volatile Flow.Subscription subscription;

        @Override
        public void onSubscribe(Flow.Subscription s) {
            subscription = s;
            s.request(1);
        }

        @Override
        public void onNext(List<ByteBuffer> item) {
            items.add(item);
        }

        @Override
        public void onError(Throwable t) {
            items.add(t);
        }

        @Override
        public void onComplete() {
            items.add(END);
        }

        void more() {
            Flow.Subscription s = subscription;
            if (s != null) {
                s.request(1);
            }
        }

        void cancel() {
            Flow.Subscription s = subscription;
            if (s != null) {
                s.cancel();
            }
        }
    }

    @Override
    public void close() {
        if (closed) {
            return;
        }
        closed = true;
        subscriber.cancel();
        onEnd.run();
    }
}
