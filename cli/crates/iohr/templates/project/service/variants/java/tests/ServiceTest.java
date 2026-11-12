import app.Service;
import java.io.ByteArrayOutputStream;
import java.io.PrintStream;
import java.net.URI;
import java.net.http.HttpClient;
import java.net.http.HttpRequest;
import java.net.http.HttpResponse;
import java.nio.charset.StandardCharsets;
import java.util.Map;

/**
 * The passes-on-init test, with no test framework to install: it boots the service on a free
 * port, calls the probes, and checks the span, the trace id and what the log must not hold.
 * It exits 1 with the first failure.
 */
public final class ServiceTest {
    private static final HttpClient HTTP = HttpClient.newHttpClient();

    private record Booted(String base, ByteArrayOutputStream logs, Service svc) {}

    private static Booted boot() throws Exception {
        ByteArrayOutputStream logs = new ByteArrayOutputStream();
        Service svc = Service.forTest(new PrintStream(logs, true, StandardCharsets.UTF_8));
        int port = svc.start(new Service.Config("127.0.0.1", 0));
        return new Booted("http://127.0.0.1:" + port, logs, svc);
    }

    private static HttpResponse<String> get(String url, String... headers) throws Exception {
        HttpRequest.Builder b = HttpRequest.newBuilder(URI.create(url));
        for (int i = 0; i + 1 < headers.length; i += 2) {
            b.header(headers[i], headers[i + 1]);
        }
        return HTTP.send(b.build(), HttpResponse.BodyHandlers.ofString());
    }

    private static void check(boolean ok, String what) {
        if (!ok) {
            System.err.println("FAIL: " + what);
            System.exit(1);
        }
    }

    private static String logText(Booted b) {
        return b.logs().toString(StandardCharsets.UTF_8);
    }

    private static int count(String text, String needle) {
        int n = 0;
        for (int i = text.indexOf(needle); i >= 0; i = text.indexOf(needle, i + 1)) {
            n++;
        }
        return n;
    }

    private static void probesAnswer() throws Exception {
        Booted b = boot();
        try {
            check(get(b.base() + "/healthz").statusCode() == 200, "/healthz is 200");
            check(get(b.base() + "/readyz").statusCode() == 200, "/readyz is 200");
            HttpResponse<String> hello = get(b.base() + "/v1/hello?name=Ada");
            check(hello.statusCode() == 200 && hello.body().contains("Hello, Ada"), "hello answers");
            check(get(b.base() + "/nope").statusCode() == 404, "an unknown path is 404");
            check(get(b.base() + "/v1/hello?name=" + "x".repeat(65)).statusCode() == 400,
                    "a name over 64 characters is 400");
        } finally {
            b.svc().stop();
        }
    }

    private static void oneSpanWithTheRouteAndTheTraceIdInTheLog() throws Exception {
        Booted b = boot();
        String trace = "4bf92f3577b34da6a3ce929d0e0e4736";
        try {
            HttpResponse<String> r = get(b.base() + "/v1/hello?name=secret-query-value",
                    "traceparent", "00-" + trace + "-00f067aa0ba902b7-01",
                    "Authorization", "Bearer secret-header-value");
            String tp = r.headers().firstValue("traceparent").orElse("");
            check(tp.startsWith("00-" + trace + "-"), "the response continues the trace: " + tp);
        } finally {
            b.svc().stop();
        }
        String all = logText(b);
        String span = null;
        String request = null;
        for (String line : all.strip().split("\\R")) {
            check(line.startsWith("{") && line.endsWith("}"), "a log line is JSON: " + line);
            if (line.contains("\"msg\":\"span\"")) {
                span = line;
            }
            if (line.contains("\"msg\":\"request\"")) {
                request = line;
            }
        }
        check(count(all, "\"msg\":\"span\"") == 1, "one span line");
        check(count(all, "\"msg\":\"request\"") == 1, "one request line");
        check(span.contains("\"route\":\"/v1/hello\"") && span.contains("\"trace_id\":\"" + trace + "\"")
                && span.contains("\"parent_span_id\":\"00f067aa0ba902b7\""), "the span is right: " + span);
        check(request.contains("\"trace_id\":\"" + trace + "\""), "the request log has the trace id");
        for (String secret : new String[] {"secret-query-value", "secret-header-value"}) {
            check(!all.contains(secret), "the log holds " + secret);
        }
        for (Map.Entry<String, String> e : System.getenv().entrySet()) {
            String v = e.getValue();
            check(v.length() < 12 || !all.contains(v), "the log holds the value of " + e.getKey());
        }
    }

    private static void anUnknownPathIsNeverLoggedAndMetricsCountByRoute() throws Exception {
        Booted b = boot();
        String metrics;
        try {
            get(b.base() + "/private/user-1234");
            get(b.base() + "/healthz");
            metrics = get(b.base() + "/metrics").body();
        } finally {
            b.svc().stop();
        }
        check(!logText(b).contains("user-1234"), "the log holds a raw path");
        check(metrics.contains("route=\"unmatched\",status=\"404\""), "metrics count unmatched: " + metrics);
        check(metrics.contains("route=\"/healthz\",status=\"200\""), "metrics count /healthz: " + metrics);
    }

    private static void config() {
        try {
            Service.Config.load(k -> k.equals("PORT") ? "99999" : null);
            check(false, "a port out of range was accepted");
        } catch (IllegalArgumentException expected) {
            // the refusal is the point
        }
        Service.Config c = Service.Config.load(k -> null);
        check(c.port() == 8080 && c.host().equals("127.0.0.1"), "defaults");
    }

    public static void main(String[] args) throws Exception {
        probesAnswer();
        oneSpanWithTheRouteAndTheTraceIdInTheLog();
        anUnknownPathIsNeverLoggedAndMetricsCountByRoute();
        config();
        System.out.println("ok: 4 checks passed");
    }
}
