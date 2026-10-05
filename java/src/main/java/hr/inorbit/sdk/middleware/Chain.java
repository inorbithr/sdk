package hr.inorbit.sdk.middleware;

/** The rest of the pipeline, as one middleware sees it. */
public interface Chain {

    /**
     * Runs the rest of the pipeline. From the per-call stage, each call is a fresh attempt.
     *
     * @param request the request to send on
     * @return the answer
     * @throws hr.inorbit.sdk.errors.InOrbitException when the rest fails
     */
    Response proceed(Request request);

    /**
     * The call's metadata, as the request this middleware got carries it.
     *
     * @return the metadata
     */
    CallInfo info();
}
