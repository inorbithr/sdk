package hr.inorbit.sdk;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.node.ObjectNode;
import hr.inorbit.sdk.auth.Token;
import hr.inorbit.sdk.errors.ApiException;
import hr.inorbit.sdk.errors.ConnectionException;
import hr.inorbit.sdk.errors.DecodeException;
import hr.inorbit.sdk.errors.InOrbitException;
import hr.inorbit.sdk.errors.TimeoutException;
import hr.inorbit.sdk.errors.TooLargeException;
import java.io.IOException;
import java.net.URI;
import java.net.http.HttpHeaders;
import java.net.http.WebSocket;
import java.net.http.WebSocketHandshakeException;
import java.nio.ByteBuffer;
import java.time.Duration;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Optional;
import java.util.concurrent.ArrayBlockingQueue;
import java.util.concurrent.BlockingQueue;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.CompletionStage;
import java.util.concurrent.ExecutionException;
import java.util.concurrent.TimeUnit;

/**
 * The client's one {@code /v1/ws} socket and the stream calls on it (design.md section 7).
 *
 * <p>The socket opens with the first call and closes when the last one ends. A call is one
 * {@code call} frame; its {@code data} frames are its items, {@code end} or an {@code error}
 * frame for its id ends it, and a caller that stops sends {@code cancel}. An {@code error}
 * frame without an id concerns the socket: {@code unauthenticated} (a revoked key) fails every
 * call and nothing reconnects; any other code, a close, a broken connection or silence past the
 * idle timeout reconnects with a token from the provider and issues every call that had not
 * ended again, within the retry budget.
 */
final class SocketHub {

    private static final Object END = new Object();
    private static final HttpHeaders NO_HEADERS = HttpHeaders.of(Map.of(), (a, b) -> true);

    private final Client client;
    private final Object lock = new Object();
    private final Map<String, Call<?>> calls = new LinkedHashMap<>();
    private Conn conn;
    private long nextId;
    private boolean reconnecting;

    SocketHub(Client client) {
        this.client = client;
    }

    <T> EventStream.Source<T> call(Operation op, Class<T> type) {
        return new Call<>(op, type);
    }

    private String host() {
        return client.baseUrl().getHost();
    }

    private URI wsUri() {
        URI base = client.baseUrl();
        String scheme = "https".equals(base.getScheme()) ? "wss" : "ws";
        return URI.create(scheme + "://" + base.getRawAuthority() + "/v1/ws");
    }

    // --- calls ---------------------------------------------------------------------------

    /** One stream: a call on the socket and the items waiting for its reader (bounded). */
    private final class Call<T> implements EventStream.Source<T> {
        private final Operation op;
        private final Class<T> type;
        private final BlockingQueue<Object> items = new ArrayBlockingQueue<>(Streams.QUEUE);
        private volatile boolean closed;
        private boolean started;
        String id;

        Call(Operation op, Class<T> type) {
            this.op = op;
            this.type = type;
        }

        @Override
        public T next() {
            if (closed) {
                return null;
            }
            if (!started) {
                started = true;
                start(this);
            }
            Object item;
            try {
                item = items.take();
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
                throw new ConnectionException(host(), "interrupted", e);
            }
            if (item == END) {
                return null;
            }
            if (item instanceof InOrbitException e) {
                throw e;
            }
            try {
                return Json.MAPPER.treeToValue((JsonNode) item, type);
            } catch (IOException e) {
                throw new DecodeException("an event is not a " + type.getSimpleName() + ": " + e.getMessage(), null, e);
            }
        }

        /** Hands an item to the reader, waiting while its queue is full; dropped once closed. */
        void put(Object item) {
            try {
                while (!closed && !items.offer(item, 100, TimeUnit.MILLISECONDS)) {
                    // The reader is slow: this holds the socket's reading back, which is the bound.
                }
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
            }
        }

        @Override
        public void close() {
            if (closed) {
                return;
            }
            closed = true;
            if (started) {
                cancel(this);
            }
            items.clear();
        }

        ObjectNode frame() {
            ObjectNode f = Json.MAPPER.createObjectNode();
            f.put("type", "call");
            f.put("id", id);
            f.put("method", op.rpc());
            ObjectNode body = Json.MAPPER.createObjectNode();
            for (Map.Entry<String, Object> e : op.fields().entrySet()) {
                ObjectNode at = body;
                String[] parts = e.getKey().split("\\.");
                for (int i = 0; i < parts.length - 1; i++) {
                    JsonNode next = at.get(parts[i]);
                    at = next instanceof ObjectNode o ? o : at.putObject(parts[i]);
                }
                at.set(parts[parts.length - 1], Json.MAPPER.valueToTree(e.getValue()));
            }
            f.set("body", body);
            return f;
        }
    }

