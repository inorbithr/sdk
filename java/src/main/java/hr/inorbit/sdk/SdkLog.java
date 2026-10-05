package hr.inorbit.sdk;

import hr.inorbit.sdk.middleware.Headers;
import java.net.http.HttpHeaders;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Set;
import java.util.function.UnaryOperator;

/**
 * One client's logging (docs/config.md section 7.9): the level, the header allowlist, the sink
 * and {@code redact}. Off by default. Records go to the {@link System.Logger} named {@code
 * hr.inorbit.sdk}, which SLF4J and Log4j bridge to, as one parameter: the record as a {@link Map}
 * whose text is {@code event key=value ...}.
 */
final class SdkLog {

    /** The logger every record goes to unless code gives one. */
    static final String LOGGER_NAME = "hr.inorbit.sdk";

    /** Never logged, whatever the settings. */
    static final Set<String> NEVER_LOGGED = Set.of("authorization", "proxy-authorization", "cookie", "set-cookie");

    static final Set<String> REQUEST_HEADERS = Set.of(
            "accept", "content-type", "content-length", "user-agent", "x-request-id", "traceparent", "idempotency-key");

    static final Set<String> RESPONSE_HEADERS = Set.of(
            "content-type",
            "content-length",
            "date",
            "retry-after",
            "x-request-id",
            "idempotency-replayed",
            "x-ratelimit-limit",
            "x-ratelimit-remaining",
            "x-ratelimit-reset",
            "ratelimit",
            "ratelimit-policy");

    private static final List<String> LEVELS = List.of("debug", "info", "warn", "error");

    private final int threshold;
    private final System.Logger logger;
    private final boolean withHeaders;
    private final Set<String> allowRequest;
    private final Set<String> allowResponse;
    private final UnaryOperator<Map<String, Object>> redact;
    private final String profile;

    SdkLog(
            String level,
            System.Logger logger,
            boolean withHeaders,
            List<String> allow,
            UnaryOperator<Map<String, Object>> redact,
            String profile) {
        this.threshold = LEVELS.indexOf(level);
        this.logger = logger != null ? logger : System.getLogger(LOGGER_NAME);
        this.withHeaders = withHeaders;
        Set<String> extra = new HashSet<>();
        for (String h : allow) {
            String n = h.toLowerCase(Locale.ROOT);
            if (!NEVER_LOGGED.contains(n)) {
                extra.add(n);
            }
        }
        this.allowRequest = union(REQUEST_HEADERS, extra);
        this.allowResponse = union(RESPONSE_HEADERS, extra);
        this.redact = redact;
        this.profile = profile;
    }

    static SdkLog off() {
        return new SdkLog("off", null, false, List.of(), null, null);
    }

    private static Set<String> union(Set<String> a, Set<String> b) {
        Set<String> out = new HashSet<>(a);
        out.addAll(b);
        return Set.copyOf(out);
    }

    /** Whether a record at {@code level} would be written. */
    boolean on(String level) {
        return threshold >= 0 && LEVELS.indexOf(level) >= threshold;
    }

    /** A request's headers as a record holds them, or {@code null} without header logging. */
    Map<String, String> headers(Headers headers) {
        if (!withHeaders) {
            return null;
        }
        Map<String, String> out = new LinkedHashMap<>();
        headers.asMap()
                .forEach((n, v) -> out.put(n, allowRequest.contains(n) && !NEVER_LOGGED.contains(n) ? v : "REDACTED"));
        return out;
    }

    /** An answer's headers as a record holds them, or {@code null} without header logging. */
    Map<String, String> headers(HttpHeaders headers) {
        if (!withHeaders) {
            return null;
        }
        Map<String, String> out = new LinkedHashMap<>();
        headers.map().forEach((name, values) -> {
            String n = name.toLowerCase(Locale.ROOT);
            if (n.startsWith(":")) {
                return;
            }
            String v = String.join(", ", values);
            out.put(n, allowResponse.contains(n) && !NEVER_LOGGED.contains(n) ? v : "REDACTED");
        });
        return out;
    }

    /** Writes one record, after {@code redact}; {@code null} fields are left out. */
    void emit(String level, String event, Map<String, Object> fields) {
        if (!on(level)) {
            return;
        }
        Record record = new Record();
        record.put("event", event);
        fields.forEach((k, v) -> {
            if (v != null) {
                record.put(k, v);
            }
        });
        if (profile != null) {
            record.putIfAbsent("profile", profile);
        }
        Map<String, Object> out = record;
        if (redact != null) {
            Map<String, Object> changed = redact.apply(record);
            if (changed == null) {
                return;
            }
            if (changed instanceof Record r) {
                out = r;
            } else {
                Record r = new Record();
                r.putAll(changed);
                out = r;
            }
        }
        System.Logger.Level l = switch (level) {
            case "debug" -> System.Logger.Level.DEBUG;
            case "info" -> System.Logger.Level.INFO;
            case "warn" -> System.Logger.Level.WARNING;
            default -> System.Logger.Level.ERROR;
        };
        logger.log(l, (java.util.ResourceBundle) null, "{0}", out);
    }

    /** One record: the fields by name, and as text {@code event key=value ...}. */
    static final class Record extends LinkedHashMap<String, Object> {
        private static final long serialVersionUID = 1L;

        @Override
        public String toString() {
            StringBuilder s = new StringBuilder(String.valueOf(get("event")));
            forEach((k, v) -> {
                if (!k.equals("event")) {
                    s.append(' ').append(k).append('=').append(v);
                }
            });
            return s.toString();
        }
    }
}
