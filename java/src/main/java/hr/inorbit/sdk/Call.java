package hr.inorbit.sdk;

import hr.inorbit.sdk.middleware.CallInfo;
import hr.inorbit.sdk.middleware.Request;
import java.time.Duration;
import java.time.Instant;
import java.util.ArrayList;
import java.util.List;
import java.util.Optional;

/** One call's state, shared by every built-in across its attempts. Not for middlewares. */
final class Call {

    final Operation op;
    final String template;
    final Duration timeout;
    final String traceparent;
    final long started = System.nanoTime();
    final Instant startedAt = Instant.now();
    volatile String idempotencyKey;
    volatile int attempts;
    volatile boolean refreshed;
    volatile Duration attemptTimeout;
    volatile RateLimit rateLimit;
    volatile Hook.Attempt lastAttempt;
    volatile Object callSpan;
    volatile Object attemptSpan;
    volatile String serverRequestId;
    final List<Runnable> onClose = new ArrayList<>();

    Call(Operation op, String template, Duration timeout, String traceparent, String idempotencyKey) {
        this.op = op;
        this.template = template;
        this.timeout = timeout;
        this.traceparent = traceparent;
        this.idempotencyKey = idempotencyKey;
    }

    /** The state of the call {@code req} belongs to, or {@code null} for a request made elsewhere. */
    static Call of(Request req) {
        return req.info() instanceof Info i ? i.call : null;
    }

    /** The {@link CallInfo} the SDK gives a request: one value per attempt, the call's state behind it. */
    static final class Info implements CallInfo {
        final Call call;
        private final String operation;
        private final boolean idempotent;
        private final String idempotencyKey;
        private final String requestId;
        private final int attempt;
        private final long deadline;
        private final boolean stream;
        private final String profile;

        Info(
                Call call,
                String operation,
                boolean idempotent,
                String idempotencyKey,
                String requestId,
                int attempt,
                long deadline,
                boolean stream,
                String profile) {
            this.call = call;
            this.operation = operation;
            this.idempotent = idempotent;
            this.idempotencyKey = idempotencyKey;
            this.requestId = requestId;
            this.attempt = attempt;
            this.deadline = deadline;
            this.stream = stream;
            this.profile = profile;
        }

        Info withAttempt(int n) {
            return new Info(call, operation, idempotent, idempotencyKey, requestId, n, deadline, stream, profile);
        }

        Info withKey(String key) {
            return new Info(call, operation, idempotent, key, requestId, attempt, deadline, stream, profile);
        }

        Info withDeadline(long nanos) {
            return new Info(call, operation, idempotent, idempotencyKey, requestId, attempt, nanos, stream, profile);
        }

        /** The deadline in {@link System#nanoTime()} terms, or {@code Long.MIN_VALUE} for none. */
        long deadlineNanos() {
            return deadline;
        }

        @Override
        public String operation() {
            return operation;
        }

        @Override
        public boolean idempotent() {
            return idempotent;
        }

        @Override
        public Optional<String> idempotencyKey() {
            return Optional.ofNullable(idempotencyKey);
        }

        @Override
        public String requestId() {
            return requestId;
        }

        @Override
        public int attempt() {
            return attempt;
        }

        @Override
        public Optional<Instant> deadline() {
            if (deadline == Long.MIN_VALUE) {
                return Optional.empty();
            }
            return Optional.of(call.startedAt.plusNanos(deadline - call.started));
        }

        @Override
        public boolean stream() {
            return stream;
        }

        @Override
        public Optional<String> profile() {
            return Optional.ofNullable(profile);
        }

        @Override
        public String toString() {
            return "CallInfo[" + operation + ", attempt " + attempt + ", " + requestId + "]";
        }
    }
}
