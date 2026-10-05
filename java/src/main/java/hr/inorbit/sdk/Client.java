package hr.inorbit.sdk;

import com.fasterxml.jackson.core.JsonProcessingException;
import hr.inorbit.sdk.auth.CliToken;
import hr.inorbit.sdk.auth.ClientCredentials;
import hr.inorbit.sdk.auth.Observable;
import hr.inorbit.sdk.auth.StaticToken;
import hr.inorbit.sdk.auth.TokenFile;
import hr.inorbit.sdk.auth.TokenProvider;
import hr.inorbit.sdk.codegen.Codegen;
import hr.inorbit.sdk.errors.ApiException;
import hr.inorbit.sdk.errors.ConfigException;
import hr.inorbit.sdk.errors.DecodeException;
import hr.inorbit.sdk.errors.InOrbitException;
import hr.inorbit.sdk.middleware.Headers;
import hr.inorbit.sdk.middleware.Middleware;
import hr.inorbit.sdk.middleware.Pipeline;
import hr.inorbit.sdk.middleware.Request;
import java.io.IOException;
import java.net.URI;
import java.net.http.HttpClient;
import java.nio.ByteBuffer;
import java.nio.file.Path;
import java.security.KeyStore;
import java.time.Duration;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Objects;
import java.util.Optional;
import java.util.Set;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.Executor;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.Flow;
import java.util.function.Consumer;
import java.util.function.UnaryOperator;

/**
 * A client for one credential: configuration, and the one request path every operation goes
 * through, a pipeline of named middlewares (docs/config.md section 7). Safe to share between
 * threads.
 *
 * <pre>{@code
 * Client client = Client.load(); // code, INORBIT_*, the iohr config file, then the iohr login
 * Public api = new Public(client);
 * Me me = api.me().value();
 * }</pre>
 */
public final class Client {

    /** This runtime's version. */
    public static final String SDK_VERSION = "0.2.1"; // x-release-please-version

    /** Where the API is. */
    public static final String DEFAULT_BASE_URL = "https://api.inorbit.hr";

    /** The largest answer read, 16 MiB. */
    public static final int MAX_BODY = 16 * 1024 * 1024;

    private static final Duration CONNECT_TIMEOUT = Duration.ofSeconds(10);
    private static final byte[] EMPTY_OBJECT = {(byte) '{', (byte) '}'};

    /** What every view of one client shares. */
    private static final class Core {
        final Engine engine;
        final TokenProvider provider;
        final List<Middleware> middlewares;
        final ResolvedConfig resolved;
        final Executor executor;
        final StreamTransport streams;
        final boolean logsCalls;
        final boolean hooksCalls;
        SocketHub hub;

        Core(
                Engine engine,
                TokenProvider provider,
                List<Middleware> middlewares,
                ResolvedConfig resolved,
                Executor executor,
                StreamTransport streams,
                boolean logsCalls,
                boolean hooksCalls) {
            this.engine = engine;
            this.provider = provider;
            this.middlewares = middlewares;
            this.resolved = resolved;
            this.executor = executor;
            this.streams = streams;
            this.logsCalls = logsCalls;
            this.hooksCalls = hooksCalls;
        }
    }

    private final Core core;
    private final CallOptions options;

    private Client(Core core, CallOptions options) {
        this.core = core;
        this.options = options;
    }

    /**
     * A builder. Give {@link Builder#token}, or {@link Builder#key} with {@link Builder#scopes}, or
     * {@link Builder#tokenProvider}, then {@link Builder#build}; or end with {@link Builder#load}
     * to read the environment and the config file too.
     *
     * @return the builder
     */
    public static Builder builder() {
        return new Builder();
    }

    /**
     * A client from the environment, the {@code iohr} config file and login, and the defaults
     * (docs/config.md): each setting takes the first source that sets it, {@code INORBIT_*}, then
     * the config file's profile and {@code [sdk]} tables, then the default; credentials come from
     * the first source of the chain that has any. Nothing is contacted until the first call.
     *
     * @return the client
     * @throws ConfigException every problem found, each with its setting and source
     */
    public static Client load() {
        return builder().load();
    }

    /**
     * A client from the environment: {@code INORBIT_TOKEN}, or {@code INORBIT_KEY_ID}, {@code
     * INORBIT_KEY_SECRET} and {@code INORBIT_SCOPES}. {@link #load()} reads more, and is the
     * recommended way.
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
     * @param profile the profile ({@code acme-ci} or {@code ACME_CI}); empty for none
     * @return the client
     * @throws ConfigException naming the variables to set when no credential is there
     */
    public static Client fromEnv(String profile) {
        return fromEnv(System.getenv(), profile);
    }

