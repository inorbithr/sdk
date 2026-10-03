package hr.inorbit.sdk;

import hr.inorbit.sdk.errors.InOrbitException;

/** Observes calls: logging, metrics, tracing. Every method has an empty default. */
public interface Hook {

    /**
     * Before an attempt is sent.
     *
     * @param attempt the attempt
     */
    default void onRequest(Attempt attempt) {}

    /**
     * After an answer arrived, whatever its status.
     *
     * @param attempt the attempt
     * @param response the answer
     */
    default void onResponse(Attempt attempt, RawResponse response) {}

    /**
     * When the call fails for good.
     *
     * @param attempt the last attempt
     * @param error why it failed
     */
    default void onError(Attempt attempt, InOrbitException error) {}

    /**
     * One attempt of a call, as hooks see it; never a header or a body.
     *
     * @param operation the operation ({@code radar.list_digests}), or the path for a raw call
     * @param method the HTTP method
     * @param path the path, parameters bound
     * @param number 1 for the first attempt
     * @param requestId the {@code x-request-id} sent
     */
    record Attempt(String operation, Method method, String path, int number, String requestId) {}
}
