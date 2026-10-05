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

    private volatile String requestId;
    private volatile String idempotencyKey;

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

    /**
     * The {@code x-request-id} the call was sent with, once the client knows it.
     *
     * @return the request id, if the error came from a call
     */
    public java.util.Optional<String> requestId() {
        return java.util.Optional.ofNullable(requestId);
    }

    /**
     * The {@code Idempotency-Key} the call sent (docs/config.md section 7.5): repeat the call
     * with the same key, and the API answers what the first one did.
     *
     * @return the key, if the operation takes one
     */
    public java.util.Optional<String> idempotencyKey() {
        return java.util.Optional.ofNullable(idempotencyKey);
    }

    /**
     * Records the call this error ended: its request id and idempotency key. The client calls
     * it once; a value already set is kept.
     *
     * @param requestId the request id, or {@code null}
     * @param idempotencyKey the idempotency key, or {@code null}
     * @return this error
     */
    public final InOrbitException attach(String requestId, String idempotencyKey) {
        if (this.requestId == null) {
            this.requestId = requestId;
        }
        if (this.idempotencyKey == null) {
            this.idempotencyKey = idempotencyKey;
        }
        return this;
    }
}
