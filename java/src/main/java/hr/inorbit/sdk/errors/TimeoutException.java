package hr.inorbit.sdk.errors;

/**
 * An attempt took longer than the client allows. Not {@link java.util.concurrent.TimeoutException}:
 * this one is unchecked and part of the {@link InOrbitException} tree.
 */
public final class TimeoutException extends InOrbitException {

    private static final long serialVersionUID = 1L;

    private final String host;

    /**
     * {@code host} did not answer within {@code seconds}.
     *
     * @param host the host
     * @param seconds the limit, in seconds
     * @param cause the underlying failure, or {@code null}
     */
    public TimeoutException(String host, long seconds, Throwable cause) {
        super(host + " did not answer within " + seconds + " s", cause);
        this.host = host;
    }

    /**
     * A limit passed for another reason than a silent host, which {@code message} says.
     *
     * @param host the host
     * @param message what limit passed, and while waiting for what
     * @param cause the underlying failure, or {@code null}
     */
    public TimeoutException(String host, String message, Throwable cause) {
        super(message, cause);
        this.host = host;
    }

    /**
     * The host that did not answer.
     *
     * @return the host
     */
    public String host() {
        return host;
    }

    @Override
    public String kind() {
        return "timeout";
    }
}
