package hr.inorbit.sdk.auth;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import hr.inorbit.sdk.errors.AuthException;
import java.io.IOException;
import java.net.URI;
import java.net.URLEncoder;
import java.net.http.HttpClient;
import java.net.http.HttpRequest;
import java.net.http.HttpResponse;
import java.nio.charset.StandardCharsets;
import java.time.Duration;
import java.time.Instant;
import java.util.Base64;
import java.util.List;
import java.util.Objects;
import java.util.concurrent.ThreadLocalRandom;

/**
 * Exchanges an API key for 15-minute tokens (OAuth 2.0 client credentials), caches the token,
 * refreshes it when less than a fifth of its life is left or after a 401, and makes one exchange
 * however many threads wait for it.
 */
public final class ClientCredentials implements TokenProvider {

    /** The audience every token for the API is asked for. */
    public static final String AUDIENCE = "iohr-api";

    /** Where an API key is exchanged for a token. */
    public static final String DEFAULT_TOKEN_URL = "https://auth.inorbit.hr/oauth2/token";

    private static final int TOKEN_RETRIES = 2;
    private static final Duration TOKEN_TIMEOUT = Duration.ofSeconds(30);
    private static final ObjectMapper JSON = new ObjectMapper();

    private final String keyId;
    private final String secret;
    private final List<String> scopes;
    private final URI tokenUrl;
    private final HttpClient http;

    private final Object lock = new Object();
    private String access;
    private Instant issued;
    private Duration lifetime;

    /**
     * A provider for one key.
     *
     * @param keyId the key's id ({@code ak_...})
     * @param secret the key's secret; never log it
     * @param scopes the scopes to ask for, a subset of the key's
     * @param tokenUrl the token endpoint
     * @param http the HTTP client to use
     */
    public ClientCredentials(String keyId, String secret, List<String> scopes, URI tokenUrl, HttpClient http) {
        this.keyId = Objects.requireNonNull(keyId, "keyId");
        this.secret = Objects.requireNonNull(secret, "secret");
        this.scopes = List.copyOf(scopes);
        this.tokenUrl = Objects.requireNonNull(tokenUrl, "tokenUrl");
        this.http = Objects.requireNonNull(http, "http");
    }

    /**
     * The key id this provider exchanges.
     *
     * @return the key id
     */
    public String keyId() {
        return keyId;
    }

    /** A cached token while four fifths of its life are left, else a fresh one (one exchange). */
    @Override
    public Token token() {
        synchronized (lock) {
            Instant now = Instant.now();
            if (access == null
                    || Duration.between(issued, now)
                                    .compareTo(lifetime.multipliedBy(4).dividedBy(5))
                            >= 0) {
                exchange();
            }
            return new Token(access, issued.plus(lifetime));
        }
    }

    /** Drops the cached token, so the next call exchanges again. */
    @Override
    public void invalidate() {
        synchronized (lock) {
            access = null;
        }
    }

    private void exchange() {
        String form = "grant_type=client_credentials&audience=" + encode(AUDIENCE) + "&scope="
                + encode(String.join(" ", scopes));
        String basic = Base64.getEncoder()
                .encodeToString((encode(keyId) + ":" + encode(secret)).getBytes(StandardCharsets.UTF_8));
        HttpRequest request = HttpRequest.newBuilder(tokenUrl)
                .timeout(TOKEN_TIMEOUT)
                .header("authorization", "Basic " + basic)
                .header("content-type", "application/x-www-form-urlencoded")
                .header("accept", "application/json")
                .POST(HttpRequest.BodyPublishers.ofString(form))
                .build();
        for (int attempt = 0; ; attempt++) {
            HttpResponse<String> resp;
            try {
                resp = http.send(request, HttpResponse.BodyHandlers.ofString());
            } catch (IOException e) {
                if (attempt < TOKEN_RETRIES) {
                    pause(attempt);
                    continue;
                }
                throw new AuthException("the token endpoint: " + describe(e), "", e);
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
                throw new AuthException("the token endpoint: interrupted", "", e);
            }
            int status = resp.statusCode();
            if ((status == 429 || status == 503 || status == 504) && attempt < TOKEN_RETRIES) {
                pause(attempt);
                continue;
            }
            JsonNode body = parse(resp.body());
            if (status < 200 || status >= 300) {
                String error = body.path("error").asText("");
                if (error.isEmpty()) {
                    error = "HTTP " + status;
                }
                String description = body.path("error_description").asText("");
                throw new AuthException(
                        "the token exchange for key " + keyId + " failed: " + error
                                + (description.isEmpty() ? "" : " (" + description + ")"),
                        error,
                        null);
            }
            String token = body.path("access_token").asText("");
            if (token.isEmpty()) {
                throw new AuthException("the token endpoint: the token answer could not be read", "", null);
            }
            access = token;
            issued = Instant.now();
            lifetime = Duration.ofSeconds(body.path("expires_in").asLong(900));
            return;
        }
    }

    private static void pause(int attempt) {
        long ceiling = Math.min(500L << attempt, 8_000L);
        try {
            Thread.sleep(ThreadLocalRandom.current().nextLong(ceiling + 1));
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            throw new AuthException("the token endpoint: interrupted", "", e);
        }
    }

    private static JsonNode parse(String text) {
        try {
            JsonNode node = JSON.readTree(text);
            return node == null ? JSON.createObjectNode() : node;
        } catch (IOException e) {
            return JSON.createObjectNode();
        }
    }

    private static String encode(String s) {
        return URLEncoder.encode(s, StandardCharsets.UTF_8);
    }

    private static String describe(Throwable e) {
        String m = e.getMessage();
        return m == null || m.isEmpty() ? e.getClass().getSimpleName() : m;
    }

    /** Never the secret. */
    @Override
    public String toString() {
        return "ClientCredentials(" + keyId + ", secret: <redacted>)";
    }
}
