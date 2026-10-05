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
import java.nio.file.Files;
import java.nio.file.Path;
import java.time.Duration;
import java.time.Instant;
import java.util.Base64;
import java.util.List;
import java.util.Objects;
import java.util.concurrent.ThreadLocalRandom;
import java.util.function.BiConsumer;

/**
 * Exchanges an API key for 15-minute tokens (OAuth 2.0 client credentials), caches the token,
 * refreshes it when less than a fifth of its life is left or after a 401, and makes one exchange
 * however many threads wait for it. A refresh that fails while the cached token is still valid
 * keeps that token (docs/config.md section 5.3). A secret file is read before every exchange, so
 * a rotated Kubernetes Secret is used at the next refresh.
 */
public final class ClientCredentials implements TokenProvider, Observable {

    /** The audience every token for the API is asked for. */
    public static final String AUDIENCE = "iohr-api";

    /** Where an API key is exchanged for a token. */
    public static final String DEFAULT_TOKEN_URL = "https://auth.inorbit.hr/oauth2/token";

    private static final int TOKEN_RETRIES = 2;
    private static final Duration TOKEN_TIMEOUT = Duration.ofSeconds(30);
    private static final ObjectMapper JSON = new ObjectMapper();

    private final String keyId;
    private final String secret;
    private final Path secretFile;
    private final List<String> scopes;
    private final URI tokenUrl;
    private final HttpClient http;
    private final String userAgent;
    private final TokenCache cache = new TokenCache(this::exchange);

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
        this(keyId, Objects.requireNonNull(secret, "secret"), null, scopes, tokenUrl, http, null);
    }

    /**
     * A provider for one key whose secret is a file (or given), sending {@code userAgent}.
     *
     * @param keyId the key's id ({@code ak_...})
     * @param secret the key's secret, or {@code null} with {@code secretFile}; never log it
     * @param secretFile a file holding the secret, read before every exchange, or {@code null}
     * @param scopes the scopes to ask for, a subset of the key's
     * @param tokenUrl the token endpoint
     * @param http the HTTP client to use
     * @param userAgent the {@code User-Agent} to send, or {@code null} for the HTTP client's
     */
    public ClientCredentials(
            String keyId,
            String secret,
            Path secretFile,
            List<String> scopes,
            URI tokenUrl,
            HttpClient http,
            String userAgent) {
        this.keyId = Objects.requireNonNull(keyId, "keyId");
        if ((secret == null) == (secretFile == null)) {
            throw new IllegalArgumentException("give the secret or the secret file, one of them");
        }
        this.secret = secret;
        this.secretFile = secretFile;
        this.scopes = List.copyOf(scopes);
        this.tokenUrl = Objects.requireNonNull(tokenUrl, "tokenUrl");
        this.http = Objects.requireNonNull(http, "http");
        this.userAgent = userAgent;
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
        return cache.get();
    }

    /** Drops the cached token, so the next call exchanges again. */
    @Override
    public void invalidate() {
        cache.invalidate();
    }

    @Override
    public void listen(BiConsumer<String, String> listener) {
        cache.listen(listener);
    }

    private String secret() {
        if (secret != null) {
            return secret;
        }
        try {
            String s = Files.readString(secretFile, StandardCharsets.UTF_8).strip();
            if (s.isEmpty()) {
                throw new AuthException("the key secret file " + secretFile + " is empty", "", null);
            }
            return s;
        } catch (IOException | RuntimeException e) {
            if (e instanceof AuthException a) {
                throw a;
            }
            throw new AuthException("cannot read the key secret file " + secretFile, "", null);
        }
    }

    private Token exchange() {
        String form = "grant_type=client_credentials&audience=" + encode(AUDIENCE) + "&scope="
                + encode(String.join(" ", scopes));
        String basic = Base64.getEncoder()
                .encodeToString((encode(keyId) + ":" + encode(secret())).getBytes(StandardCharsets.UTF_8));
        HttpRequest.Builder rb = HttpRequest.newBuilder(tokenUrl)
                .timeout(TOKEN_TIMEOUT)
                .header("authorization", "Basic " + basic)
                .header("content-type", "application/x-www-form-urlencoded")
                .header("accept", "application/json");
        if (userAgent != null) {
            rb.header("user-agent", userAgent);
        }
        HttpRequest request = rb.POST(HttpRequest.BodyPublishers.ofString(form)).build();
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
            return new Token(
                    token, Instant.now().plusSeconds(body.path("expires_in").asLong(900)));
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
