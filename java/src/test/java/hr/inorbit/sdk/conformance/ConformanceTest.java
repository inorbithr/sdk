package hr.inorbit.sdk.conformance;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertTrue;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.node.ObjectNode;
import hr.inorbit.sdk.CallOptions;
import hr.inorbit.sdk.Client;
import hr.inorbit.sdk.EventStream;
import hr.inorbit.sdk.LoadOptions;
import hr.inorbit.sdk.RawResponse;
import hr.inorbit.sdk.StreamTransport;
import hr.inorbit.sdk.errors.ApiException;
import hr.inorbit.sdk.errors.InOrbitException;
import hr.inorbit.sdk.generated.AccountsGetUsageParams;
import hr.inorbit.sdk.generated.CreateDocumentRequest;
import hr.inorbit.sdk.generated.CreateEndpointRequest;
import hr.inorbit.sdk.generated.EventsStreamEventsParams;
import hr.inorbit.sdk.generated.Public;
import hr.inorbit.sdk.generated.StreamEventsResponse;
import hr.inorbit.sdk.generated.UpdateEndpointRequest;
import hr.inorbit.sdk.middleware.Chain;
import hr.inorbit.sdk.middleware.Middleware;
import hr.inorbit.sdk.middleware.Request;
import io.opentelemetry.sdk.testing.exporter.InMemorySpanExporter;
import io.opentelemetry.sdk.trace.IdGenerator;
import io.opentelemetry.sdk.trace.SdkTracerProvider;
import io.opentelemetry.sdk.trace.data.SpanData;
import io.opentelemetry.sdk.trace.export.SimpleSpanProcessor;
import java.io.BufferedReader;
import java.io.IOException;
import java.io.InputStreamReader;
import java.net.URI;
import java.net.http.HttpClient;
import java.net.http.HttpRequest;
import java.net.http.HttpResponse;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.attribute.FileTime;
import java.time.Duration;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.ResourceBundle;
import java.util.Set;
import java.util.concurrent.Callable;
import java.util.concurrent.CopyOnWriteArrayList;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.Future;
import org.junit.jupiter.api.Assumptions;
import org.junit.jupiter.api.Tag;
import org.junit.jupiter.api.Test;

/**
 * The driver for {@code conformance/cases}: starts the replay server, loads every case, runs its
 * action through the generated public surface, and compares the result and the server's verdict
 * with {@code expect} ({@code conformance/README.md}). Without the server binary ({@code mise run
 * conformance:server:build}) it skips, unless {@code IOHR_TEST_REQUIRE_REPLAY} is set.
 */
@Tag("conformance")
class ConformanceTest {

    private static final ObjectMapper JSON = new ObjectMapper();
    private static final HttpClient HTTP =
            HttpClient.newBuilder().version(HttpClient.Version.HTTP_1_1).build();

    private static final Path ROOT = Path.of("..").toAbsolutePath().normalize();
    // `mise run conformance:server:build` writes bin/replay, bin/replay.exe on Windows.
    private static final Path BIN = ROOT.resolve(
            "conformance/server/bin/replay" + (System.getProperty("os.name", "").startsWith("Windows") ? ".exe" : ""));

    @Test
    void everyCasePasses() throws Exception {
        Path root = ROOT;
        Path bin = BIN;
        if (!Files.isExecutable(bin)) {
            String note =
                    "conformance: no replay server at conformance/server/bin/replay; run `mise run conformance:server:build`";
            assertTrue(System.getenv("IOHR_TEST_REQUIRE_REPLAY") == null, note);
            Assumptions.abort(note + "; skipping");
        }
        Process server = new ProcessBuilder(
                        bin.toString(),
                        "--addr",
                        "127.0.0.1:0",
                        "--cases",
                        root.resolve("conformance/cases").toString())
                .redirectError(ProcessBuilder.Redirect.INHERIT)
                .start();
        try {
            BufferedReader out =
                    new BufferedReader(new InputStreamReader(server.getInputStream(), StandardCharsets.UTF_8));
            String first = out.readLine();
            String prefix = "replay: listening on ";
            assertTrue(first != null && first.startsWith(prefix), "unexpected first line: " + first);
            String url = first.substring(prefix.length()).strip();
            List<String> failed = new ArrayList<>();
            for (JsonNode name : get(url + "/_cases").path("cases")) {
                runCase(url, name.asText(), failed);
            }
            assertEquals(List.of(), failed, String.join("\n", failed));
            pinnedKeysHoldTheConnectionToItsKey(url);
        } finally {
            // SIGTERM, so the server removes its certificate directory.
            server.destroy();
            server.waitFor(10, java.util.concurrent.TimeUnit.SECONDS);
        }
    }

