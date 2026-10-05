package hr.inorbit.sdk.auth;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import hr.inorbit.sdk.errors.AuthException;
import java.io.IOException;
import java.io.InputStream;
import java.nio.charset.StandardCharsets;
import java.time.Instant;
import java.time.format.DateTimeParseException;
import java.util.List;
import java.util.Objects;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.ExecutionException;
import java.util.concurrent.TimeUnit;
import java.util.function.BiConsumer;

/**
 * The {@code iohr} login (docs/config.md section 5.4): runs {@code iohr auth token --profile
 * <name> --format json}, without a shell, standard input closed, a 10 s limit and the inherited
 * environment. The token is cached by section 5.3's rules and the command runs again in the
 * refresh window. A failure is an {@link AuthException} carrying the first line of standard error;
 * standard output, which holds the token, is never quoted.
 */
public final class CliToken implements TokenProvider, Observable {

    private static final long LIMIT_SECONDS = 10;
    private static final ObjectMapper JSON = new ObjectMapper();

    private final String profile;
    private final String cliPath;
    private final TokenCache cache;

    /**
     * A provider for one {@code iohr} profile.
     *
     * @param profile the command line's profile name
     * @param cliPath the command line to run ({@code iohr} on {@code PATH} by default)
     */
    public CliToken(String profile, String cliPath) {
        this.profile = Objects.requireNonNull(profile, "profile");
        this.cliPath = Objects.requireNonNull(cliPath, "cliPath");
        this.cache = new TokenCache(this::fetch);
    }

    /**
     * A provider for one {@code iohr} profile, running {@code iohr} from {@code PATH}.
     *
     * @param profile the command line's profile name
     */
    public CliToken(String profile) {
        this(profile, "iohr");
    }

    @Override
    public Token token() {
        return cache.get();
    }

    /** Drops the cached token, so the next call runs {@code iohr} again. */
    @Override
    public void invalidate() {
        cache.invalidate();
    }

    @Override
    public void listen(BiConsumer<String, String> listener) {
        cache.listen(listener);
    }

    private Token fetch() {
        Process p;
        try {
            p = new ProcessBuilder(List.of(cliPath, "auth", "token", "--profile", profile, "--format", "json")).start();
        } catch (IOException | RuntimeException e) {
            throw new AuthException("cannot run " + cliPath + ": is iohr installed?", "", null);
        }
        try {
            p.getOutputStream().close();
        } catch (IOException e) {
            // Standard input is closed either way once the process ends.
        }
        CompletableFuture<byte[]> out = CompletableFuture.supplyAsync(() -> drain(p.getInputStream()));
        CompletableFuture<byte[]> err = CompletableFuture.supplyAsync(() -> drain(p.getErrorStream()));
        int code;
        try {
            if (!p.waitFor(LIMIT_SECONDS, TimeUnit.SECONDS)) {
                p.destroyForcibly();
                throw new AuthException("`iohr auth token` did not answer within " + LIMIT_SECONDS + " s", "", null);
            }
            code = p.exitValue();
            byte[] stdout = out.get(LIMIT_SECONDS, TimeUnit.SECONDS);
            byte[] stderr = err.get(LIMIT_SECONDS, TimeUnit.SECONDS);
            return answer(code, stdout, stderr);
        } catch (InterruptedException e) {
            p.destroyForcibly();
            Thread.currentThread().interrupt();
            throw new AuthException("`iohr auth token` was interrupted", "", e);
        } catch (ExecutionException | java.util.concurrent.TimeoutException e) {
            p.destroyForcibly();
            throw new AuthException("`iohr auth token` could not be read", "", null);
        }
    }

    private Token answer(int code, byte[] out, byte[] err) {
        if (code != 0) {
            String text = new String(err, StandardCharsets.UTF_8).strip();
            String line = text.isEmpty()
                    ? "exit status " + code
                    : text.lines().findFirst().orElse("");
            if (line.length() > 200) {
                line = line.substring(0, 200);
            }
            throw new AuthException("`iohr auth token` failed: " + line, "", null);
        }
        AuthException refused = new AuthException(cliPath + " answered something that is not a token", "", null);
        JsonNode answer;
        try {
            answer = JSON.readTree(out);
        } catch (IOException e) {
            throw refused;
        }
        if (answer == null
                || !answer.path("access_token").isTextual()
                || answer.path("access_token").asText().isEmpty()) {
            throw refused;
        }
        JsonNode expires = answer.get("expires_at");
        Instant at = null;
        if (expires != null && !expires.isNull()) {
            try {
                at = Instant.parse(expires.asText());
            } catch (DateTimeParseException e) {
                throw refused;
            }
        }
        return new Token(answer.path("access_token").asText(), at);
    }

    private static byte[] drain(InputStream in) {
        try (in) {
            return in.readNBytes(1024 * 1024);
        } catch (IOException e) {
            return new byte[0];
        }
    }

    /** The profile and program, never the token. */
    @Override
    public String toString() {
        return "CliToken(" + profile + ", " + cliPath + ")";
    }
}
