package hr.inorbit.sdk.auth;

/**
 * Hands out a valid token for each attempt, and is told when the API refused the last one. Plug your
 * own in with {@code Client.builder().tokenProvider(...)}; it must be safe to call from several
 * threads.
 */
public interface TokenProvider {

    /**
     * A token for the next attempt.
     *
     * @return the token
     * @throws hr.inorbit.sdk.errors.AuthException when no token can be had
     */
    Token token();

    /** The API answered 401 with the last token: drop any cached one. The default does nothing. */
    default void invalidate() {}
}
