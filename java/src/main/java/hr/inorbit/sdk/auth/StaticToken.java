package hr.inorbit.sdk.auth;

import java.util.Objects;

/** A token you already hold, such as an API token from the console or {@code iohr token create}. */
public final class StaticToken implements TokenProvider {

    private final String token;

    /**
     * A provider that always hands out {@code token}.
     *
     * @param token the token
     */
    public StaticToken(String token) {
        this.token = Objects.requireNonNull(token, "token");
    }

    @Override
    public Token token() {
        return new Token(token, null);
    }

    /** Never the token. */
    @Override
    public String toString() {
        return "StaticToken(<redacted>)";
    }
}
