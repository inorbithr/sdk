package hr.inorbit.sdk;

import com.fasterxml.jackson.core.JsonProcessingException;
import hr.inorbit.sdk.auth.ClientCredentials;
import hr.inorbit.sdk.auth.StaticToken;
import hr.inorbit.sdk.auth.Token;
import hr.inorbit.sdk.auth.TokenProvider;
import hr.inorbit.sdk.codegen.Codegen;
import hr.inorbit.sdk.errors.ApiException;
import hr.inorbit.sdk.errors.ConfigException;
import hr.inorbit.sdk.errors.ConnectionException;
import hr.inorbit.sdk.errors.DecodeException;
import hr.inorbit.sdk.errors.InOrbitException;
import hr.inorbit.sdk.errors.TimeoutException;
import hr.inorbit.sdk.errors.TooLargeException;
import java.io.IOException;
import java.io.InputStream;
import java.net.URI;
import java.net.http.HttpClient;
import java.net.http.HttpRequest;
import java.net.http.HttpResponse;
import java.net.http.HttpTimeoutException;
import java.time.Duration;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Objects;
import java.util.Optional;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.Executor;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;

/**
 * A client for one credential: configuration, and the one request path every operation goes
 * through, with the token and retry rules of design.md sections 3 to 6. Safe to share between
 * threads; it holds no per-call state.
 *
 * <pre>{@code
 * Client client = Client.fromEnv(); // INORBIT_TOKEN, or INORBIT_KEY_ID + _KEY_SECRET + _SCOPES
 * Public api = new Public(client);
 * Me me = api.me().value();
 * }</pre>
 */
public final class Client {

    /** This runtime's version. */
    public static final String SDK_VERSION = "0.1.0"; // x-release-please-version

    /** Where the API is. */
    public static final String DEFAULT_BASE_URL = "https://api.inorbit.hr";

    /** The largest answer read, 16 MiB. */
    public static final int MAX_BODY = 16 * 1024 * 1024;

    private static final Duration CONNECT_TIMEOUT = Duration.ofSeconds(10);
    private static final byte[] EMPTY_OBJECT = {(byte) '{', (byte) '}'};

    private final URI base;
    private final TokenProvider provider;
    private final Duration timeout;
    private final int maxRetries;
    private final String userAgent;
    private final List<Hook> hooks;
    private final HttpClient http;
    private final Executor executor;
    private final StreamTransport streams;
    private final Duration streamIdleTimeout;
    private SocketHub hub;

    private Client(
            URI base,
            TokenProvider provider,
            Duration timeout,
            int maxRetries,
            String userAgent,
            List<Hook> hooks,
            HttpClient http,
            Executor executor,
            StreamTransport streams,
            Duration streamIdleTimeout) {
        this.base = base;
        this.provider = provider;
        this.timeout = timeout;
        this.maxRetries = maxRetries;
        this.userAgent = userAgent;
        this.hooks = hooks;
        this.http = http;
        this.executor = executor;
        this.streams = streams;
        this.streamIdleTimeout = streamIdleTimeout;
    }

    /**
     * A builder. Give {@link Builder#token}, or {@link Builder#key} with {@link Builder#scopes}, or
     * {@link Builder#tokenProvider}.
     *
     * @return the builder
     */
    public static Builder builder() {
        return new Builder();
    }

    /**
     * A client from the environment: {@code INORBIT_TOKEN}, or {@code INORBIT_KEY_ID}, {@code
     * INORBIT_KEY_SECRET} and {@code INORBIT_SCOPES}.
     *
     * @return the client
     * @throws ConfigException naming the variables to set when no credential is there
     */
    public static Client fromEnv() {
        return fromEnv(System.getenv(), "");
    }

    /**
     * A client for a named profile from the environment: {@code INORBIT_<PROFILE>_TOKEN}, or {@code
     * INORBIT_<PROFILE>_KEY_ID}, {@code _KEY_SECRET} and {@code _SCOPES}, and nothing else. {@code
     * INORBIT_BASE_URL} and {@code INORBIT_TOKEN_URL} apply to every profile.
     *
     * @param profile the profile as its variables carry it ({@code ACME_CI}); empty for none
     * @return the client
     * @throws ConfigException naming the variables to set when no credential is there
     */
    public static Client fromEnv(String profile) {
        return fromEnv(System.getenv(), profile);
    }