    private static void runCase(String url, String name, List<String> failed) throws Exception {
        HttpResponse<String> loaded = HTTP.send(
                HttpRequest.newBuilder(URI.create(url + "/_case"))
                        .POST(HttpRequest.BodyPublishers.ofString(JSON.writeValueAsString(Map.of("name", name))))
                        .build(),
                HttpResponse.BodyHandlers.ofString());
        if (loaded.statusCode() == 501) {
            System.out.println("skip " + name + ": not implemented by the replay server yet");
            return;
        }
        assertEquals(200, loaded.statusCode(), name + ": loading answered " + loaded.statusCode());
        JsonNode c = JSON.readTree(loaded.body()).path("case");
        String area = c.path("area").asText();
        if (contains(c.path("pending"), "java")) {
            System.out.println("skip " + name + ": pending for java");
            return;
        }
        JsonNode answer = JSON.readTree(loaded.body());
        Path dir = Files.createTempDirectory("java-conformance");
        try {
            Setup setup = new Setup(c, answer, dir);
            Client client;
            try {
                client = configure(setup);
            } catch (InOrbitException e) {
                failed.add(name + ":\n  the client could not be built: " + e.getMessage());
                get(url + "/_result");
                return;
            }
            JsonNode action = c.path("action");
            Client caller = client;
            JsonNode opts = action.path("options");
            if (opts.isObject() && opts.size() > 0) {
                CallOptions.Builder ob = CallOptions.builder();
                if (opts.has("idempotency_key")) {
                    ob.idempotencyKey(opts.path("idempotency_key").asText());
                }
                if (opts.has("traceparent")) {
                    ob.traceparent(opts.path("traceparent").asText());
                }
                if (opts.has("timeout_ms")) {
                    ob.timeout(Duration.ofMillis(opts.path("timeout_ms").asLong()));
                }
                caller = client.withOptions(ob.build());
            }
            Public api = new Public(caller);
            Callable<Object> run = () -> {
                if (action.path("op").asText().equals("events.stream_events")) {
                    return stream(api, action);
                }
                try {
                    return call(api, action);
                } catch (InOrbitException e) {
                    return e;
                }
            };
            List<Object> results = new ArrayList<>();
            if (action.has("concurrent")) {
                ExecutorService pool =
                        Executors.newFixedThreadPool(action.path("concurrent").asInt());
                try {
                    List<Future<Object>> futures = new ArrayList<>();
                    for (int i = 0; i < action.path("concurrent").asInt(); i++) {
                        futures.add(pool.submit(run));
                    }
                    for (Future<Object> f : futures) {
                        results.add(f.get());
                    }
                } finally {
                    pool.shutdown();
                }
            } else {
                for (int i = 0; i < action.path("repeat").asInt(1); i++) {
                    results.add(run.call());
                    rewrite(setup, i + 1);
                }
            }
            JsonNode config = client.config().describe();
            runChecks(name, url, c, setup, results, config, failed);
        } finally {
            deleteTree(dir);
        }
    }