    private void start(Call<?> call) {
        synchronized (lock) {
            if (conn == null && !reconnecting) {
                conn = connect();
            }
            call.id = String.valueOf(++nextId);
            calls.put(call.id, call);
            if (conn != null && !reconnecting) {
                conn.send(call.frame().toString());
            }
        }
    }

    private void cancel(Call<?> call) {
        synchronized (lock) {
            if (calls.remove(call.id) == null) {
                return;
            }
            if (conn != null) {
                ObjectNode f = Json.MAPPER.createObjectNode();
                f.put("type", "cancel");
                f.put("id", call.id);
                conn.send(f.toString());
            }
            closeIfIdle();
        }
    }

    /** No call left: the socket closes (lock held). */
    private void closeIfIdle() {
        if (calls.isEmpty() && conn != null && !reconnecting) {
            conn.close();
            conn = null;
        }
    }

    // --- the connection ------------------------------------------------------------------

    /** Opens a socket, retried like a GET: one fresh token after 401, retryable failures within the budget. */
    private Conn connect() {
        boolean refreshed = false;
        int retries = 0;
        while (true) {
            InOrbitException failure;
            boolean again;
            try {
                Token token = client.provider().token();
                Conn c = new Conn();
                WebSocket ws = client.http()
                        .newWebSocketBuilder()
                        .connectTimeout(client.timeout())
                        .header("authorization", "Bearer " + token.access())
                        .header("user-agent", client.userAgent())
                        .header("x-request-id", Retry.requestId())
                        .buildAsync(wsUri(), c)
                        .get(client.timeout().toMillis() + 1000, TimeUnit.MILLISECONDS);
                c.ws = ws;
                c.watch();
                return c;
            } catch (ExecutionException e) {
                Throwable cause = e.getCause();
                if (cause instanceof WebSocketHandshakeException h) {
                    int status = h.getResponse().statusCode();
                    HttpHeaders headers = h.getResponse().headers();
                    byte[] body = h.getResponse().body() instanceof byte[] b
                            ? b
                            : h.getResponse().body() instanceof String s
                                    ? s.getBytes(java.nio.charset.StandardCharsets.UTF_8)
                                    : new byte[0];
                    RawResponse raw =
                            new RawResponse(status, headers == null ? NO_HEADERS : headers, body, "", retries + 1);
                    if (status == 401 && !refreshed) {
                        client.provider().invalidate();
                        refreshed = true;
                        continue;
                    }
                    failure = ApiException.of(raw);
                    again = Retry.retryableStatus(status);
                    if (again && retries < client.maxRetries()) {
                        Optional<Duration> wait = Retry.retryAfter(raw.headers());
                        Retry.sleep(wait.orElseGet(() -> Retry.backoff(0)), host());
                        retries++;
                        continue;
                    }
                    throw failure;
                }
                failure = new ConnectionException(host(), describe(cause), cause);
                again = true;
            } catch (java.util.concurrent.TimeoutException e) {
                failure = new TimeoutException(host(), client.timeout().toSeconds(), e);
                again = true;
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
                throw new ConnectionException(host(), "interrupted", e);
            }
            if (again && retries < client.maxRetries()) {
                Retry.sleep(Retry.backoff(retries), host());
                retries++;
                continue;
            }
            throw failure;
        }
    }

    /**
     * The connection is gone ({@code why}: an id-less error, a close, an error, silence). When
     * calls are in flight, a new socket takes them over, off the listener's thread.
     */
    private void lost(Conn c, InOrbitException why) {
        synchronized (lock) {
            if (c != conn) {
                return;
            }
            conn = null;
            c.abort();
            if (calls.isEmpty()) {
                return;
            }
            reconnecting = true;
        }
        Thread t = new Thread(this::reconnect, "inorbit-sdk-socket");
        t.setDaemon(true);
        t.start();
    }

    /**
     * Opens a new socket for the calls in flight and issues them again; when no socket can be
     * had within the retry budget, they end with that failure.
     */
    private void reconnect() {
        Conn fresh;
        try {
            fresh = connect();
        } catch (InOrbitException e) {
            List<Call<?>> all;
            synchronized (lock) {
                reconnecting = false;
                all = new ArrayList<>(calls.values());
                calls.clear();
            }
            for (Call<?> call : all) {
                call.put(e);
            }
            return;
        }
        synchronized (lock) {
            reconnecting = false;
            conn = fresh;
            Map<String, Call<?>> again = new LinkedHashMap<>(calls);
            calls.clear();
            for (Call<?> call : again.values()) {
                call.id = String.valueOf(++nextId);
                calls.put(call.id, call);
                fresh.send(call.frame().toString());
            }
            closeIfIdle();
        }
    }

