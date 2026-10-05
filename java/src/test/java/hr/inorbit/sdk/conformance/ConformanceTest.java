package hr.inorbit.sdk.conformance;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertTrue;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import hr.inorbit.sdk.Client;
import hr.inorbit.sdk.EventStream;
import hr.inorbit.sdk.RawResponse;
import hr.inorbit.sdk.StreamTransport;
import hr.inorbit.sdk.errors.ApiException;
import hr.inorbit.sdk.errors.InOrbitException;
import hr.inorbit.sdk.generated.AccountsGetUsageParams;
import hr.inorbit.sdk.generated.CreateEndpointRequest;
import hr.inorbit.sdk.generated.EventsStreamEventsParams;
import hr.inorbit.sdk.generated.Public;
import hr.inorbit.sdk.generated.StreamEventsResponse;
import hr.inorbit.sdk.generated.UpdateEndpointRequest;
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
import java.time.Duration;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import java.util.concurrent.Callable;
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

    @Test
    void everyCasePasses() throws Exception {
        Path root = Path.of("..").toAbsolutePath().normalize();
        // `mise run conformance:server:build` writes bin/replay, bin/replay.exe on Windows.
        String exe = System.getProperty("os.name", "").startsWith("Windows") ? ".exe" : "";
        Path bin = root.resolve("conformance/server/bin/replay" + exe);
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
        } finally {
            server.destroy();
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
        JsonNode o = c.path("client");
        List<String> scopes = new ArrayList<>();
        for (JsonNode s : o.has("scopes") ? o.path("scopes") : JSON.readTree("[\"identity:read\"]")) {
            scopes.add(s.asText());
        }
        Client.Builder b = Client.builder()
                .baseUrl(url)
                .tokenUrl(url + "/oauth2/token")
                .key(o.path("key_id").asText("ak_test"), o.path("key_secret").asText("s3cr3t"))
                .scopes(scopes)
                .maxRetries(o.path("max_retries").asInt(2));
        if (o.has("timeout_ms")) {
            b.timeout(Duration.ofMillis(o.path("timeout_ms").asLong()));
        }
        if (o.path("streams").asText("sse").equals("socket")) {
            b.streams(StreamTransport.SOCKET);
        }
        if (o.has("stream_idle_timeout_ms")) {
            b.streamIdleTimeout(
                    Duration.ofMillis(o.path("stream_idle_timeout_ms").asLong()));
        }
        Public api = new Public(b.build());
        JsonNode action = c.path("action");
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
            }
        }
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
        if (problems.isEmpty()) {
            System.out.println("pass " + name);
        } else {
            failed.add(name + ":\n  " + String.join("\n  ", problems));
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
