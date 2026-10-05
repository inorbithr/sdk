package hr.inorbit.sdk;

import java.time.Duration;
import java.util.Objects;

/**
 * Options for the calls of one {@link Client#withOptions} view (docs/config.md section 3.5,
 * per-call options): a timeout that replaces {@code timeout} and shortens {@code total_timeout},
 * the {@code Idempotency-Key} to send, and the caller's {@code traceparent}.
 *
 * <pre>{@code
 * Public api = new Public(client.withOptions(CallOptions.builder().idempotencyKey("order-42").build()));
 * api.events().createEndpoint(body);
 * }</pre>
 */
public final class CallOptions {

    /** No options: the client's settings. */
    public static final CallOptions NONE = builder().build();

    private final Duration timeout;
    private final String idempotencyKey;
    private final String traceparent;

    private CallOptions(Builder b) {
        this.timeout = b.timeout;
        this.idempotencyKey = b.idempotencyKey;
        this.traceparent = b.traceparent;
    }

    /**
     * A builder.
     *
     * @return the builder
     */
    public static Builder builder() {
        return new Builder();
    }

    /**
     * Each attempt's limit, also capping the call's total.
     *
     * @return the timeout, or {@code null}
     */
    public Duration timeout() {
        return timeout;
    }

    /**
     * The key to send on an operation that takes one.
     *
     * @return the key, or {@code null}
     */
    public String idempotencyKey() {
        return idempotencyKey;
    }

    /**
     * The W3C {@code traceparent} to continue.
     *
     * @return the header value, or {@code null}
     */
    public String traceparent() {
        return traceparent;
    }

    @Override
    public String toString() {
        return "CallOptions[timeout=" + timeout + ", idempotencyKey=" + (idempotencyKey != null) + ", traceparent="
                + (traceparent != null) + "]";
    }

    /** Builds {@link CallOptions}. */
    public static final class Builder {
        private Duration timeout;
        private String idempotencyKey;
        private String traceparent;

        private Builder() {}

        /**
         * Replaces {@code timeout} for each attempt and shortens {@code total_timeout}; it never
         * extends it.
         *
         * @param timeout the limit, above zero
         * @return this builder
         */
        public Builder timeout(Duration timeout) {
            if (timeout.isZero() || timeout.isNegative()) {
                throw new IllegalArgumentException("the timeout must be positive");
            }
            this.timeout = timeout;
            return this;
        }

        /**
         * The {@code Idempotency-Key} to send (docs/config.md section 7.5); only an operation
         * that takes one accepts it, any other call fails with a {@link
         * hr.inorbit.sdk.errors.ConfigException}.
         *
         * @param key the key
         * @return this builder
         */
        public Builder idempotencyKey(String key) {
            this.idempotencyKey = Objects.requireNonNull(key, "key");
            return this;
        }

        /**
         * A W3C {@code traceparent} to continue: the parent of the call's span with tracing on,
         * sent unchanged with tracing off.
         *
         * @param traceparent the header value
         * @return this builder
         */
        public Builder traceparent(String traceparent) {
            this.traceparent = Objects.requireNonNull(traceparent, "traceparent");
            return this;
        }

        /**
         * The options.
         *
         * @return the options
         */
        public CallOptions build() {
            return new CallOptions(this);
        }
    }
}