    /** Ends every call with {@code error}; nothing reconnects (a revoked key). */
    private void fatal(Conn c, InOrbitException error) {
        List<Call<?>> all;
        synchronized (lock) {
            if (c != conn) {
                return;
            }
            conn = null;
            c.abort();
            all = new ArrayList<>(calls.values());
            calls.clear();
        }
        for (Call<?> call : all) {
            call.put(error);
        }
    }

    private void frame(Conn c, JsonNode f) {
        String type = f.path("type").asText("");
        String id = f.has("id") && !f.path("id").isNull() ? f.path("id").asText() : null;
        if (type.equals("error") && id == null) {
            if (f.path("code").asText("").equals("unauthenticated")) {
                fatal(c, Streams.error(f, "", 1));
            } else {
                lost(c, Streams.error(f, "", 1));
            }
            return;
        }
        Call<?> call;
        Object item;
        synchronized (lock) {
            if (c != conn || id == null) {
                return;
            }
            switch (type) {
                case "data" -> {
                    call = calls.get(id);
                    item = f.path("body");
                }
                case "end" -> {
                    call = calls.remove(id);
                    item = END;
                    closeIfIdle();
                }
                case "error" -> {
                    call = calls.remove(id);
                    item = Streams.error(f, "", 1);
                    closeIfIdle();
                }
                default -> {
                    return;
                }
            }
        }
        if (call != null) {
            call.put(item);
        }
    }

    private static String describe(Throwable e) {
        String m = e == null ? null : e.getMessage();
        return m == null || m.isEmpty() ? (e == null ? "unknown" : e.getClass().getSimpleName()) : m;
    }

    /** One WebSocket and its listener; frames are sent one after another. */
    private final class Conn implements WebSocket.Listener {
        volatile WebSocket ws;
        private final StringBuilder text = new StringBuilder();
        private CompletableFuture<?> sending = CompletableFuture.completedFuture(null);
        private volatile long heard = System.nanoTime();
        private volatile boolean done;

        synchronized void send(String frame) {
            WebSocket w = ws;
            sending = sending.handle((v, e) -> null).thenCompose(v -> w.sendText(frame, true));
        }

        synchronized void close() {
            done = true;
            WebSocket w = ws;
            sending = sending.handle((v, e) -> null).thenCompose(v -> w.sendClose(WebSocket.NORMAL_CLOSURE, ""));
        }

        void abort() {
            done = true;
            WebSocket w = ws;
            if (w != null) {
                w.abort();
            }
        }

        /** The idle clock: silence past the timeout is a lost connection. */
        void watch() {
            Duration idle = client.streamIdleTimeout();
            Streams.TIMER.schedule(this::check, idle.toNanos(), TimeUnit.NANOSECONDS);
        }

        private void check() {
            if (done) {
                return;
            }
            long idle = client.streamIdleTimeout().toNanos();
            long silent = System.nanoTime() - heard;
            if (silent >= idle) {
                lost(
                        this,
                        new TimeoutException(host(), client.streamIdleTimeout().toSeconds(), null));
            } else {
                Streams.TIMER.schedule(this::check, idle - silent, TimeUnit.NANOSECONDS);
            }
        }

        @Override
        public void onOpen(WebSocket webSocket) {
            webSocket.request(1);
        }

        @Override
        public CompletionStage<?> onText(WebSocket webSocket, CharSequence data, boolean last) {
            heard = System.nanoTime();
            text.append(data);
            if (text.length() > Streams.MAX_EVENT) {
                text.setLength(0);
                fatal(this, new TooLargeException());
                return null;
            }
            if (last) {
                String whole = text.toString();
                text.setLength(0);
                try {
                    frame(this, Json.MAPPER.readTree(whole));
                } catch (IOException e) {
                    lost(this, new DecodeException("a socket frame is not JSON", null, e));
                }
            }
            webSocket.request(1);
            return null;
        }

        @Override
        public CompletionStage<?> onBinary(WebSocket webSocket, ByteBuffer data, boolean last) {
            heard = System.nanoTime();
            webSocket.request(1);
            return null;
        }

        @Override
        public CompletionStage<?> onPing(WebSocket webSocket, ByteBuffer message) {
            heard = System.nanoTime();
            webSocket.request(1);
            return null;
        }

        @Override
        public CompletionStage<?> onPong(WebSocket webSocket, ByteBuffer message) {
            heard = System.nanoTime();
            webSocket.request(1);
            return null;
        }

        @Override
        public CompletionStage<?> onClose(WebSocket webSocket, int statusCode, String reason) {
            done = true;
            lost(this, new ConnectionException(host(), "the socket closed (" + statusCode + ")", null));
            return null;
        }

        @Override
        public void onError(WebSocket webSocket, Throwable error) {
            done = true;
            lost(this, new ConnectionException(host(), describe(error), error));
        }
    }
}