    static Client fromEnv(Map<String, String> env, String profile) {
        String prefix = profile == null || profile.isEmpty() ? "INORBIT_" : "INORBIT_" + profile + "_";
        Builder b = builder();
        value(env, prefix + "BASE_URL").or(() -> value(env, "INORBIT_BASE_URL")).ifPresent(b::baseUrl);
        value(env, prefix + "TOKEN_URL")
                .or(() -> value(env, "INORBIT_TOKEN_URL"))
                .ifPresent(b::tokenUrl);
        Optional<String> token = value(env, prefix + "TOKEN");
        if (token.isPresent()) {
            return b.token(token.get()).build();
        }
        Optional<String> keyId = value(env, prefix + "KEY_ID");
        Optional<String> keySecret = value(env, prefix + "KEY_SECRET");
        if (keyId.isPresent() && keySecret.isPresent()) {
            List<String> scopes = value(env, prefix + "SCOPES")
                    .map(s -> Arrays.stream(s.split("\\s+"))
                            .filter(x -> !x.isEmpty())
                            .toList())
                    .orElse(List.of());
            if (scopes.isEmpty()) {
                throw new ConfigException("no scopes: set " + prefix
                        + "SCOPES (space-separated, such as \"identity:read account:read\")");
            }
            return b.key(keyId.get(), keySecret.get()).scopes(scopes).build();
        }
        throw new ConfigException(
                "no credentials: set " + prefix + "TOKEN, or " + prefix + "KEY_ID and " + prefix + "KEY_SECRET");
    }

    private static Optional<String> value(Map<String, String> env, String name) {
        return Optional.ofNullable(env.get(name)).filter(v -> !v.isEmpty());
    }

    /**
     * The API's origin this client calls.
     *
     * @return the origin
     */
    public URI baseUrl() {
        return base;
    }

    /**
     * The same client with another limit for each attempt; the credential is shared.
     *
     * @param timeout how long one attempt may take
     * @return the client
     */
    public Client withTimeout(Duration timeout) {
        return new Client(
                base,
                provider,
                positive(timeout),
                maxRetries,
                userAgent,
                hooks,
                http,
                executor,
                streams,
                streamIdleTimeout);
    }

    /**
     * Calls {@code op} and reads its JSON answer as {@code type}.
     *
     * @param op the call
     * @param type the answer's type
     * @param <T> the answer's type
     * @return the typed answer and the raw one
     * @throws ApiException for an error answer
     * @throws InOrbitException for connection, timeout, token, size and decoding failures
     */
    public <T> Response<T> request(Operation op, Class<T> type) {
        RawResponse raw = send(op);
        try {
            byte[] body = raw.body();
            T value = Json.MAPPER.readValue(body.length == 0 ? EMPTY_OBJECT : body, type);
            return new Response<>(value, raw);
        } catch (IOException e) {
            throw new DecodeException(describe(e), raw, e);
        }
    }

    /**
     * {@link #request} on the client's executor.
     *
     * @param op the call
     * @param type the answer's type
     * @param <T> the answer's type
     * @return the typed answer and the raw one, when they arrive
     */
    public <T> CompletableFuture<Response<T>> requestAsync(Operation op, Class<T> type) {
        return CompletableFuture.supplyAsync(() -> request(op, type), executor);
    }

    /**
     * Calls {@code op} and hands back the answer as it came; a 2xx one only.
     *
     * @param op the call
     * @return the answer
     * @throws ApiException for an error answer
     * @throws InOrbitException for connection, timeout, token and size failures
     */
    public RawResponse send(Operation op) {
        URI url = url(op);
        byte[] body = body(op);
        String id = Retry.requestId();
        int retries = 0;
        boolean refreshed = false;
        for (int number = 1; ; number++) {
            Hook.Attempt attempt = new Hook.Attempt(op.name(), op.method(), op.path(), number, id);
            Outcome outcome;
            try {
                outcome = attempt(op, url, body, attempt);
            } catch (InOrbitException e) {
                failed(attempt, e);
                throw e;
            }
            if (outcome instanceof Outcome.Done done) {
                return done.raw();
            }
            if (outcome instanceof Outcome.Unauthorized && !refreshed) {
                provider.invalidate();
                refreshed = true;
                continue;
            }
            if (outcome instanceof Outcome.Again again && op.retrySafe() && retries < maxRetries) {
                Retry.sleep(again.delay().orElseGet(() -> Retry.backoff(again.retry())), base.getHost());
                retries++;
                continue;
            }
            InOrbitException error;
            if (outcome instanceof Outcome.Again a) {
                error = a.error() != null ? a.error() : ApiException.of(a.raw());
            } else {
                error = ApiException.of(((Outcome.Unauthorized) outcome).raw());
            }
            failed(attempt, error);
            throw error;
        }
    }

