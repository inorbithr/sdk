package hr.inorbit.sdk.errors;

/** The client was configured in a way it cannot work with; the message says what to set. */
public final class ConfigException extends InOrbitException {

    private static final long serialVersionUID = 1L;

    /**
     * A configuration problem.
     *
     * @param message what is wrong and what to do
     */
    public ConfigException(String message) {
        super(message, null);
    }

    @Override
    public String kind() {
        return "config";
    }
}