    static Client fromEnv(Map<String, String> env, String profile) {
        String prefix = profile == null || profile.isEmpty() ? "INORBIT_" : "INORBIT_" + Config.envName(profile) + "_";
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
        return core.engine.base;
    }

    /**
     * The effective configuration and where each value came from ({@code describe()}).
     *
     * @return the configuration
     */
    public ResolvedConfig config() {
        return core.resolved;
    }

    /**
     * The latest rate-limit snapshot any call of this client saw (docs/config.md section 7.8).
     *
     * @return the snapshot, if any answer carried one
     */
    public Optional<RateLimit> rateLimit() {
        return Optional.ofNullable(core.engine.latest());
    }

    /**
     * The credential this client sends.
     *
     * @return the token provider
     */
    public TokenProvider credential() {
        return core.provider;
    }

    /**
     * The same client with another limit for each attempt, which also caps each call's total;
     * the credential and everything else are shared.
     *
     * @param timeout how long one attempt may take
     * @return the client
     */
    public Client withTimeout(Duration timeout) {
        CallOptions.Builder b = CallOptions.builder().timeout(positive(timeout));
        if (options.idempotencyKey() != null) {
            b.idempotencyKey(options.idempotencyKey());
        }
        if (options.traceparent() != null) {
            b.traceparent(options.traceparent());
        }
        return new Client(core, b.build());
    }

    /**
     * The same client, its calls made with {@code options}: a per-call timeout, the {@code
     * Idempotency-Key} to send, the caller's {@code traceparent}. Everything else is shared.
     *
     * <pre>{@code
     * Public api = new Public(client.withOptions(CallOptions.builder().idempotencyKey("order-42").build()));
     * }</pre>
     *
     * @param options the options
     * @return the client
     */
    public Client withOptions(CallOptions options) {
        return new Client(core, Objects.requireNonNull(options, "options"));
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
            throw new DecodeException(Engine.describe(e), raw, e);
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
        return CompletableFuture.supplyAsync(() -> request(op, type), core.executor);
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
        Request req = prepare(op, false);
        hr.inorbit.sdk.middleware.Response resp;
        try {
            resp = run(req);
        } catch (InOrbitException e) {
            throw failed(req, e);
        }
        return finish(req, resp);
    }

    /**
     * {@link #send} on the client's executor.
     *
     * @param op the call
     * @return the answer, when it arrives
     */
    public CompletableFuture<RawResponse> sendAsync(Operation op) {
        return CompletableFuture.supplyAsync(() -> send(op), core.executor);
    }

    /**
     * Opens a stream operation and reads its events as {@code type} (design.md section 7): over
     * server-sent events through the pipeline, or as a call on the client's one {@code /v1/ws}
     * socket when the client was built with {@link StreamTransport#SOCKET} and the operation names
     * its RPC. The stream opens on the first step of the iteration.
     *
     * @param op the call
     * @param type the model of one event
     * @param <T> the model of one event
     * @return the events; close it to stop early
     */
    public <T> EventStream<T> stream(Operation op, Class<T> type) {
        if (core.streams == StreamTransport.SOCKET
                && op.rpc() != null
                && !op.rpc().isEmpty()) {
            return new EventStream<>(hub().call(op, type));
        }
        return new EventStream<>(new SseSource<>(this, op, type, core.engine.idle));
    }

    /** The client's one socket, made when the first stream asks for it. */
    private SocketHub hub() {
        synchronized (core) {
            if (core.hub == null) {
                core.hub = new SocketHub(this);
            }
            return core.hub;
        }
    }

    /** A stream's body as it opened, with the ids its errors name, and what to run at its end. */
    record Opened(Flow.Publisher<List<ByteBuffer>> body, String requestId, int attempts, Runnable onEnd) {}

    /** The server-sent events request for {@code op}, through the pipeline. */
    Opened openSse(Operation op) {
        Request req = prepare(op, true);
        hr.inorbit.sdk.middleware.Response resp;
        try {
            resp = run(req);
        } catch (InOrbitException e) {
            throw failed(req, e);
        }
        RawResponse raw = finish(req, resp);
        Call c = Call.of(req);
        Runnable end = () -> {
            List<Runnable> todo;
            synchronized (c.onClose) {
                todo = new ArrayList<>(c.onClose);
                c.onClose.clear();
            }
            todo.forEach(Runnable::run);
        };
        return new Opened(resp.stream(), raw.requestId(), raw.attempts(), end);
    }

