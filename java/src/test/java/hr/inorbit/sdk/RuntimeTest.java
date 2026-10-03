package hr.inorbit.sdk;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertInstanceOf;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

import com.fasterxml.jackson.databind.ObjectMapper;
import com.sun.net.httpserver.HttpServer;
import hr.inorbit.sdk.auth.ClientCredentials;
import hr.inorbit.sdk.auth.StaticToken;
import hr.inorbit.sdk.auth.Token;
import hr.inorbit.sdk.codegen.Codegen;
import hr.inorbit.sdk.errors.ApiException;
import hr.inorbit.sdk.errors.Code;
import hr.inorbit.sdk.errors.ConfigException;
import hr.inorbit.sdk.errors.Detail;
import hr.inorbit.sdk.generated.Digest;
import java.net.InetSocketAddress;
import java.net.URI;
import java.net.http.HttpClient;
import java.net.http.HttpHeaders;
import java.nio.charset.StandardCharsets;
import java.time.Duration;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.Future;
import java.util.concurrent.atomic.AtomicInteger;
import org.junit.jupiter.api.Test;

class RuntimeTest {

    private static RawResponse raw(int status, String body, Map<String, List<String>> headers) {
        return new RawResponse(
                status,
                HttpHeaders.of(headers, (a, b) -> true),
                body.getBytes(StandardCharsets.UTF_8),
                "iohr-0000000000000000",
                1);
    }

    @Test
    void secretsNeverPrint() {
        HttpClient http = HttpClient.newHttpClient();
        ClientCredentials cc = new ClientCredentials(
                "ak_1", "s3cr3t", List.of("identity:read"), URI.create("https://auth.inorbit.hr/oauth2/token"), http);
        assertFalse(cc.toString().contains("s3cr3t"), cc.toString());
        assertTrue(cc.toString().contains("ak_1"));
        assertFalse(new StaticToken("tok-123").toString().contains("tok-123"));
        assertFalse(new Token("tok-123", null).toString().contains("tok-123"));
        Client client = Client.builder().token("tok-123").build();
        assertFalse(client.toString().contains("tok-123"), client.toString());
    }

    @Test
    void codesAreKeptWhenUnknown() {
        assertEquals(Code.FORBIDDEN, Code.of("forbidden"));
        assertTrue(Code.FORBIDDEN.isKnown());
        assertEquals(403, Code.FORBIDDEN.httpStatus().orElseThrow());
        Code fresh = Code.of("brand_new_code");
        assertFalse(fresh.isKnown());
        assertEquals("brand_new_code", fresh.slug());
        assertEquals(Code.INTERNAL, Code.forStatus(502));
        assertEquals("http_418", Code.forStatus(418).slug());
        assertEquals(Code.UNPROCESSABLE, Code.of("unprocessable"));
        assertEquals(422, Code.UNPROCESSABLE.httpStatus().orElseThrow());
        assertEquals(Code.UNPROCESSABLE, Code.forStatus(422));
    }

    @Test
    void anEnvelopeAndPlainTextBothRead() {
        ApiException e = ApiException.of(raw(
                400,
                "{\"code\":\"brand_new_code\",\"error\":\"Something new.\",\"details\":[{\"type\":\"retry\",\"after_seconds\":3},{\"type\":\"future\",\"x\":1}]}",
                Map.of()));
        assertEquals("brand_new_code", e.code().slug());
        assertEquals(3, e.retryAfterSeconds().orElseThrow());
        assertInstanceOf(Detail.Unknown.class, e.details().get(1));
        assertEquals("api", e.kind());
        ApiException g = ApiException.of(raw(403, "RBAC: access denied", Map.of()));
        assertEquals(Code.FORBIDDEN, g.code());
        assertTrue(g.getMessage().contains("RBAC: access denied"), g.getMessage());
    }

    @Test
    void aPathParameterIsOneSegment() {
        assertEquals("a%2Fb%20c", Codegen.pathSegment("a/b c"));
        assertEquals("%C3%BCber-._~", Codegen.pathSegment("über-._~"));
        assertEquals(1, Codegen.V1);
    }

