package hr.inorbit.sdk.errors;

/**
 * The base of every error the SDK throws. {@link #kind()} and the subclass tell them apart; the
 * message says what failed and never holds a secret.
 */
public abstract sealed class InOrbitException extends RuntimeException
        permits ApiException,
                ConnectionException,
                TimeoutException,
                AuthException,
                ConfigException,
                TooLargeException,
                DecodeException {

    private static final long serialVersionUID = 1L;

    /**
     * An error with {@code message}.
     *
     * @param message what failed
     * @param cause the underlying failure, or {@code null}
     */
    protected InOrbitException(String message, Throwable cause) {
        super(message, cause);
    }

    /**
     * A stable kind: {@code api}, {@code connection}, {@code timeout}, {@code auth}, {@code config},
     * {@code too_large} or {@code decode}.
     *
     * @return the kind
     */
    public abstract String kind();
}
