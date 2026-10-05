package hr.inorbit.sdk;

import hr.inorbit.sdk.errors.InOrbitException;
import java.time.Duration;

/**
 * Observes calls: logging, metrics, tracing. Every method has an empty default. The {@code
 * hooks} middleware runs them on every attempt (docs/config.md section 7.7); a hook observes, and
 * one that needs to change a request is a middleware.
 */
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
     * Before the wait of each retry.
     *
     * @param attempt the attempt that is retried
     * @param reason why: the status ({@code 503}) or the error's kind ({@code timeout})
     * @param delay how long the client waits before the next attempt
     */
    default void onRetry(Attempt attempt, String reason, Duration delay) {}

    /**
     * One attempt of a call, as hooks see it; never a header or a body.
     *
     * @param operation the operation ({@code radar.list_digests}), or the path for a raw call
     * @param method the HTTP method
     * @param path the path, parameters bound
     * @param number 1 for the first attempt
     * @param requestId the {@code x-request-id} sent
     * @param idempotencyKey the {@code Idempotency-Key} sent, or {@code null}
     * @param stage {@code per_retry} for an attempt; {@code per_call} for a call's last word
     */
    record Attempt(
            String operation,
            Method method,
            String path,
            int number,
            String requestId,
            String idempotencyKey,
            String stage) {

        /**
         * An attempt without an idempotency key, in the per-retry stage: the form before M6.
         *
         * @param operation the operation
         * @param method the HTTP method
         * @param path the path
         * @param number 1 for the first attempt
         * @param requestId the {@code x-request-id} sent
         */
        public Attempt(String operation, Method method, String path, int number, String requestId) {
            this(operation, method, path, number, requestId, null, "per_retry");
        }
    }
}
