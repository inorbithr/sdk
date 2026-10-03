package hr.inorbit.sdk.errors;

/** The answer is larger than 16 MiB, which the SDK refuses to read. */
public final class TooLargeException extends InOrbitException {

    private static final long serialVersionUID = 1L;

    /** An answer over the cap. */
    public TooLargeException() {
        super("the answer is larger than 16 MiB; refusing to read it", null);
    }

    @Override
    public String kind() {
        return "too_large";
    }
}