    private static void runChecks(
            String name,
            String url,
            JsonNode c,
            Setup setup,
            List<Object> results,
            JsonNode config,
            List<String> failed)
            throws Exception {
        JsonNode verdict = get(url + "/_result");
        JsonNode expect = c.path("expect");
        List<String> problems = new ArrayList<>();
        if (!verdict.path("status").asText().equals("pass")) {
            problems.add("server: " + verdict.path("status").asText() + " mismatch=" + verdict.path("mismatch")
                    + " next=" + verdict.path("next"));
        }
        if (expect.has("attempts")
                && verdict.path("attempts").asInt() != expect.path("attempts").asInt()) {
            problems.add("attempts: want " + expect.path("attempts") + ", got " + verdict.path("attempts"));
        }
        if (expect.has("token_exchanges")
                && verdict.path("token_exchanges").asInt()
                        != expect.path("token_exchanges").asInt()) {
            problems.add("token_exchanges: want " + expect.path("token_exchanges") + ", got "
                    + verdict.path("token_exchanges"));
        }
        for (Object r : results) {
            check(r, expect, problems);
        }
        Object last = results.isEmpty() ? null : results.get(results.size() - 1);
        if (last instanceof RawResponse raw) {
            checkResult(expect, raw, problems);
        }
        checkWatched(expect, setup, config, problems);
        if (problems.isEmpty()) {
            System.out.println("pass " + name);
        } else {
            failed.add(name + ":\n  " + String.join("\n  ", problems));
        }
    }

    /** SR-06: a matching pin connects, a set of pins that matches nothing does not. */
    private static void pinnedKeysHoldTheConnectionToItsKey(String url) throws Exception {
        String other = "47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU="; // SHA-256 of nothing
        for (boolean match : new boolean[] {true, false}) {
            HttpResponse<String> loaded = HTTP.send(
                    HttpRequest.newBuilder(URI.create(url + "/_case"))
                            .POST(HttpRequest.BodyPublishers.ofString("{\"name\":\"a-private-ca-is-trusted\"}"))
                            .build(),
                    HttpResponse.BodyHandlers.ofString());
            JsonNode a = JSON.readTree(loaded.body());
            String https = a.path("https_url").asText();
            String pin = match ? leafPin(https) : other;
            Client client = Client.builder()
                    .baseUrl(https)
                    .tokenUrl(https + "/oauth2/token")
                    .key("ak_test", "s3cr3t")
                    .scopes(List.of("identity:read"))
                    .caBundle(Path.of(a.path("ca_file").asText()))
                    .pinnedKeys(List.of(other, pin))
                    .maxRetries(0)
                    .build();
            boolean ok;
            try {
                new Public(client).me();
                ok = true;
            } catch (InOrbitException e) {
                ok = false;
            }
            assertEquals(match, ok, "pins matching=" + match);
        }
    }

    /** The SPKI pin of the certificate the replay's TLS listener presents. */
    private static String leafPin(String https) throws Exception {
        URI u = URI.create(https);
        java.security.cert.Certificate[] seen = new java.security.cert.Certificate[1];
        javax.net.ssl.TrustManager record = new javax.net.ssl.X509TrustManager() {
            @Override
            public void checkClientTrusted(java.security.cert.X509Certificate[] chain, String authType) {}

            @Override
            public void checkServerTrusted(java.security.cert.X509Certificate[] chain, String authType) {
                seen[0] = chain[0];
            }

            @Override
            public java.security.cert.X509Certificate[] getAcceptedIssuers() {
                return new java.security.cert.X509Certificate[0];
            }
        };
        javax.net.ssl.SSLContext ctx = javax.net.ssl.SSLContext.getInstance("TLS");
        ctx.init(null, new javax.net.ssl.TrustManager[] {record}, null);
        try (javax.net.ssl.SSLSocket sock =
                (javax.net.ssl.SSLSocket) ctx.getSocketFactory().createSocket(u.getHost(), u.getPort())) {
            sock.startHandshake();
        }
        byte[] spki = seen[0].getPublicKey().getEncoded();
        return java.util.Base64.getEncoder()
                .encodeToString(
                        java.security.MessageDigest.getInstance("SHA-256").digest(spki));
    }

