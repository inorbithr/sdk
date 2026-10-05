package hr.inorbit.sdk.auth;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import hr.inorbit.sdk.errors.AuthException;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.attribute.BasicFileAttributes;
import java.time.Duration;
import java.time.Instant;
import java.util.Base64;
import java.util.Objects;

/**
 * A bearer token read from a file, such as a mounted Kubernetes Secret (docs/config.md section
 * 5.3). The file is read at first use, then again when its modification time or size changes
 * (checked at most once a minute), and right after the API refuses the token. A JWT's {@code exp}
 * claim, read and not verified, is the token's expiry. A file that disappears keeps the cached
 * token until the API refuses it.
 */
public final class TokenFile implements TokenProvider {

    private static final Duration CHECK = Duration.ofSeconds(60);
    private static final ObjectMapper JSON = new ObjectMapper();

    private final Path path;
    private final Object lock = new Object();
    private Token token;
    private Object stamp;
    private long checked;
    private boolean forced = true;

    /**
     * A provider reading {@code path}.
     *
     * @param path the file holding the token
     */
    public TokenFile(Path path) {
        this.path = Objects.requireNonNull(path, "path");
    }

    /**
     * The file this provider reads.
     *
     * @return the path
     */
    public Path path() {
        return path;
    }

    /**
     * The file's token, read again when it changed.
     *
     * @throws AuthException the file cannot be read and no token is cached, or the API refused
     *     the cached one
     */
    @Override
    public Token token() {
        synchronized (lock) {
            long now = System.nanoTime();
            Token t = token;
            boolean expired = t != null
                    && t.expiresAt().map(x -> !Instant.now().isBefore(x)).orElse(false);
            if (t != null && !forced && !expired && now - checked < CHECK.toNanos()) {
                return t;
            }
            checked = now;
            Object st;
            String access;
            try {
                BasicFileAttributes a = Files.readAttributes(path, BasicFileAttributes.class);
                st = java.util.List.of(a.lastModifiedTime().toMillis(), a.size());
                if (t != null && !forced && st.equals(stamp) && !expired) {
                    return t;
                }
                access = Files.readString(path, StandardCharsets.UTF_8).strip();
            } catch (IOException | RuntimeException e) {
                if (t != null && !forced) {
                    return t;
                }
                throw new AuthException("cannot read the token file " + path, "", null);
            }
            if (access.isEmpty()) {
                throw new AuthException("the token file " + path + " is empty", "", null);
            }
            token = new Token(access, jwtExpiry(access));
            stamp = st;
            forced = false;
            return token;
        }
    }

    /** The API refused the token: read the file again at the next call. */
    @Override
    public void invalidate() {
        synchronized (lock) {
            forced = true;
        }
    }

    /**
     * A JWT's {@code exp} claim, read and not verified; {@code null} for anything else.
     *
     * @param token the token
     * @return the expiry, or {@code null}
     */
    static Instant jwtExpiry(String token) {
        String[] parts = token.split("\\.", -1);
        if (parts.length != 3) {
            return null;
        }
        try {
            JsonNode claims = JSON.readTree(Base64.getUrlDecoder().decode(parts[1]));
            JsonNode exp = claims == null ? null : claims.get("exp");
            return exp != null && exp.isNumber() ? Instant.ofEpochSecond(exp.asLong()) : null;
        } catch (IOException | IllegalArgumentException e) {
            return null;
        }
    }

    /** The path, never the token. */
    @Override
    public String toString() {
        return "TokenFile(" + path + ")";
    }
}