    // Package-private views the socket uses.

    TokenProvider provider() {
        return core.provider;
    }

    HttpClient http() {
        return core.engine.http;
    }

    Engine engine() {
        return core.engine;
    }

    int maxRetries() {
        return core.engine.maxRetries;
    }

    Duration streamIdleTimeout() {
        return core.engine.idle;
    }

    String userAgent() {
        return core.engine.userAgent;
    }

    Duration timeout() {
        return options.timeout() != null ? options.timeout() : core.engine.timeout;
    }

    // --- one call --------------------------------------------------------------------------

    private hr.inorbit.sdk.middleware.Response run(Request req) {
        return new Engine.Link(core.middlewares, 0, core.engine::send, req.info()).proceed(req);
    }

    private URI url(Operation op) {
        String p = op.path();
        if (!p.startsWith("/") || p.startsWith("//") || p.contains("?") || p.contains("#")) {
            throw new ConfigException("the path \"" + p + "\" is not usable: it starts with one / and has no query");
        }
        StringBuilder s = new StringBuilder(core.engine.base.toString()).append(p);
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

    private Request prepare(Operation op, boolean stream) {
        if (options.idempotencyKey() != null && !op.takesIdempotencyKey()) {
            throw new ConfigException(op.name() + " does not take an idempotency key: the API would ignore it, "
                    + "so repeating the call would not be safe");
        }
        URI url = url(op);
        String template = op.template() != null ? op.template() : (!op.name().equals(op.path()) ? op.path() : null);
        Call call = new Call(op, template, options.timeout(), options.traceparent(), options.idempotencyKey());
        Headers headers = new Headers().set("accept", stream ? "text/event-stream" : "application/json");
        byte[] body = body(op);
        if (body != null) {
            headers.set("content-type", "application/json");
        }
        Call.Info info = new Call.Info(
                call,
                op.name(),
                op.retrySafe(),
                null,
                Retry.requestId(),
                0,
                Long.MIN_VALUE,
                stream,
                core.engine.profile);
        return new Request(op.method().name(), url, headers, body, info);
    }

    private RawResponse finish(Request req, hr.inorbit.sdk.middleware.Response resp) {
        Call c = Call.of(req);
        RawResponse raw = new RawResponse(
                resp.status(),
                resp.headers(),
                resp.body() == null ? new byte[0] : resp.body(),
                req.info().requestId(),
                Math.max(c.attempts, 1),
                c.idempotencyKey,
                c.rateLimit);
        if (resp.status() < 200 || resp.status() >= 300) {
            Engine.discard(resp);
            throw failed(req, ApiException.of(raw));
        }
        logCall(req, resp.status(), null);
        return raw;
    }

    private InOrbitException failed(Request req, InOrbitException error) {
        Call c = Call.of(req);
        error.attach(req.info().requestId(), c.idempotencyKey);
        Engine e = core.engine;
        if (core.hooksCalls && !e.hooks.isEmpty()) {
            Hook.Attempt a = c.lastAttempt != null
                    ? c.lastAttempt
                    : new Hook.Attempt(
                            req.info().operation(),
                            c.op.method(),
                            c.op.path(),
                            Math.max(c.attempts, 1),
                            req.info().requestId(),
                            c.idempotencyKey,
                            "per_call");
            for (Hook h : e.hooks) {
                h.onError(a, error);
            }
        }
        Integer status = error instanceof ApiException api ? api.status() : null;
        logCall(req, status, error);
        if (core.logsCalls) {
            e.log.emit(
                    "error",
                    "call_failed",
                    Engine.fields(
                            "operation", req.info().operation(),
                            "error_kind", error.kind(),
                            "error_code",
                                    error instanceof ApiException api
                                            ? api.code().slug()
                                            : null,
                            "status", status,
                            "request_id", req.info().requestId()));
        }
        return error;
    }

    private void logCall(Request req, Integer status, InOrbitException error) {
        Call c = Call.of(req);
        Engine e = core.engine;
        double seconds = (System.nanoTime() - c.started) / 1e9;
        e.telemetry.record(
                "call",
                seconds,
                Engine.fields(
                        "inorbit.operation",
                        req.info().operation(),
                        "error.type",
                        error == null ? null : Engine.errorType(error)));
        if (!core.logsCalls) {
            return;
        }
        e.log.emit(
                "info",
                "call",
                Engine.fields(
                        "operation",
                        req.info().operation(),
                        "status",
                        status,
                        "error_kind",
                        error == null ? null : error.kind(),
                        "error_code",
                        error instanceof ApiException api ? api.code().slug() : null,
                        "attempts",
                        Math.max(c.attempts, 1),
                        "duration_ms",
                        Math.round(seconds * 1000),
                        "request_id",
                        req.info().requestId(),
                        "server_request_id",
                        error instanceof ApiException api
                                ? api.raw().serverRequestId().orElse(null)
                                : c.serverRequestId));
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
        return "Client[" + core.engine.base + ", " + core.provider + "]";
    }

    /** {@code inorbithr-sdk-java/<v> java/<v> <os>/<arch>[ <suffix>]}, normalised (section 7.6). */
    static String userAgent(String suffix) {
        String os = System.getProperty("os.name", "").toLowerCase(Locale.ROOT);
        String osName = os.startsWith("linux")
                ? (System.getProperty("java.vendor", "")
                                .toLowerCase(Locale.ROOT)
                                .contains("android")
                        ? "android"
                        : "linux")
                : os.startsWith("mac") || os.startsWith("darwin")
                        ? "macos"
                        : os.startsWith("windows") ? "windows" : os.startsWith("freebsd") ? "freebsd" : "other";
        String arch = switch (System.getProperty("os.arch", "").toLowerCase(Locale.ROOT)) {
            case "amd64", "x86_64" -> "x86_64";
            case "aarch64", "arm64" -> "aarch64";
            case "x86", "i386", "i486", "i586", "i686" -> "x86";
            case "arm", "arm32" -> "arm";
            case "riscv64" -> "riscv64";
            default -> "other";
        };
        String version = System.getProperty("java.version", "unknown").replace(' ', '-');
        return "inorbithr-sdk-java/" + SDK_VERSION + " java/" + version + " " + osName + "/" + arch
                + (suffix == null || suffix.isEmpty() ? "" : " " + suffix);
    }

    /** Builds a {@link Client}: {@link #build()} from these options alone, {@link #load()} reading more. */
    public static final class Builder {

        /** Settings set in code, by catalogue name. */
        private final Map<String, Object> code = new LinkedHashMap<>();

        private TokenProvider tokenProvider;
        private final List<Hook> hooks = new ArrayList<>();
        private HttpClient httpClient;
        private Executor executor;
        private Consumer<Pipeline> pipeline;
        private System.Logger logger;
        private UnaryOperator<Map<String, Object>> redact;
        private Object tracerProvider;
        private Object meterProvider;
        private Integer retryBudgetCapacity;
        private KeyStore clientKeyStore;
        private char[] clientKeyStorePassword;
        private String profileType;

        private Builder() {}

        private Builder put(String name, Object value) {
            if (value == null) {
                code.remove(name);
            } else {
                code.put(name, value);
            }
            return this;
        }

        /**
         * How streams open: {@link StreamTransport#SSE} (default) or every stream over one
         * {@code /v1/ws} socket.
         *
         * @param streams the transport
         * @return this builder
         */
        public Builder streams(StreamTransport streams) {
            return put(
                    "streams", Objects.requireNonNull(streams, "streams") == StreamTransport.SOCKET ? "socket" : "sse");
        }

        /**
         * How long a stream may be silent (no event, comment or ping) before it fails with a
         * timeout, or on the socket reconnects (default 45 s).
         *
         * @param idle the limit
         * @return this builder
         */
        public Builder streamIdleTimeout(Duration idle) {
            return put("stream_idle_timeout", idle);
        }

        /**
         * An API token (from the console or {@code iohr token create}).
         *
         * @param token the token
         * @return this builder
         */
        public Builder token(String token) {
            return put("token", token == null || token.isEmpty() ? null : token);
        }

        /**
         * A file holding a bearer token, read again when it changes (a mounted Secret).
         *
         * @param tokenFile the file
         * @return this builder
         */
        public Builder tokenFile(Path tokenFile) {
            return put("token_file", tokenFile == null ? null : tokenFile.toString());
        }

        /**
         * An API key, exchanged for short-lived tokens; give {@link #scopes} too.
         *
         * @param keyId the key's id
         * @param keySecret the key's secret
         * @return this builder
         */
        public Builder key(String keyId, String keySecret) {
            put("key_id", keyId);
            code.remove("key_secret_file");
            return put("key_secret", keySecret);
        }

        /**
         * An API key whose secret is a file, read before every token exchange (rotation).
         *
         * @param keyId the key's id
         * @param keySecretFile the file holding the secret
         * @return this builder
         */
        public Builder key(String keyId, Path keySecretFile) {
            put("key_id", keyId);
            code.remove("key_secret");
            return put("key_secret_file", keySecretFile == null ? null : keySecretFile.toString());
        }

        /**
         * The scopes to ask for with a key, a subset of the key's; no default.
         *
         * @param scopes the scopes
         * @return this builder
         */
        public Builder scopes(List<String> scopes) {
            return put("scopes", List.copyOf(scopes));
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
            return put("base_url", baseUrl);
        }

        /**
         * The token endpoint for a key (default {@code https://auth.inorbit.hr/oauth2/token}).
         *
         * @param tokenUrl the endpoint
         * @return this builder
         */
        public Builder tokenUrl(String tokenUrl) {
            return put("token_url", tokenUrl);
        }

        /**
         * How long each attempt may take, the answer's body included (default 30 s).
         *
         * @param timeout the limit
         * @return this builder
         */
        public Builder timeout(Duration timeout) {
            return put("timeout", timeout);
        }

        /**
         * DNS, TCP and TLS per new connection (default 10 s).
         *
         * @param timeout the limit
         * @return this builder
         */
        public Builder connectTimeout(Duration timeout) {
            return put("connect_timeout", timeout);
        }

        /**
         * One call, every attempt and every wait included (default 120 s).
         *
         * @param timeout the limit
         * @return this builder
         */
        public Builder totalTimeout(Duration timeout) {
            return put("total_timeout", timeout);
        }

        /**
         * Retries after the first attempt (default 2; 0 disables).
         *
         * @param maxRetries the retries
         * @return this builder
         */
        public Builder maxRetries(int maxRetries) {
            return put("max_retries", maxRetries);
        }

        /**
         * The exponential backoff's base (default 500 ms), full jitter.
         *
         * @param delay the base
         * @return this builder
         */
        public Builder retryBaseDelay(Duration delay) {
            return put("retry_base_delay", delay);
        }

        /**
         * The backoff's cap (default 8 s).
         *
         * @param delay the cap
         * @return this builder
         */
        public Builder retryMaxDelay(Duration delay) {
            return put("retry_max_delay", delay);
        }

        /**
         * The longest {@code Retry-After} waited for (default 60 s); a longer one ends the call.
         *
         * @param max the limit
         * @return this builder
         */
        public Builder retryAfterMax(Duration max) {
            return put("retry_after_max", max);
        }

        /**
         * The per-client retry quota (default on, section 7.4).
         *
         * @param on whether retries draw from the quota
         * @return this builder
         */
        public Builder retryBudget(boolean on) {
            return put("retry_budget", on);
        }

        /**
         * The retry quota's capacity (default 500), for tests.
         *
         * @param capacity the capacity
         * @return this builder
         */
        public Builder retryBudgetCapacity(int capacity) {
            this.retryBudgetCapacity = capacity;
            return this;
        }

        /**
         * An {@code http://} proxy URL (user-info is sent as Basic {@code Proxy-Authorization}),
         * or {@code off}.
         *
         * @param proxy the proxy
         * @return this builder
         */
        public Builder proxy(String proxy) {
            return put("proxy", proxy);
        }

        /**
         * Hosts reached directly (the grammar of docs/config.md section 6.2).
         *
         * @param noProxy the entries
         * @return this builder
         */
        public Builder noProxy(List<String> noProxy) {
            return put("no_proxy", List.copyOf(noProxy));
        }

        /**
         * PEM certificates added to the system's trust store.
         *
         * @param caBundle the file
         * @return this builder
         */
        public Builder caBundle(Path caBundle) {
            return put("ca_bundle", caBundle == null ? null : caBundle.toString());
        }

        /**
         * {@code false} trusts {@link #caBundle} only, for a private gateway.
         *
         * @param on whether the system's trust store is used
         * @return this builder
         */
        public Builder systemTrust(boolean on) {
            return put("system_trust", on);
        }

        /**
         * A PEM client certificate chain and its PKCS#8 key, for mTLS.
         *
         * @param cert the certificate chain
         * @param key the private key
         * @return this builder
         */
        public Builder clientCertificate(Path cert, Path key) {
            put("client_cert", cert == null ? null : cert.toString());
            return put("client_key", key == null ? null : key.toString());
        }

        /**
         * The password of an encrypted PKCS#8 client key.
         *
         * @param password the password
         * @return this builder
         */
        public Builder clientKeyPassword(String password) {
            return put("client_key_password", password);
        }

        /**
         * A client certificate and key from a {@link KeyStore} (PKCS#12, or a hardware store), for
         * mTLS.
         *
         * @param store the store
         * @param password the key's password
         * @return this builder
         */
        public Builder clientCertificate(KeyStore store, char[] password) {
            this.clientKeyStore = store;
            this.clientKeyStorePassword = password == null ? null : password.clone();
            return this;
        }

        /**
         * Base64 SHA-256 hashes of public keys to pin, at least two (the current and a backup).
         *
         * @param pins the pins
         * @return this builder
         */
        public Builder pinnedKeys(List<String> pins) {
            return put("pinned_keys", List.copyOf(pins));
        }

        /**
         * The log level: {@code off} (default), {@code error}, {@code warn}, {@code info}, {@code
         * debug}.
         *
         * @param level the level
         * @return this builder
         */
        public Builder log(String level) {
            return put("log", level);
        }

        /**
         * Log allowlisted header values at {@code debug}.
         *
         * @param on whether headers are logged
         * @return this builder
         */
        public Builder logHeaders(boolean on) {
            return put("log_headers", on);
        }

        /**
         * Header names added to the logging allowlist; the never-logged ones stay out.
         *
         * @param names the headers
         * @return this builder
         */
        public Builder logAllowHeaders(List<String> names) {
            return put("log_allow_headers", List.copyOf(names));
        }

        /**
         * Where records go (default {@code System.getLogger("hr.inorbit.sdk")}). Each record is
         * logged with format {@code "{0}"} and one parameter, the record as a {@link Map}.
         *
         * @param logger the logger
         * @return this builder
         */
        public Builder logger(System.Logger logger) {
            this.logger = logger;
            return this;
        }

        /**
         * Sees every record last and returns it changed, or {@code null} to drop it (SR-14).
         *
         * @param redact the function
         * @return this builder
         */
        public Builder redact(UnaryOperator<Map<String, Object>> redact) {
            this.redact = redact;
            return this;
        }

        /**
         * Spans through OpenTelemetry (default on when {@code opentelemetry-api} is present).
         *
         * @param on whether spans are made
         * @return this builder
         */
        public Builder tracing(boolean on) {
            return put("tracing", on);
        }

        /**
         * Metrics through OpenTelemetry (default as {@link #tracing}).
         *
         * @param on whether metrics are recorded
         * @return this builder
         */
        public Builder metrics(boolean on) {
            return put("metrics", on);
        }

        /**
         * The OpenTelemetry tracer provider (default the global one).
         *
         * @param provider the provider
         * @return this builder
         */
        public Builder tracerProvider(io.opentelemetry.api.trace.TracerProvider provider) {
            this.tracerProvider = provider;
            return this;
        }

        /**
         * The OpenTelemetry meter provider (default the global one).
         *
         * @param provider the provider
         * @return this builder
         */
        public Builder meterProvider(io.opentelemetry.api.metrics.MeterProvider provider) {
            this.meterProvider = provider;
            return this;
        }

        /**
         * {@code observe} (default), {@code wait} or {@code off} (section 7.8).
         *
         * @param mode the mode
         * @return this builder
         */
        public Builder rateLimit(String mode) {
            return put("rate_limit", mode);
        }

        /**
         * Edits the middleware pipeline: {@code p -> p.addPerRetry(m).remove("rate_limit")}.
         *
         * @param edit the edit
         * @return this builder
         */
        public Builder pipeline(Consumer<Pipeline> edit) {
            this.pipeline = edit;
            return this;
        }

        /**
         * The config file profile, for {@link #load()} (otherwise {@code INORBIT_PROFILE}, then
         * the file's {@code default}).
         *
         * @param profile the profile's name
         * @return this builder
         */
        public Builder profile(String profile) {
            return put("profile", profile);
        }

        /**
         * For a generated surface: resolve as the typed profile {@code name} (its own table and
         * {@code INORBIT_<NAME>_*} variables). Used by the generated {@code load()}.
         *
         * @param name the profile's name ({@code acme-ci})
         * @return this builder
         */
        public Builder profileType(String name) {
            this.profileType = name;
            return this;
        }

        /**
         * The config file {@link #load()} reads, or {@code off} for none.
         *
         * @param path the file
         * @return this builder
         */
        public Builder configFile(String path) {
            return put("config_file", path);
        }

        /**
         * The credential sources {@link #load()} may use: {@code env}, {@code workload}, {@code
         * file}, {@code cli}. Code is always allowed.
         *
         * @param sources the sources
         * @return this builder
         */
        public Builder credentialSources(List<String> sources) {
            return put("credential_sources", List.copyOf(sources));
        }

        /**
         * The command line the {@code cli} credential source runs (default {@code iohr} on {@code
         * PATH}).
         *
         * @param path the program
         * @return this builder
         */
        public Builder cliPath(String path) {
            return put("cli_path", path);
        }

        /**
         * Appended to the user agent: product tokens, at most 128 characters.
         *
         * @param suffix the suffix
         * @return this builder
         */
        public Builder userAgentSuffix(String suffix) {
            return put("user_agent_suffix", suffix == null || suffix.isEmpty() ? null : suffix);
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
         * The HTTP client to use (default one that follows no redirects). Proxy, trust,
         * certificate and connect timeout settings then belong to it.
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
         * The client, from these options alone: no environment, no config file.
         *
         * @return the client
         * @throws ConfigException when no credential is given, a key has no scopes, or a URL is not
         *     https (plain http only to this machine)
         */
        public Client build() {
            String base = (String) code.getOrDefault("base_url", DEFAULT_BASE_URL);
            checkUrl("the base URL", base, true);
            checkUrl(
                    "the token URL",
                    (String) code.getOrDefault("token_url", ClientCredentials.DEFAULT_TOKEN_URL),
                    false);
            if (code.get("max_retries") instanceof Integer n && n < 0) {
                throw new ConfigException("max retries must not be negative");
            }
            for (String d : List.of("timeout", "stream_idle_timeout")) {
                if (code.containsKey(d)) {
                    positive((Duration) code.get(d));
                }
            }
            if (tokenProvider == null && !code.containsKey("token")) {
                boolean key = code.containsKey("key_id")
                        && (code.containsKey("key_secret") || code.containsKey("key_secret_file"));
                if (key && !(code.get("scopes") instanceof List<?> l && !l.isEmpty())) {
                    throw new ConfigException(
                            "no scopes: set INORBIT_SCOPES (space-separated, such as \"identity:read account:read\")");
                }
                if (!key && !code.containsKey("token_file")) {
                    throw new ConfigException(
                            "no credentials: set INORBIT_TOKEN, or INORBIT_KEY_ID and INORBIT_KEY_SECRET");
                }
            }
            Map<String, Object> c = codeMap();
            c.put("config_file", "off");
            LoadOptions none = LoadOptions.builder().env(Map.of()).home("").build();
            Config.Resolution res = Config.resolve(c, none, null, p -> false, null);
            boolean ownTransport = Set.of(
                                    "connect_timeout",
                                    "proxy",
                                    "no_proxy",
                                    "ca_bundle",
                                    "system_trust",
                                    "client_cert",
                                    "client_key",
                                    "pinned_keys")
                            .stream()
                            .anyMatch(code::containsKey)
                    || clientKeyStore != null;
            return setup(res, ownTransport, false);
        }

        /**
         * A client from these options, the environment, the config file and the {@code iohr}
         * login (docs/config.md): each setting takes the first source that sets it.
         *
         * @return the client
         * @throws ConfigException every problem found, each with its setting and source
         */
        public Client load() {
            return load(null);
        }

        /**
         * {@link #load()} reading {@code options} instead of the process's environment, OS and
         * directories.
         *
         * @param options what to read
         * @return the client
         * @throws ConfigException every problem found, each with its setting and source
         */
        public Client load(LoadOptions options) {
            if (profileType != null && code.containsKey("profile")) {
                throw new ConfigException("a typed profile is its own profile; leave profile out");
            }
            Config.Resolution res = Config.resolve(codeMap(), options, profileType, null, null);
            String base = (String) res.values().get("base_url");
            checkUrl("the base URL", base, true);
            return setup(res, true, true);
        }

        private Map<String, Object> codeMap() {
            Map<String, Object> c = new LinkedHashMap<>(code);
            if (tokenProvider != null) {
                c.put("token_provider", tokenProvider);
            }
            if (httpClient != null) {
                c.put("http_client", httpClient);
            }
            return c;
        }

        @SuppressWarnings("unchecked")
        private Client setup(Config.Resolution res, boolean ownTransport, boolean loaded) {
            Map<String, Object> v = res.values();
            URI base = checkUrl("the base URL", (String) v.get("base_url"), true);
            URI tokenUrl = checkUrl("the token URL", (String) v.get("token_url"), false);
            String ua = userAgent((String) v.get("user_agent_suffix"));
            String proxySource =
                    res.doc().path("settings").path("proxy").path("source").asText("");
            NoProxy proxy = httpClient != null ? null : NoProxy.of(v, proxySource);
            if (proxy != null
                    && proxy.proxy() != null
                    && !"http".equals(proxy.proxy().getScheme())) {
                throw new ConfigException("proxy: Java's HTTP client reaches a proxy over plain http:// only; "
                        + "use an http:// proxy URL, or configure your own HttpClient");
            }
            HttpClient http;
            if (httpClient != null) {
                http = httpClient;
            } else if (ownTransport) {
                http = Transport.build(
                        base,
                        new Transport.Net(
                                (Duration) v.getOrDefault("connect_timeout", CONNECT_TIMEOUT),
                                proxy,
                                (String) v.get("ca_bundle"),
                                (Boolean) v.getOrDefault("system_trust", Boolean.TRUE),
                                (String) v.get("client_cert"),
                                (String) v.get("client_key"),
                                (String) v.get("client_key_password"),
                                clientKeyStore,
                                clientKeyStorePassword,
                                (List<String>) v.get("pinned_keys")));
            } else {
                http = HttpClient.newBuilder()
                        .followRedirects(HttpClient.Redirect.NEVER)
                        .connectTimeout(CONNECT_TIMEOUT)
                        .version(
                                "http".equals(base.getScheme())
                                        ? HttpClient.Version.HTTP_1_1
                                        : HttpClient.Version.HTTP_2)
                        .build();
                proxy = null;
            }
            Config.Credential cred = res.credential();
            TokenProvider provider = switch (cred.kind) {
                case "custom" -> (TokenProvider) cred.provider;
                case "static_token" -> new StaticToken(cred.token);
                case "token_file" -> new TokenFile(Path.of(cred.tokenFile));
                case "cli" -> new CliToken(cred.profile, cred.cliPath);
                default ->
                    new ClientCredentials(
                            cred.keyId,
                            cred.keySecret,
                            cred.keySecretFile == null ? null : Path.of(cred.keySecretFile),
                            cred.scopes,
                            tokenUrl,
                            http,
                            ua);
            };
            String profile = (String) v.get("profile");
            SdkLog log = new SdkLog(
                    (String) v.getOrDefault("log", "off"),
                    logger,
                    (Boolean) v.getOrDefault("log_headers", Boolean.FALSE),
                    (List<String>) v.getOrDefault("log_allow_headers", List.of()),
                    redact,
                    profile);
            Telemetry telemetry =
                    Telemetry.of((Boolean) v.get("tracing"), (Boolean) v.get("metrics"), tracerProvider, meterProvider);
            long maxRetries = (Long) v.get("max_retries");
            Engine engine = new Engine(
                    base,
                    ua,
                    (Duration) v.get("timeout"),
                    (Duration) v.get("total_timeout"),
                    (Duration) v.get("stream_idle_timeout"),
                    (int) Math.min(maxRetries, Integer.MAX_VALUE),
                    (Duration) v.get("retry_base_delay"),
                    (Duration) v.get("retry_max_delay"),
                    (Duration) v.get("retry_after_max"),
                    (String) v.get("rate_limit"),
                    new RetryBudget(retryBudgetCapacity != null ? retryBudgetCapacity : RetryBudget.CAPACITY, (Boolean)
                            v.get("retry_budget")),
                    log,
                    telemetry,
                    hooks,
                    profile,
                    cred.source,
                    http,
                    proxy,
                    loaded);
            if (provider instanceof Observable o) {
                o.listen((event, value) -> {
                    if (event.equals("token_exchange")) {
                        telemetry.record(
                                "exchanges",
                                1,
                                Engine.fields("inorbit.credential.source", cred.source, "error.type", value));
                    } else {
                        log.emit("warn", event, Engine.fields("reason", value));
                    }
                });
            }
            Pipeline p = engine.builtins(provider);
            if (pipeline != null) {
                pipeline.accept(p);
            }
            List<Middleware> ms = p.middlewares();
            boolean logsCalls = ms.stream()
                    .anyMatch(m -> m instanceof Engine.Builtin b && b.name().equals("logging"));
            boolean hooksCalls = ms.stream()
                    .anyMatch(m -> m instanceof Engine.Builtin b && b.name().equals("hooks"));
            com.fasterxml.jackson.databind.node.ObjectNode doc = res.doc();
            com.fasterxml.jackson.databind.node.ArrayNode names = doc.putArray("pipeline");
            p.names().forEach(names::add);
            Core core = new Core(
                    engine,
                    provider,
                    ms,
                    new ResolvedConfig(doc),
                    executor != null ? executor : Pool.EXECUTOR,
                    "socket".equals(v.get("streams")) ? StreamTransport.SOCKET : StreamTransport.SSE,
                    logsCalls,
                    hooksCalls);
            return new Client(core, CallOptions.NONE);
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