    /** A case's client options, what the driver watches, and its temporary directory. */
    private static final class Setup {
        final JsonNode c;
        final JsonNode answer;
        final Path dir;
        final Map<String, List<Map<String, String>>> probes = new LinkedHashMap<>();
        final List<Map<String, Object>> records = new CopyOnWriteArrayList<>();
        final List<String> messages = new CopyOnWriteArrayList<>();
        InMemorySpanExporter exporter;

        Setup(JsonNode c, JsonNode answer, Path dir) {
            this.c = c;
            this.answer = answer;
            this.dir = dir;
        }

        String sub(String v) {
            return v.replace("{replay}", answer.path("base_url").asText()).replace("{dir}", dir.toString());
        }
    }

    /** A user middleware that records the headers of every request it sees. */
    private record Probe(String name, List<Map<String, String>> seen) implements Middleware {
        @Override
        public hr.inorbit.sdk.middleware.Response handle(Request request, Chain chain) {
            seen.add(new LinkedHashMap<>(request.headers().asMap()));
            return chain.proceed(request);
        }
    }

    /**
     * Random ids without the W3C level 2 random flag, so {@code traceparent} ends in {@code -01}:
     * the case's pattern is level 1's, and the SDK sends what OpenTelemetry gives it.
     */
    private static final class LevelOneIds implements IdGenerator {
        private final IdGenerator random = IdGenerator.random();

        @Override
        public String generateSpanId() {
            return random.generateSpanId();
        }

        @Override
        public String generateTraceId() {
            return random.generateTraceId();
        }
    }

    /** Keeps every record the client logs, as data and as text. */
    private record Capture(Setup setup) implements System.Logger {
        @Override
        public String getName() {
            return "capture";
        }

        @Override
        public boolean isLoggable(Level level) {
            return true;
        }

        @Override
        @SuppressWarnings("unchecked")
        public void log(Level level, ResourceBundle bundle, String format, Object... params) {
            if (params.length == 1 && params[0] instanceof Map<?, ?> m) {
                setup.records.add((Map<String, Object>) m);
            }
            setup.messages.add(java.text.MessageFormat.format(format, params));
        }

        @Override
        public void log(Level level, ResourceBundle bundle, String msg, Throwable thrown) {
            setup.messages.add(msg);
        }
    }

    private static void writeFiles(Setup setup, JsonNode files) throws IOException {
        for (Map.Entry<String, JsonNode> f : files.properties()) {
            Path p = setup.dir.resolve(f.getKey());
            Files.createDirectories(p.getParent());
            Files.writeString(p, setup.sub(f.getValue().asText()));
        }
    }

