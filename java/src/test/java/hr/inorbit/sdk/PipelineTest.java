package hr.inorbit.sdk;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

import hr.inorbit.sdk.auth.TokenFile;
import hr.inorbit.sdk.errors.ConfigException;
import hr.inorbit.sdk.middleware.Middleware;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Base64;
import java.util.List;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

/** The pipeline's edits, describe() and the token file, without a server (docs/config.md). */
class PipelineTest {

    private static final Middleware PROBE = Middleware.of("probe", (r, c) -> c.proceed(r));

    private static Client.Builder base() {
        return Client.builder().token("t");
    }

    private static List<String> names(Client c) {
        List<String> out = new ArrayList<>();
        c.config().describe().path("pipeline").forEach(n -> out.add(n.asText()));
        return out;
    }

    @Test
    void editsGoWhereTheirNamesSay() {
        Client c = base().pipeline(p -> p.addPerCall(Middleware.of("call", (r, n) -> n.proceed(r)))
                        .addPerRetry(PROBE)
                        .insertBefore("auth", Middleware.of("before-auth", (r, n) -> n.proceed(r)))
                        .insertAfter("logging", Middleware.of("after-logging", (r, n) -> n.proceed(r)))
                        .replace("rate_limit", Middleware.of("ignored-name", (r, n) -> n.proceed(r)))
                        .remove("request_id"))
                .build();
        assertEquals(
                List.of(
                        "user_agent",
                        "idempotency_key",
                        "call_tracing",
                        "deadline",
                        "call",
                        "retry",
                        "before-auth",
                        "auth",
                        "rate_limit",
                        "attempt_tracing",
                        "logging",
                        "after-logging",
                        "hooks",
                        "probe",
                        "timeout"),
                names(c));
    }

    @Test
    void retryAuthAndTimeoutCannotBeRemovedAndNamesAreUnique() {
        for (String n : List.of("retry", "auth", "timeout")) {
            assertThrows(
                    ConfigException.class,
                    () -> base().pipeline(p -> p.remove(n)).build());
        }
        assertThrows(
                ConfigException.class,
                () -> base().pipeline(p -> p.addPerRetry(PROBE).addPerCall(PROBE))
                        .build());
        assertThrows(
                ConfigException.class,
                () -> base().pipeline(p -> p.remove("nope")).build());
    }

    @Test
    void explicitConstructionReadsNoEnvironmentAndDescribesItself() {
        Client c = base().timeout(java.time.Duration.ofSeconds(5)).build();
        var doc = c.config().describe();
        assertEquals("5s", doc.path("settings").path("timeout").path("value").asText());
        assertEquals("code", doc.path("settings").path("timeout").path("source").asText());
        assertEquals(
                "120s", doc.path("settings").path("total_timeout").path("value").asText());
        assertEquals(Config.PIPELINE, names(c));
        assertFalse(doc.toString().contains("\"t\""), doc.toString());
    }

    @Test
    void aTokenFileIsReadWithItsJwtExpiry(@TempDir Path dir) throws Exception {
        String payload = Base64.getUrlEncoder().withoutPadding().encodeToString("{\"exp\":4102444800}".getBytes());
        Path f = dir.resolve("tok");
        Files.writeString(f, "h." + payload + ".s\n");
        TokenFile tf = new TokenFile(f);
        assertEquals(4102444800L, tf.token().expiresAt().orElseThrow().getEpochSecond());
        assertFalse(tf.toString().contains(payload));
        Files.delete(f);
        assertTrue(tf.token().access().startsWith("h."), "a vanished file keeps the cached token");
    }

    @Test
    void aCallerKeyOnAnOperationWithoutOneIsAnError() {
        Client c = base().baseUrl("http://127.0.0.1:9").maxRetries(0).build();
        Client keyed = c.withOptions(CallOptions.builder().idempotencyKey("k").build());
        assertThrows(
                ConfigException.class,
                () -> keyed.send(
                        Operation.builder(Method.GET, "/v1/me").name("me").build()));
        assertTrue(Client.userAgent(null).matches("inorbithr-sdk-java/\\S+ java/\\S+ [a-z]+/[a-z0-9_]+"));
    }
}