    /**
     * {@link #send} on the client's executor.
     *
     * @param op the call
     * @return the answer, when it arrives
     */
    public CompletableFuture<RawResponse> sendAsync(Operation op) {
        return CompletableFuture.supplyAsync(() -> send(op), executor);
    }

    /**
     * Opens a stream operation and reads its events as {@code type} (design.md section 7): over
     * server-sent events, or as a call on the client's one {@code /v1/ws} socket when the client
     * was built with {@link StreamTransport#SOCKET} and the operation names its RPC. The stream
     * opens on the first step of the iteration, with the retry and token rules of {@link #send}.
     *
     * @param op the call
     * @param type the model of one event
     * @param <T> the model of one event
     * @return the events; close it to stop early
     */
    public <T> EventStream<T> stream(Operation op, Class<T> type) {
        if (streams == StreamTransport.SOCKET && op.rpc() != null && !op.rpc().isEmpty()) {
            return new EventStream<>(hub().call(op, type));
        }
        return new EventStream<>(new SseSource<>(this, op, type, streamIdleTimeout));
    }

    /** The client's one socket, made when the first stream asks for it. */
    private synchronized SocketHub hub() {
        if (hub == null) {
            hub = new SocketHub(this);
        }
        return hub;
    }

    /** A stream's response as it opened, with the ids its errors name. */
    record Opened(
            HttpResponse<java.util.concurrent.Flow.Publisher<List<java.nio.ByteBuffer>>> response,
            String requestId,
            int attempts) {}

    /** The server-sent events request for {@code op}, retried like {@link #send}. */
    Opened openSse(Operation op) {
        URI url = url(op);
        String id = Retry.requestId();
        int retries = 0;
        boolean refreshed = false;
        for (int number = 1; ; number++) {
            Hook.Attempt attempt = new Hook.Attempt(op.name(), op.method(), op.path(), number, id);
            Token token = provider.token();
            HttpRequest req = HttpRequest.newBuilder(url)
                    .timeout(timeout)
                    .header("authorization", "Bearer " + token.access())
                    .header("accept", "text/event-stream")
                    .header("user-agent", userAgent)
                    .header("x-request-id", id)
                    .GET()
                    .build();
            for (Hook h : hooks) {
                h.onRequest(attempt);
            }
            HttpResponse<java.util.concurrent.Flow.Publisher<List<java.nio.ByteBuffer>>> resp;
            InOrbitException failure = null;
            RawResponse raw = null;
            try {
                resp = http.send(req, HttpResponse.BodyHandlers.ofPublisher());
                if (resp.statusCode() >= 200 && resp.statusCode() < 300) {
                    return new Opened(resp, id, number);
                }
                raw = new RawResponse(resp.statusCode(), resp.headers(), drain(resp.body()), id, number);
                for (Hook h : hooks) {
                    h.onResponse(attempt, raw);
                }
            } catch (HttpTimeoutException e) {
                failure = new TimeoutException(base.getHost(), timeout.toSeconds(), e);
            } catch (IOException e) {
                failure = new ConnectionException(base.getHost(), describe(e), e);
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
                throw new ConnectionException(base.getHost(), "interrupted", e);
            }
            if (raw != null && raw.status() == 401 && !refreshed) {
                provider.invalidate();
                refreshed = true;
                continue;
            }
            boolean again = failure != null || (raw != null && Retry.retryableStatus(raw.status()));
            if (again && retries < maxRetries) {
                Optional<Duration> wait = raw != null ? Retry.retryAfter(raw.headers()) : Optional.empty();
                Retry.sleep(wait.orElseGet(() -> Retry.backoff(attempt.number() - 1)), base.getHost());
                retries++;
                continue;
            }
            InOrbitException error = failure != null ? failure : ApiException.of(raw);
            failed(attempt, error);
            throw error;
        }
    }

