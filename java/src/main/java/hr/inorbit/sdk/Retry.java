package hr.inorbit.sdk;

import java.net.http.HttpHeaders;
import java.security.SecureRandom;
import java.time.Duration;
import java.util.HexFormat;
import java.util.Optional;
import java.util.concurrent.ThreadLocalRandom;

/** The retry rules of design.md section 6, and the request id. */
final class Retry {

    /** Retry-After is honoured up to a minute. */
    static final Duration RETRY_AFTER_CAP = Duration.ofSeconds(60);

    private static final long BACKOFF_BASE_MS = 500;
    private static final long BACKOFF_CAP_MS = 8_000;
    private static final SecureRandom IDS = new SecureRandom();

    private Retry() {}

    /** Statuses worth another attempt on an idempotent call. */
    static boolean retryableStatus(int status) {
        return status == 429 || status == 503 || status == 504;
    }

    /** The wait {@code Retry-After} asks for, capped; seconds only. */
    static Optional<Duration> retryAfter(HttpHeaders headers) {
        return headers.firstValue("retry-after")
                .map(String::strip)
                .filter(v -> v.matches("\\d{1,9}"))
                .map(v -> Duration.ofSeconds(Long.parseLong(v)))
                .map(d -> d.compareTo(RETRY_AFTER_CAP) > 0 ? RETRY_AFTER_CAP : d);
    }

    /** Full jitter: a random wait up to 0.5 s doubled per retry, at most 8 s. */
    static Duration backoff(int retry) {
        long ceiling = Math.min(BACKOFF_BASE_MS << Math.min(retry, 5), BACKOFF_CAP_MS);
        // Jitter only spreads retries out; it is not a secret.
        return Duration.ofMillis(ThreadLocalRandom.current().nextLong(ceiling + 1));
    }

    /** Full jitter: a random wait up to {@code base} doubled per retry, at most {@code cap}. */
    static Duration backoff(int retry, Duration base, Duration cap) {
        long ceiling = Math.min(base.toMillis() << Math.min(retry, 20), cap.toMillis());
        return Duration.ofMillis(ThreadLocalRandom.current().nextLong(Math.max(ceiling, 0) + 1));
    }

    /**
     * The wait {@code Retry-After} asks for, as delay-seconds or an HTTP date (RFC 9110 section
     * 10.2.3); not capped.
     */
    static Optional<Duration> retryAfterUncapped(HttpHeaders headers) {
        Optional<String> raw = headers.firstValue("retry-after").map(String::strip);
        if (raw.isEmpty() || raw.get().isEmpty()) {
            return Optional.empty();
        }
        String v = raw.get();
        if (v.matches("\\d{1,9}")) {
            return Optional.of(Duration.ofSeconds(Long.parseLong(v)));
        }
        try {
            java.time.ZonedDateTime when =
                    java.time.ZonedDateTime.parse(v, java.time.format.DateTimeFormatter.RFC_1123_DATE_TIME);
            Duration d = Duration.between(java.time.Instant.now(), when.toInstant());
            return Optional.of(d.isNegative() ? Duration.ZERO : d);
        } catch (java.time.format.DateTimeParseException e) {
            return Optional.empty();
        }
    }

    /** {@code iohr-<16 hex>}, the id each call is sent with. */
    static String requestId() {
        byte[] bytes = new byte[8];
        IDS.nextBytes(bytes);
        return "iohr-" + HexFormat.of().formatHex(bytes);
    }
}
