package hr.inorbit.sdk.errors;

/** The API could not be reached: DNS, TCP, TLS, or a reset before an answer. */
public final class ConnectionException extends InOrbitException {

    private static final long serialVersionUID = 1L;

    private final String host;

    /**
     * Cannot reach {@code host}, because of {@code reason}.
     *
     * @param host the unreachable host
     * @param reason what went wrong, without the request
     * @param cause the underlying failure, or {@code null}
     */
    public ConnectionException(String host, String reason, Throwable cause) {
        super("cannot reach " + host + ": " + reason, cause);
        this.host = host;
    }

    /**
     * The unreachable host.
     *
     * @return the host
     */
    public String host() {
        return host;
    }

    @Override
    public String kind() {
        return "connection";
    }
}