    /** The client a case's {@code client} asks for (conformance/README.md). */
    private static Client configure(Setup setup) throws IOException {
        JsonNode o = setup.c.path("client");
        JsonNode a = setup.answer;
        String url = a.path("base_url").asText();
        Client.Builder b = Client.builder();
        LoadOptions load = null;
        if (o.path("load").asBoolean(false)) {
            Map<String, String> env = new LinkedHashMap<>();
            o.path("env")
                    .properties()
                    .forEach(e -> env.put(e.getKey(), setup.sub(e.getValue().asText())));
            writeFiles(setup, o.path("files"));
            if (o.has("config_file")) {
                Path p = setup.dir.resolve("config.toml");
                Files.writeString(p, setup.sub(o.path("config_file").asText()));
                b.configFile(p.toString());
            }
            if (o.path("cli").asBoolean(false)) {
                b.cliPath(BIN.toString());
            }
            if (o.has("profile")) {
                b.profile(o.path("profile").asText());
            }
            if (o.has("credential_sources")) {
                List<String> l = new ArrayList<>();
                o.path("credential_sources").forEach(x -> l.add(x.asText()));
                b.credentialSources(l);
            }
            load = LoadOptions.builder()
                    .env(env)
                    .home("")
                    .cwd(setup.dir.toString())
                    .build();
        } else {
            List<String> scopes = new ArrayList<>();
            for (JsonNode s : o.has("scopes") ? o.path("scopes") : JSON.readTree("[\"identity:read\"]")) {
                scopes.add(s.asText());
            }
            b.baseUrl(url)
                    .tokenUrl(url + "/oauth2/token")
                    .key(
                            o.path("key_id").asText("ak_test"),
                            o.path("key_secret").asText("s3cr3t"))
                    .scopes(scopes)
                    .maxRetries(o.path("max_retries").asInt(2));
        }
        if (o.has("max_retries")) {
            b.maxRetries(o.path("max_retries").asInt());
        }
        if (o.has("timeout_ms")) {
            b.timeout(Duration.ofMillis(o.path("timeout_ms").asLong()));
        }
        if (o.has("total_timeout_ms")) {
            b.totalTimeout(Duration.ofMillis(o.path("total_timeout_ms").asLong()));
        }
        if (o.path("streams").asText("sse").equals("socket")) {
            b.streams(StreamTransport.SOCKET);
        }
        if (o.has("stream_idle_timeout_ms")) {
            b.streamIdleTimeout(
                    Duration.ofMillis(o.path("stream_idle_timeout_ms").asLong()));
        }
        if (o.has("rate_limit")) {
            b.rateLimit(o.path("rate_limit").asText());
        }
        if (o.has("retry_budget_capacity")) {
            b.retryBudgetCapacity(o.path("retry_budget_capacity").asInt());
        }
        if (o.has("no_proxy")) {
            b.noProxy(List.of(o.path("no_proxy").asText().split(",")));
        }
        if (o.has("log_headers")) {
            b.logHeaders(o.path("log_headers").asBoolean());
        }
        if (o.has("log_allow_headers")) {
            List<String> l = new ArrayList<>();
            o.path("log_allow_headers").forEach(x -> l.add(x.asText()));
            b.logAllowHeaders(l);
        }
        if (o.has("log")) {
            b.log(o.path("log").asText()).logger(new Capture(setup));
        }
        String transport = o.path("transport").asText("");
        if (Set.of("https", "mtls", "proxy").contains(transport)
                && o.path("ca_bundle").asBoolean(true)) {
            b.caBundle(Path.of(a.path("ca_file").asText()));
        }
        if (transport.equals("mtls")) {
            b.clientCertificate(
                    Path.of(a.path("client_cert_file").asText()),
                    Path.of(a.path("client_key_file").asText()));
        }
        if (transport.equals("proxy")) {
            b.proxy(a.path("proxy_url").asText());
        }
        if (o.path("tracing").isBoolean()) {
            if (o.path("tracing").asBoolean()) {
                setup.exporter = InMemorySpanExporter.create();
                b.tracing(true)
                        .tracerProvider(SdkTracerProvider.builder()
                                .setIdGenerator(new LevelOneIds())
                                .addSpanProcessor(SimpleSpanProcessor.create(setup.exporter))
                                .build());
            } else {
                b.tracing(false);
            }
        }
        JsonNode pipeline = o.path("pipeline");
        if (pipeline.isObject()) {
            b.pipeline(p -> {
                for (JsonNode add : pipeline.path("add")) {
                    String n = add.path("name").asText();
                    Probe probe = new Probe(n, setup.probes.computeIfAbsent(n, k -> new CopyOnWriteArrayList<>()));
                    if (add.path("stage").asText().equals("per_call")) {
                        p.addPerCall(probe);
                    } else {
                        p.addPerRetry(probe);
                    }
                }
                for (JsonNode r : pipeline.path("remove")) {
                    p.remove(r.asText());
                }
            });
        }
        return load != null ? b.load(load) : b.build();
    }

    /** Rewrites the action's files after {@code after} calls (rotation), with a new modification time. */
    private static void rewrite(Setup setup, int done) throws IOException {
        JsonNode spec = setup.c.path("action").path("rewrite");
        if (spec.isObject() && spec.path("after").asInt() == done) {
            writeFiles(setup, spec.path("files"));
            for (Map.Entry<String, JsonNode> f : spec.path("files").properties()) {
                Path p = setup.dir.resolve(f.getKey());
                Files.setLastModifiedTime(
                        p, FileTime.fromMillis(Files.getLastModifiedTime(p).toMillis() + 2_000));
            }
        }
    }

