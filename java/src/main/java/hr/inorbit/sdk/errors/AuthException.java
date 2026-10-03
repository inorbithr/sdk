package hr.inorbit.sdk.errors;

/** The token exchange, or a custom token provider, failed. The message never holds the secret. */
public final class AuthException extends InOrbitException {

    private static final long serialVersionUID = 1L;

    private final String error;

    /**
     * An auth failure.
     *
     * @param message what failed
     * @param error the token endpoint's {@code error}, or {@code HTTP <status>}; empty for a
     *     transport failure
     * @param cause the underlying failure, or {@code null}
     */
    public AuthException(String message, String error, Throwable cause) {
        super(message, cause);
        this.error = error;
    }

    /**
     * The token endpoint's {@code error} (such as {@code invalid_client}), or {@code HTTP <status>};
     * empty for a transport failure.
     *
     * @return the error
     */
    public String error() {
        return error;
    }

    @Override
    public String kind() {
        return "auth";
    }
}
