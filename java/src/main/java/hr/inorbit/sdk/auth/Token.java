package hr.inorbit.sdk.auth;

import com.fasterxml.jackson.annotation.JsonValue;
import java.time.Instant;
import java.util.LinkedHashMap;
import java.util.Map;
import java.util.Objects;
import java.util.Optional;

/**
 * A bearer token and, when known, when it stops working. Neither {@link #toString()} nor its JSON
 * form shows it.
 */
public final class Token {

    private final transient String access;
    private final Instant expiresAt;

    /**
     * A token.
     *
     * @param access the token; never log it
     * @param expiresAt when it expires, or {@code null} when the provider does not know
     */
    public Token(String access, Instant expiresAt) {
        this.access = Objects.requireNonNull(access, "access");
        this.expiresAt = expiresAt;
    }

    /**
     * The token. Never log it.
     *
     * @return the token
     */
    public String access() {
        return access;
    }

    /**
     * When it expires.
     *
     * @return the expiry, if the provider knows it
     */
    public Optional<Instant> expiresAt() {
        return Optional.ofNullable(expiresAt);
    }

    /** Never the token. */
    @Override
    public String toString() {
        return "Token[" + Redaction.MARKER + "]";
    }

    /** The JSON form: the marker in place of the token, and when it expires when known. */
    @JsonValue
    private Map<String, Object> json() {
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("access", Redaction.MARKER);
        if (expiresAt != null) {
            m.put("expires_at", expiresAt.toString());
        }
        return m;
    }
}
