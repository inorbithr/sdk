package hr.inorbit.sdk;

import java.net.http.HttpHeaders;
import java.time.Duration;
import java.util.HashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Optional;

/**
 * What the API said about this credential's rate limit on one answer (docs/config.md section
 * 7.8): the edge's {@code X-RateLimit-*} headers, or the IETF {@code RateLimit} and {@code
 * RateLimit-Policy} fields, which win when both are sent.
 */
public final class RateLimit {

    private final Long limit;
    private final Long remaining;
    private final Duration reset;
    private final Policy policy;
    private final long observedAt;

    RateLimit(Long limit, Long remaining, Duration reset, Policy policy) {
        this.limit = limit;
        this.remaining = remaining;
        this.reset = reset;
        this.policy = policy;
        this.observedAt = System.nanoTime();
    }

    /**
     * Requests allowed per window.
     *
     * @return the limit, if the API said
     */
    public Optional<Long> limit() {
        return Optional.ofNullable(limit);
    }

    /**
     * Requests left in the current window.
     *
     * @return what remains, if the API said
     */
    public Optional<Long> remaining() {
        return Optional.ofNullable(remaining);
    }

    /**
     * Time until the window resets, from when the answer arrived.
     *
     * @return the time, if the API said
     */
    public Optional<Duration> reset() {
        return Optional.ofNullable(reset);
    }

    /**
     * The IETF policy, when the API sent one.
     *
     * @return the policy
     */
    public Optional<Policy> policy() {
        return Optional.ofNullable(policy);
    }

    /** How long until the window resets when nothing remains; zero otherwise. */
    Duration waitNeeded() {
        if (remaining == null || remaining != 0 || reset == null) {
            return Duration.ZERO;
        }
        Duration left = reset.minusNanos(System.nanoTime() - observedAt);
        return left.isNegative() ? Duration.ZERO : left;
    }

    @Override
    public String toString() {
        return "RateLimit[limit=" + limit + ", remaining=" + remaining + ", reset=" + reset + ", policy=" + policy
                + "]";
    }

    /**
     * The quota policy the IETF {@code RateLimit-Policy} field names.
     *
     * @param name the policy's name
     * @param quota requests the policy allows per window ({@code q}), or {@code null}
     * @param window the window ({@code w}), or {@code null}
     */
    public record Policy(String name, Long quota, Duration window) {}

    /**
     * The snapshot an answer's headers give, or {@code null} when they give none; a malformed
     * value is ignored, never an error.
     *
     * @param headers the answer's headers
     * @return the snapshot, or {@code null}
     */
    static RateLimit parse(HttpHeaders headers) {
        Map<String, String> h = new HashMap<>();
        headers.map().forEach((k, v) -> {
            if (!v.isEmpty()) {
                h.put(k.toLowerCase(Locale.ROOT), v.get(0));
            }
        });
        return parse(h);
    }

    /**
     * The snapshot from headers by lower-case name.
     *
     * @param h the headers, names in lower case
     * @return the snapshot, or {@code null}
     */
    static RateLimit parse(Map<String, String> h) {
        if (h.containsKey("ratelimit")) {
            Item item = item(h.get("ratelimit"));
            if (item != null) {
                Long remaining = number(item.params.get("r"));
                Long t = number(item.params.get("t"));
                Policy policy = null;
                if (h.containsKey("ratelimit-policy")) {
                    Item p = item(h.get("ratelimit-policy"));
                    if (p != null) {
                        Long w = number(p.params.get("w"));
                        policy =
                                new Policy(p.name, number(p.params.get("q")), w == null ? null : Duration.ofSeconds(w));
                    }
                }
                if (remaining != null) {
                    return new RateLimit(
                            policy != null ? policy.quota() : null,
                            remaining,
                            t == null ? null : Duration.ofSeconds(t),
                            policy);
                }
            }
        }
        List<String> names = List.of("x-ratelimit-limit", "x-ratelimit-remaining", "x-ratelimit-reset");
        if (names.stream().noneMatch(h::containsKey)) {
            return null;
        }
        Long[] got = new Long[3];
        for (int i = 0; i < 3; i++) {
            String raw = h.get(names.get(i));
            got[i] = number(raw);
            if (raw != null && got[i] == null) {
                return null;
            }
        }
        return new RateLimit(got[0], got[1], got[2] == null ? null : Duration.ofSeconds(got[2]), null);
    }

    private record Item(String name, Map<String, String> params) {}

    private static Item item(String value) {
        String first = value.split(",", 2)[0].strip();
        if (first.isEmpty()) {
            return null;
        }
        String[] parts = first.split(";");
        String name = parts[0].strip();
        if (name.length() >= 2 && name.startsWith("\"") && name.endsWith("\"")) {
            name = name.substring(1, name.length() - 1);
        }
        Map<String, String> params = new HashMap<>();
        for (int i = 1; i < parts.length; i++) {
            String p = parts[i].strip();
            int eq = p.indexOf('=');
            String k = eq < 0 ? p : p.substring(0, eq);
            if (!k.isEmpty()) {
                params.put(
                        k.strip().toLowerCase(Locale.ROOT),
                        eq < 0 ? "" : p.substring(eq + 1).strip());
            }
        }
        return new Item(name, params);
    }

    private static Long number(String v) {
        if (v == null) {
            return null;
        }
        String t = v.strip();
        return t.matches("[0-9]{1,18}") ? Long.valueOf(t) : null;
    }
}