    private static void deleteTree(Path dir) throws IOException {
        try (var walk = Files.walk(dir)) {
            for (Path p : walk.sorted(java.util.Comparator.reverseOrder()).toList()) {
                Files.deleteIfExists(p);
            }
        }
    }

    /** A header value against a matcher: a literal, {@code *}, {@code $name} or {@code ~regex}. */
    private static boolean matches(String want, String got, Map<String, String> captures) {
        if (got == null) {
            return false;
        }
        if (want.equals("*")) {
            return true;
        }
        if (want.startsWith("$")) {
            return captures.computeIfAbsent(want, k -> got).equals(got);
        }
        if (want.startsWith("~")) {
            return got.matches(want.substring(1));
        }
        return want.equals(got);
    }

    private static void checkResult(JsonNode expect, RawResponse r, List<String> problems) {
        if (expect.has("rate_limit")) {
            ObjectNode got = JSON.createObjectNode();
            r.rateLimit().ifPresent(rl -> {
                rl.limit().ifPresent(x -> got.put("limit", x));
                rl.remaining().ifPresent(x -> got.put("remaining", x));
                rl.reset().ifPresent(x -> got.put("reset_ms", x.toMillis()));
            });
            if (r.rateLimit().isEmpty() || !subset(expect.path("rate_limit"), got)) {
                problems.add("rate_limit: want " + expect.path("rate_limit") + ", got " + got);
            }
        }
        if (expect.has("idempotency_key")) {
            String want = expect.path("idempotency_key").asText();
            String key = r.idempotencyKey().orElse(null);
            boolean ok = want.equals("*") ? key != null : want.equals(key);
            if (!ok) {
                problems.add("idempotency_key: want " + want + ", got " + key);
            }
        }
    }

    private static void checkWatched(JsonNode expect, Setup setup, JsonNode config, List<String> problems)
            throws IOException {
        for (Map.Entry<String, JsonNode> e : expect.path("probes").properties()) {
            List<Map<String, String>> seen = setup.probes.getOrDefault(e.getKey(), List.of());
            JsonNode want = e.getValue();
            if (want.has("count") && seen.size() != want.path("count").asInt()) {
                problems.add("probe " + e.getKey() + ": ran " + seen.size() + " times, want " + want.path("count"));
            }
            Map<String, String> captures = new HashMap<>();
            int i = 0;
            for (JsonNode headers : want.path("seen")) {
                Map<String, String> got = i < seen.size() ? seen.get(i) : Map.of();
                for (Map.Entry<String, JsonNode> h : headers.properties()) {
                    if (!matches(h.getValue().asText(), got.get(h.getKey()), captures)) {
                        problems.add("probe " + e.getKey() + " #" + (i + 1) + ": " + h.getKey() + " "
                                + got.get(h.getKey()) + " does not match " + h.getValue());
                    }
                }
                i++;
            }
        }
        JsonNode logs = expect.path("logs");
        List<JsonNode> records = new ArrayList<>();
        for (Map<String, Object> r : setup.records) {
            records.add(JSON.valueToTree(r));
        }
        for (JsonNode want : logs.path("contains")) {
            if (records.stream().noneMatch(r -> subset(want, r))) {
                problems.add("logs: no record contains " + want + "; got " + records);
            }
        }
        String text = JSON.writeValueAsString(records) + String.join("\n", setup.messages);
        for (JsonNode x : logs.path("excludes")) {
            if (text.contains(x.asText())) {
                problems.add("logs: " + x + " appears in a record");
            }
        }
        if (expect.has("spans")) {
            List<SpanData> spans = new ArrayList<>(setup.exporter.getFinishedSpanItems());
            spans.sort(java.util.Comparator.comparingLong(SpanData::getStartEpochNanos));
            List<JsonNode> got = new ArrayList<>();
            for (SpanData s : spans) {
                ObjectNode n = JSON.createObjectNode();
                n.put("name", s.getName());
                n.put("kind", s.getKind().name().toLowerCase(Locale.ROOT));
                ObjectNode attrs = n.putObject("attributes");
                s.getAttributes().forEach((k, v) -> attrs.set(k.getKey(), JSON.valueToTree(v)));
                got.add(n);
            }
            JsonNode want = expect.path("spans");
            if (got.size() != want.size()) {
                problems.add("spans: want " + want.size() + ", got " + got);
            }
            for (int i = 0; i < Math.min(got.size(), want.size()); i++) {
                if (!subset(want.get(i), got.get(i))) {
                    problems.add("spans: #" + (i + 1) + " " + got.get(i) + " does not match " + want.get(i));
                }
            }
        }
        if (expect.has("config") && !subset(expect.path("config"), config)) {
            problems.add("config: want " + expect.path("config") + ", got " + config);
        }
    }

