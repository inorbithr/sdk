package hr.inorbit.sdk.middleware;

import java.time.Instant;
import java.util.Optional;

/** What a middleware may read about the call it handles (docs/config.md section 7.13). */
public interface CallInfo {

    /**
     * The operation ({@code radar.list_digests}), or the path for a raw call.
     *
     * @return the operation
     */
    String operation();

    /**
     * Whether {@code retry} may repeat it without an idempotency key.
     *
     * @return whether the call is idempotent
     */
    boolean idempotent();

    /**
     * The {@code Idempotency-Key} the call sends, once {@code idempotency_key} has run.
     *
     * @return the key, for an operation that takes one
     */
    Optional<String> idempotencyKey();

    /**
     * The call's {@code x-request-id}, the same on every attempt.
     *
     * @return the request id
     */
    String requestId();

    /**
     * 1-based on each attempt; 0 in the per-call stage.
     *
     * @return the attempt
     */
    int attempt();

    /**
     * When the call must end, once {@code deadline} has run.
     *
     * @return the deadline
     */
    Optional<Instant> deadline();

    /**
     * Whether the answer is a stream, whose body a middleware must not read.
     *
     * @return whether it streams
     */
    boolean stream();

    /**
     * The configuration profile the client was loaded with.
     *
     * @return the profile, if one was chosen
     */
    Optional<String> profile();
}