    /** An error answer's body, at most {@link #MAX_BODY}. */
    private static byte[] drain(java.util.concurrent.Flow.Publisher<List<java.nio.ByteBuffer>> body)
            throws IOException, InterruptedException {
        java.util.concurrent.CompletableFuture<byte[]> bytes = new java.util.concurrent.CompletableFuture<>();
        HttpResponse.BodySubscriber<byte[]> sub = HttpResponse.BodySubscribers.ofByteArray();
        body.subscribe(sub);
        sub.getBody().whenComplete((b, e) -> {
            if (e != null) {
                bytes.completeExceptionally(e);
            } else {
                bytes.complete(b);
            }
        });
        try {
            byte[] b = bytes.get();
            if (b.length > MAX_BODY) {
                throw new TooLargeException();
            }
            return b;
        } catch (java.util.concurrent.ExecutionException e) {
            throw new IOException(describe(e.getCause()), e.getCause());
        }
    }

    // Package-private views the socket uses.

    TokenProvider provider() {
        return provider;
    }

    HttpClient http() {
        return http;
    }

    int maxRetries() {
        return maxRetries;
    }

    Duration streamIdleTimeout() {
        return streamIdleTimeout;
    }

    String userAgent() {
        return userAgent;
    }

    Duration timeout() {
        return timeout;
    }

    private void failed(Hook.Attempt attempt, InOrbitException e) {
        for (Hook h : hooks) {
            h.onError(attempt, e);
        }
    }

    private URI url(Operation op) {
        String p = op.path();
        if (!p.startsWith("/") || p.startsWith("//") || p.contains("?") || p.contains("#")) {
            throw new ConfigException("the path \"" + p + "\" is not usable: it starts with one / and has no query");
        }
        StringBuilder s = new StringBuilder(base.toString()).append(p);
        char sep = '?';
        for (Map.Entry<String, String> q : op.query()) {
            s.append(sep).append(Codegen.pathSegment(q.getKey())).append('=').append(Codegen.pathSegment(q.getValue()));
            sep = '&';
        }
        return URI.create(s.toString());
    }

    private static byte[] body(Operation op) {
        if (op.body() == null) {
            return null;
        }
        try {
            return Json.MAPPER.writeValueAsBytes(op.body());
        } catch (JsonProcessingException e) {
            throw new ConfigException("the request body cannot be written as JSON: " + e.getOriginalMessage());
        }
    }

    /** What one attempt came to. */
    private sealed interface Outcome {
        /** A 2xx answer. */
        record Done(RawResponse raw) implements Outcome {}

        /** A 401. */
        record Unauthorized(RawResponse raw) implements Outcome {}

        /** A retryable answer or failure; {@code raw} or {@code error} says which. */
        record Again(Optional<Duration> delay, int retry, RawResponse raw, InOrbitException error) implements Outcome {}
    }

    private Outcome attempt(Operation op, URI url, byte[] body, Hook.Attempt attempt) {
        Token token = provider.token();
        HttpRequest.Builder req = HttpRequest.newBuilder(url)
                .timeout(timeout)
                .header("authorization", "Bearer " + token.access())
                .header("accept", "application/json")
                .header("user-agent", userAgent)
                .header("x-request-id", attempt.requestId());
        if (body == null) {
            req.method(op.method().name(), HttpRequest.BodyPublishers.noBody());
        } else {
            req.header("content-type", "application/json")
                    .method(op.method().name(), HttpRequest.BodyPublishers.ofByteArray(body));
        }
        for (Hook h : hooks) {
            h.onRequest(attempt);
        }
        int retry = attempt.number() - 1;
        HttpResponse<InputStream> resp;
        byte[] bytes;
        try {
            resp = http.send(req.build(), HttpResponse.BodyHandlers.ofInputStream());
            long length = resp.headers().firstValueAsLong("content-length").orElse(0);
            if (length > MAX_BODY) {
                resp.body().close();
                throw new TooLargeException();
            }
            try (InputStream in = resp.body()) {
                bytes = in.readNBytes(MAX_BODY + 1);
            }
        } catch (HttpTimeoutException e) {
            return new Outcome.Again(
                    Optional.empty(), retry, null, new TimeoutException(base.getHost(), timeout.toSeconds(), e));
        } catch (IOException e) {
            return new Outcome.Again(
                    Optional.empty(), retry, null, new ConnectionException(base.getHost(), describe(e), e));
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            throw new ConnectionException(base.getHost(), "interrupted", e);
        }
        if (bytes.length > MAX_BODY) {
            throw new TooLargeException();
        }
        RawResponse raw =
                new RawResponse(resp.statusCode(), resp.headers(), bytes, attempt.requestId(), attempt.number());
        for (Hook h : hooks) {
            h.onResponse(attempt, raw);
        }
        int status = raw.status();
        if (status == 401) {
            return new Outcome.Unauthorized(raw);
        }
        if (Retry.retryableStatus(status)) {
            return new Outcome.Again(Retry.retryAfter(raw.headers()), retry, raw, null);
        }
        if (status >= 200 && status < 300) {
            return new Outcome.Done(raw);
        }
        throw ApiException.of(raw);
    }

