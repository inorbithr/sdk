package app;

import java.io.PrintStream;
import java.nio.charset.StandardCharsets;
import java.security.SecureRandom;
import java.time.Instant;
import java.util.HexFormat;
import java.util.Map;
import java.util.TreeMap;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

/**
 * Observability on the JDK alone: JSON logs, spans that follow the W3C traceparent header, and
 * request counters in the Prometheus text format. Nothing here records a header, a query string
 * or an environment value: a log line and a span name the route that matched, the status, the
 * duration and the trace id.
 */
final class Obs {
    private Obs() {}

    private static final SecureRandom RANDOM = new SecureRandom();
    private static final Pattern TRACEPARENT =
            Pattern.compile("^00-([0-9a-f]{32})-([0-9a-f]{16})-([0-9a-f]{2})$");

    static String randomHex(int bytes) {
        byte[] b = new byte[bytes];
        RANDOM.nextBytes(b);
        return HexFormat.of().formatHex(b);
    }

    /** One JSON object per line, written to a stream. Values are strings, numbers or booleans. */
    static final class Logger {
        private final PrintStream out;

        Logger(PrintStream out) {
            this.out = out;
        }

        /** Writes one record: the time, the level, the message, then the attributes in order. */
        void log(String level, String msg, Object... keyValues) {
            StringBuilder b = new StringBuilder(160);
            b.append("{\"time\":\"").append(Instant.now()).append("\",\"level\":\"").append(level)
                    .append("\",\"msg\":");
            quote(b, msg);
            for (int i = 0; i + 1 < keyValues.length; i += 2) {
                b.append(',');
                quote(b, String.valueOf(keyValues[i]));
                b.append(':');
                Object v = keyValues[i + 1];
                if (v instanceof Number || v instanceof Boolean) {
                    b.append(v);
                } else {
                    quote(b, String.valueOf(v));
                }
            }
            b.append('}');
            synchronized (out) {
                out.println(b);
                out.flush();
            }
        }

        void info(String msg, Object... kv) {
            log("INFO", msg, kv);
        }

        void error(String msg, Object... kv) {
            log("ERROR", msg, kv);
        }
    }

    /** Appends s as a JSON string, escaping what JSON requires. */
    static void quote(StringBuilder b, String s) {
        b.append('"');
        for (int i = 0; i < s.length(); i++) {
            char c = s.charAt(i);
            switch (c) {
                case '"' -> b.append("\\\"");
                case '\\' -> b.append("\\\\");
                case '\n' -> b.append("\\n");
                case '\r' -> b.append("\\r");
                case '\t' -> b.append("\\t");
                default -> {
                    if (c < 0x20) {
                        b.append(String.format("\\u%04x", (int) c));
                    } else {
                        b.append(c);
                    }
                }
            }
        }
        b.append('"');
    }

    static String json(String s) {
        StringBuilder b = new StringBuilder();
        quote(b, s);
        return b.toString();
    }

    /** One request's trace: the id it joined (or started) and its own id. */
    record Span(String traceId, String spanId, String parentId, long startNanos) {
        /** Joins the trace of a valid traceparent header, or starts a new one. */
        static Span start(String traceparent) {
            Matcher m = traceparent == null ? null : TRACEPARENT.matcher(traceparent);
            if (m != null && m.matches()
                    && !m.group(1).equals("0".repeat(32)) && !m.group(2).equals("0".repeat(16))) {
                return new Span(m.group(1), randomHex(8), m.group(2), System.nanoTime());
            }
            return new Span(randomHex(16), randomHex(8), null, System.nanoTime());
        }

        /** The traceparent to send back (and on to callers): this span is the parent. */
        String header() {
            return "00-" + traceId + "-" + spanId + "-01";
        }

        double millis() {
            return (System.nanoTime() - startNanos) / 1_000_000.0;
        }
    }

    /**
     * Counts requests by route and status. Routes are the matched patterns, never raw paths, so
     * the label set is bounded by the code and holds no user input.
     */
    static final class Metrics {
        private final Map<String, Long> counts = new TreeMap<>();

        synchronized void inc(String route, int status) {
            counts.merge(route + "\u0000" + status, 1L, Long::sum);
        }

        synchronized String render() {
            StringBuilder b = new StringBuilder(
                    "# HELP http_requests_total Requests served, by route and status.\n"
                            + "# TYPE http_requests_total counter\n");
            counts.forEach((k, n) -> {
                String[] p = k.split("\u0000");
                b.append("http_requests_total{route=").append(json(p[0])).append(",status=")
                        .append(json(p[1])).append("} ").append(n).append('\n');
            });
            return b.toString();
        }
    }

    static byte[] utf8(String s) {
        return s.getBytes(StandardCharsets.UTF_8);
    }
}
