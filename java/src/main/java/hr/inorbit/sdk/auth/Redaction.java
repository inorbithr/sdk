package hr.inorbit.sdk.auth;

/** What every printer in this package shows in place of a secret. */
final class Redaction {

    /** The marker: the same text {@code describe()} and every other SDK show. */
    static final String MARKER = "<redacted>";

    private Redaction() {}
}