    private static String describe(Throwable e) {
        String m = e.getMessage();
        return m == null || m.isEmpty() ? e.getClass().getSimpleName() : m;
    }

    private static Duration positive(Duration d) {
        if (d == null || d.isZero() || d.isNegative()) {
            throw new ConfigException("the timeout must be positive");
        }
        return d;
    }

    /** Never a credential. */
    @Override
    public String toString() {
        return "Client[" + base + ", " + provider + "]";
    }

    /** Builds a {@link Client}. */
    public static final class Builder {

        private String token;
        private String keyId;
        private String keySecret;
        private List<String> scopes = List.of();
        private TokenProvider tokenProvider;
        private String baseUrl = DEFAULT_BASE_URL;
        private String tokenUrl = ClientCredentials.DEFAULT_TOKEN_URL;
        private Duration timeout = Duration.ofSeconds(30);
        private int maxRetries = 2;
        private String userAgentSuffix = "";
        private final List<Hook> hooks = new ArrayList<>();
        private HttpClient httpClient;
        private Executor executor;
        private StreamTransport streams = StreamTransport.SSE;
        private Duration streamIdleTimeout = Duration.ofSeconds(45);

        private Builder() {}

        /**
         * How streams open: {@link StreamTransport#SSE} (default) or every stream over one
         * {@code /v1/ws} socket.
         *
         * @param streams the transport
         * @return this builder
         */
        public Builder streams(StreamTransport streams) {
            this.streams = Objects.requireNonNull(streams, "streams");
            return this;
        }

        /**
         * How long a stream may be silent (no event, comment or ping) before it fails with a
         * timeout, or on the socket reconnects (default 45 s).
         *
         * @param idle the limit
         * @return this builder
         */
        public Builder streamIdleTimeout(Duration idle) {
            this.streamIdleTimeout = idle;
            return this;
        }

        /**
         * An API token (from the console or {@code iohr token create}).
         *
         * @param token the token
         * @return this builder
         */
        public Builder token(String token) {
            this.token = token;
            return this;
        }

        /**
         * An API key, exchanged for short-lived tokens; give {@link #scopes} too.
         *
         * @param keyId the key's id
         * @param keySecret the key's secret
         * @return this builder
         */
        public Builder key(String keyId, String keySecret) {
            this.keyId = keyId;
            this.keySecret = keySecret;
            return this;
        }

        /**
         * The scopes to ask for with a key, a subset of the key's; no default.
         *
         * @param scopes the scopes
         * @return this builder
         */
        public Builder scopes(List<String> scopes) {
            this.scopes = List.copyOf(scopes);
            return this;
        }

        /**
         * Your own token source.
         *
         * @param provider the provider
         * @return this builder
         */
        public Builder tokenProvider(TokenProvider provider) {
            this.tokenProvider = provider;
            return this;
        }

        /**
         * The API's origin (default {@code https://api.inorbit.hr}; plain http only to this machine).
         *
         * @param baseUrl the origin
         * @return this builder
         */
        public Builder baseUrl(String baseUrl) {
            this.baseUrl = baseUrl;
            return this;
        }

        /**
         * The token endpoint for a key (default {@code https://auth.inorbit.hr/oauth2/token}).
         *
         * @param tokenUrl the endpoint
         * @return this builder
         */
        public Builder tokenUrl(String tokenUrl) {
            this.tokenUrl = tokenUrl;
            return this;
        }

        /**
         * How long each attempt may take (default 30 s).
         *
         * @param timeout the limit
         * @return this builder
         */
        public Builder timeout(Duration timeout) {
            this.timeout = timeout;
            return this;
        }

        /**
         * Retries after the first attempt (default 2; 0 disables).
         *
         * @param maxRetries the retries
         * @return this builder
         */
        public Builder maxRetries(int maxRetries) {
            this.maxRetries = maxRetries;
            return this;
        }

        /**
         * Appended to the user agent.
         *
         * @param suffix the suffix
         * @return this builder
         */
        public Builder userAgentSuffix(String suffix) {
            this.userAgentSuffix = Objects.requireNonNullElse(suffix, "");
            return this;
        }