    /** What a stream yielded before it ended, and the error that ended it, if one did. */
    private record Streamed(List<JsonNode> items, InOrbitException error) {}

    /** Reads the stream, stopping after {@code take} items when the case says so. */
    private static Streamed stream(Public api, JsonNode action) {
        JsonNode args = action.path("args");
        int take = action.path("take").asInt(Integer.MAX_VALUE);
        List<JsonNode> items = new ArrayList<>();
        try (EventStream<StreamEventsResponse> events = api.events()
                .streamEvents(EventsStreamEventsParams.builder()
                        .types(text(args, "types"))
                        .build())) {
            for (StreamEventsResponse ev : events) {
                items.add(JSON.valueToTree(ev));
                if (items.size() >= take) {
                    break;
                }
            }
        } catch (InOrbitException e) {
            return new Streamed(items, e);
        }
        return new Streamed(items, null);
    }

    private static RawResponse call(Public api, JsonNode action) {
        JsonNode args = action.path("args");
        return switch (action.path("op").asText()) {
            case "me" -> api.me().raw();
            case "accounts.get_me" -> api.accounts().getMe().raw();
            case "accounts.get_usage" ->
                api.accounts()
                        .getUsage(
                                args.path("org_id").asText(),
                                AccountsGetUsageParams.builder()
                                        .from(text(args, "from"))
                                        .to(text(args, "to"))
                                        .build())
                        .raw();
            case "radar.get_digest" ->
                api.radar().getDigest(args.path("id").asText()).raw();
            case "events.create_endpoint" -> {
                List<String> types = null;
                if (args.has("event_types")) {
                    types = new ArrayList<>();
                    for (JsonNode type : args.path("event_types")) {
                        types.add(type.asText());
                    }
                }
                yield api.events()
                        .createEndpoint(CreateEndpointRequest.builder()
                                .accountId(text(args, "account_id"))
                                .url(text(args, "url"))
                                .description(text(args, "description"))
                                .eventTypes(types)
                                .build())
                        .raw();
            }
            case "events.update_endpoint" -> {
                List<String> types = null;
                if (args.has("event_types")) {
                    types = new ArrayList<>();
                    for (JsonNode type : args.path("event_types")) {
                        types.add(type.asText());
                    }
                }
                Boolean enabled = args.has("enabled") ? args.path("enabled").asBoolean() : null;
                yield api.events()
                        .updateEndpoint(
                                args.path("endpoint_id").asText(),
                                UpdateEndpointRequest.builder()
                                        .url(text(args, "url"))
                                        .description(text(args, "description"))
                                        .enabled(enabled)
                                        .eventTypes(types)
                                        .build())
                        .raw();
            }
            case "events.delete_endpoint" ->
                api.events().deleteEndpoint(args.path("endpoint_id").asText()).raw();
            case "rfcs.create_document" ->
                api.rfcs()
                        .createDocument(
                                args.path("space_id").asText(),
                                CreateDocumentRequest.builder()
                                        .kind(text(args, "kind"))
                                        .title(text(args, "title"))
                                        .summary(text(args, "summary"))
                                        .build())
                        .raw();
            default ->
                throw new IllegalArgumentException("the conformance schema names an op this driver does not know: "
                        + action.path("op").asText());
        };
    }