    @Test
    void aNamedProfileReadsOnlyItsVariables() {
        ConfigException e =
                assertThrows(ConfigException.class, () -> Client.fromEnv(Map.of("INORBIT_TOKEN", "t"), "ACME_CI"));
        assertTrue(e.getMessage().contains("INORBIT_ACME_CI_TOKEN"), e.getMessage());
        Client c = Client.fromEnv(Map.of("INORBIT_ACME_CI_TOKEN", "t"), "ACME_CI");
        assertEquals(URI.create("https://api.inorbit.hr"), c.baseUrl());
        ConfigException noScopes = assertThrows(
                ConfigException.class,
                () -> Client.fromEnv(Map.of("INORBIT_KEY_ID", "ak", "INORBIT_KEY_SECRET", "s"), ""));
        assertTrue(noScopes.getMessage().contains("INORBIT_SCOPES"), noScopes.getMessage());
    }

    @Test
    void onlyHttpsLeavesThisMachine() {
        assertThrows(
                ConfigException.class,
                () -> Client.builder()
                        .token("t")
                        .baseUrl("http://api.inorbit.hr")
                        .build());
        assertThrows(
                ConfigException.class,
                () -> Client.builder()
                        .token("t")
                        .baseUrl("https://api.inorbit.hr/v1")
                        .build());
        Client.builder().token("t").baseUrl("http://127.0.0.1:8080").build();
    }

    @Test
    void backoffStaysWithinItsCeiling() {
        for (int retry = 0; retry < 10; retry++) {
            long ms = Retry.backoff(retry).toMillis();
            assertTrue(ms >= 0 && ms <= Math.min(500L << Math.min(retry, 5), 8_000L), retry + ": " + ms);
        }
        Map<String, List<String>> h = Map.of("retry-after", List.of("120"));
        assertEquals(
                Duration.ofSeconds(60),
                Retry.retryAfter(HttpHeaders.of(h, (a, b) -> true)).orElseThrow());
        assertTrue(Retry.requestId().matches("iohr-[0-9a-f]{16}"));
    }

    @Test
    void int64TravelsAsADecimalString() throws Exception {
        ObjectMapper json = Json.MAPPER;
        Digest d = json.readValue("{\"id\":\"9007199254740993\",\"item_count\":2,\"future\":true}", Digest.class);
        assertEquals(9_007_199_254_740_993L, d.id());
        assertTrue(json.writeValueAsString(d).contains("\"id\":\"9007199254740993\""));
        assertEquals(9_007_199_254_740_993L, Int64.parse("9007199254740993"));
    }

    @Test
    void concurrentCallsShareOneExchange() throws Exception {
        AtomicInteger exchanges = new AtomicInteger();
        HttpServer server = HttpServer.create(new InetSocketAddress("127.0.0.1", 0), 0);
        server.createContext("/oauth2/token", ex -> {
            exchanges.incrementAndGet();
            try {
                Thread.sleep(200);
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
            }
            byte[] body = "{\"access_token\":\"t1\",\"expires_in\":899}".getBytes(StandardCharsets.UTF_8);
            ex.sendResponseHeaders(200, body.length);
            ex.getResponseBody().write(body);
            ex.close();
        });
        server.start();
        try {
            ClientCredentials cc = new ClientCredentials(
                    "ak_test",
                    "s3cr3t",
                    List.of("identity:read"),
                    URI.create("http://127.0.0.1:" + server.getAddress().getPort() + "/oauth2/token"),
                    HttpClient.newHttpClient());
            ExecutorService pool = Executors.newFixedThreadPool(10);
            List<Future<Token>> tokens = new ArrayList<>();
            for (int i = 0; i < 10; i++) {
                tokens.add(pool.submit(cc::token));
            }
            for (Future<Token> t : tokens) {
                assertEquals("t1", t.get().access());
            }
            pool.shutdown();
            assertEquals(1, exchanges.get());
            cc.invalidate();
            cc.token();
            assertEquals(2, exchanges.get());
        } finally {
            server.stop(0);
        }
    }
}