        /**
         * Adds an observer of every attempt.
         *
         * @param hook the hook
         * @return this builder
         */
        public Builder hook(Hook hook) {
            this.hooks.add(Objects.requireNonNull(hook, "hook"));
            return this;
        }

        /**
         * The HTTP client to use (default one that follows no redirects).
         *
         * @param httpClient the client
         * @return this builder
         */
        public Builder httpClient(HttpClient httpClient) {
            this.httpClient = httpClient;
            return this;
        }

        /**
         * Where the {@code ...Async} calls run (default a shared pool of daemon threads).
         *
         * @param executor the executor
         * @return this builder
         */
        public Builder executor(Executor executor) {
            this.executor = executor;
            return this;
        }

        /**
         * The client.
         *
         * @return the client
         * @throws ConfigException when no credential is given, a key has no scopes, or a URL is not
         *     https (plain http only to this machine)
         */
        public Client build() {
            URI base = checkUrl("the base URL", baseUrl, true);
            URI tokenEndpoint = checkUrl("the token URL", tokenUrl, false);
            if (maxRetries < 0) {
                throw new ConfigException("max retries must not be negative");
            }
            HttpClient http = httpClient != null
                    ? httpClient
                    : HttpClient.newBuilder()
                            .followRedirects(HttpClient.Redirect.NEVER)
                            .connectTimeout(CONNECT_TIMEOUT)
                            .version(
                                    "http".equals(base.getScheme())
                                            ? HttpClient.Version.HTTP_1_1
                                            : HttpClient.Version.HTTP_2)
                            .build();
            TokenProvider provider;
            if (tokenProvider != null) {
                provider = tokenProvider;
            } else if (token != null && !token.isEmpty()) {
                provider = new StaticToken(token);
            } else if (keyId != null && keySecret != null) {
                if (scopes.isEmpty()) {
                    throw new ConfigException(
                            "no scopes: set INORBIT_SCOPES (space-separated, such as \"identity:read account:read\")");
                }
                provider = new ClientCredentials(keyId, keySecret, scopes, tokenEndpoint, http);
            } else {
                throw new ConfigException(
                        "no credentials: set INORBIT_TOKEN, or INORBIT_KEY_ID and INORBIT_KEY_SECRET");
            }
            String ua = "inorbithr-sdk-java/" + SDK_VERSION + " java/" + System.getProperty("java.version", "unknown")
                    + " "
                    + System.getProperty("os.name", "unknown")
                            .toLowerCase(Locale.ROOT)
                            .replace(' ', '-')
                    + "/" + System.getProperty("os.arch", "unknown")
                    + (userAgentSuffix.isEmpty() ? "" : " " + userAgentSuffix);
            return new Client(
                    base,
                    provider,
                    positive(timeout),
                    maxRetries,
                    ua,
                    List.copyOf(hooks),
                    http,
                    executor != null ? executor : Pool.EXECUTOR,
                    streams,
                    positive(streamIdleTimeout));
        }

        private static URI checkUrl(String what, String raw, boolean originOnly) {
            URI url;
            try {
                url = URI.create(Objects.requireNonNull(raw, what));
            } catch (IllegalArgumentException e) {
                throw new ConfigException(what + " is not usable: " + e.getMessage());
            }
            String host = url.getHost() == null ? "" : url.getHost().replaceAll("^\\[|\\]$", "");
            boolean loopback = host.equals("localhost") || host.equals("::1") || host.startsWith("127.");
            if (!("https".equals(url.getScheme()) || ("http".equals(url.getScheme()) && loopback)) || host.isEmpty()) {
                throw new ConfigException(what + " is not usable: it must use https (plain http only to this machine)");
            }
            if (url.getUserInfo() != null || url.getFragment() != null) {
                throw new ConfigException(what + " is not usable: it must not carry credentials or a fragment");
            }
            if (originOnly) {
                String path = url.getRawPath();
                if ((path != null && !path.isEmpty() && !path.equals("/")) || url.getRawQuery() != null) {
                    throw new ConfigException(
                            what + " is not usable: it is an origin only, such as https://api.inorbit.hr");
                }
                return URI.create(url.getScheme() + "://" + url.getRawAuthority());
            }
            return url;
        }
    }

    /** The default executor of the async calls: daemon threads, made when needed. */
    private static final class Pool {
        static final ExecutorService EXECUTOR = Executors.newCachedThreadPool(r -> {
            Thread t = new Thread(r, "inorbit-sdk");
            t.setDaemon(true);
            return t;
        });

        private Pool() {}
    }
}
