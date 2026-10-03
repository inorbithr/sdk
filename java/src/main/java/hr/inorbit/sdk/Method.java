package hr.inorbit.sdk;

/** An HTTP method. */
public enum Method {
    /** {@code GET}. */
    GET,
    /** {@code POST}. */
    POST,
    /** {@code PUT}. */
    PUT,
    /** {@code PATCH}. */
    PATCH,
    /** {@code DELETE}. */
    DELETE,
    /** {@code HEAD}. */
    HEAD;

    /**
     * Whether calling it twice has the effect of calling it once, so a failed attempt may be
     * repeated.
     *
     * @return {@code true} for GET, PUT, DELETE and HEAD
     */
    public boolean isIdempotent() {
        return this == GET || this == PUT || this == DELETE || this == HEAD;
    }
}