    private static void check(Object result, JsonNode expect, List<String> problems) {
        if (result instanceof Streamed streamed) {
            JsonNode want = expect.path("items");
            if (expect.has("items")
                    && (want.size() != streamed.items().size() || !subset(want, JSON.valueToTree(streamed.items())))) {
                problems.add("items: want " + want + ", got " + streamed.items());
            }
            if (streamed.error() != null) {
                result = streamed.error();
            } else if (expect.has("error")) {
                problems.add("want the stream to end with an error, it ended cleanly");
                return;
            } else {
                return;
            }
        }
        boolean isError = result instanceof InOrbitException;
        if (!isError && expect.has("ok")) {
            JsonNode got = ((RawResponse) result).json();
            if (!subset(expect.path("ok"), got)) {
                problems.add("ok: want a superset of " + expect.path("ok") + ", got " + got);
            }
        } else if (!isError && expect.has("error")) {
            problems.add("want an error, got HTTP " + ((RawResponse) result).status());
        } else if (isError && expect.has("error")) {
            InOrbitException e = (InOrbitException) result;
            JsonNode want = expect.path("error");
            if (want.has("kind") && !e.kind().equals(want.path("kind").asText())) {
                problems.add("error kind: want " + want.path("kind").asText() + ", got " + e.kind() + " ("
                        + e.getMessage() + ")");
            }
            if (e instanceof ApiException api) {
                if (want.has("code")
                        && !api.code().slug().equals(want.path("code").asText())) {
                    problems.add("error code: want " + want.path("code").asText() + ", got " + api.code());
                }
                if (want.has("status") && api.status() != want.path("status").asInt()) {
                    problems.add("error status: want " + want.path("status") + ", got " + api.status());
                }
            } else if (want.has("code") || want.has("status")) {
                problems.add("error: want an API error, got " + e.getMessage());
            }
            if (want.has("message_contains")
                    && !e.getMessage().contains(want.path("message_contains").asText())) {
                problems.add(
                        "message: want it to contain " + want.path("message_contains") + ", got " + e.getMessage());
            }
            if (want.has("message_excludes")
                    && e.getMessage().contains(want.path("message_excludes").asText())) {
                problems.add("message: must not contain " + want.path("message_excludes") + ", got " + e.getMessage());
            }
        } else if (isError && expect.has("ok")) {
            problems.add("want ok, got " + ((InOrbitException) result).getMessage());
        }
    }

    private static boolean subset(JsonNode want, JsonNode got) {
        if (got == null) {
            return false;
        }
        if (want.isArray()) {
            if (!got.isArray() || got.size() != want.size()) {
                return false;
            }
            for (int i = 0; i < want.size(); i++) {
                if (!subset(want.get(i), got.get(i))) {
                    return false;
                }
            }
            return true;
        }
        if (want.isObject()) {
            if (!got.isObject()) {
                return false;
            }
            for (Map.Entry<String, JsonNode> e : want.properties()) {
                if (!subset(e.getValue(), got.get(e.getKey()))) {
                    return false;
                }
            }
            return true;
        }
        if (want.isNumber() && got.isNumber()) {
            return want.asDouble() == got.asDouble();
        }
        if (want.isTextual() && !got.isTextual() && !got.isContainerNode()) {
            return want.asText().equals(got.asText());
        }
        return want.equals(got);
    }

    private static boolean contains(JsonNode list, String value) {
        for (JsonNode v : list) {
            if (v.asText().equals(value)) {
                return true;
            }
        }
        return false;
    }

    private static String text(JsonNode args, String key) {
        return args.has(key) ? args.path(key).asText() : null;
    }

    private static JsonNode get(String url) throws IOException, InterruptedException {
        HttpResponse<String> resp =
                HTTP.send(HttpRequest.newBuilder(URI.create(url)).GET().build(), HttpResponse.BodyHandlers.ofString());
        return JSON.readTree(resp.body());
    }
}
