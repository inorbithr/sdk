package hr.inorbit.sdk.errors;

import hr.inorbit.sdk.RawResponse;

/** The API answered something this version of the SDK cannot read. */
public final class DecodeException extends InOrbitException {

    private static final long serialVersionUID = 1L;

    private final transient RawResponse raw;

    /**
     * An answer that does not read.
     *
     * @param reason why it does not read
     * @param raw the answer as it came
     * @param cause the underlying failure, or {@code null}
     */
    public DecodeException(String reason, RawResponse raw, Throwable cause) {
        super("the API answered something this version of hr.inorbit:inorbit-sdk cannot read: " + reason, cause);
        this.raw = raw;
    }

    /**
     * The answer as it came.
     *
     * @return the raw answer
     */
    public RawResponse raw() {
        return raw;
    }

    @Override
    public String kind() {
        return "decode";
    }
}
