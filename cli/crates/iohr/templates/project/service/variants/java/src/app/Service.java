package app;

import com.sun.net.httpserver.HttpExchange;
import com.sun.net.httpserver.HttpServer;
import java.io.IOException;
import java.io.OutputStream;
import java.net.InetSocketAddress;
import java.net.URLDecoder;
import java.nio.charset.StandardCharsets;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.atomic.AtomicBoolean;

/**
 * The service: /healthz, /readyz, /metrics and one route of your own. ready is false until the
 * server listens and again as soon as shutdown starts, so a load balancer stops sending traffic
 * before the process stops.
 */
public final class Service {
    // The JDK web server's own limits (module jdk.httpserver): read a request in 15 s, write a
    // response in 30 s, drop an idle connection after 60 s. Set before the first server exists.
    static {
        System.setProperty("sun.net.httpserver.maxReqTime", "15");
        System.setProperty("sun.net.httpserver.maxRspTime", "30");
        System.setProperty("sun.net.httpserver.idleInterval", "60");
    }

    final Obs.Logger log;
    final Obs.Metrics metrics = new Obs.Metrics();
    final AtomicBoolean ready = new AtomicBoolean(false);
    private HttpServer server;
    private ExecutorService pool;

    Service(Obs.Logger log) {
        this.log = log;
    }

    /** A service that logs to the stream given (the test reads it back). */
    public static Service forTest(java.io.PrintStream out) {
        return new Service(new Obs.Logger(out));
    }

    /** The settings, read from the environment: the names are in .env.example. */
    public record Config(String host, int port) {
        public static Config load(java.util.function.Function<String, String> get) {
            String host = "127.0.0.1";
            int port = 8080;
            String h = get.apply("HOST");
            if (h != null && !h.isEmpty()) {
                host = h;
            }
            String p = get.apply("PORT");
            if (p != null && !p.isEmpty()) {
                try {
                    port = Integer.parseInt(p);
                } catch (NumberFormatException e) {
                    throw new IllegalArgumentException("PORT must be a number from 0 to 65535");
                }
                if (port < 0 || port > 65535) {
                    throw new IllegalArgumentException("PORT must be a number from 0 to 65535");
                }
            }
            return new Config(host, port);
        }
    }

    /** Starts listening and returns the port it got (PORT=0 picks a free one). */
    public int start(Config cfg) throws IOException {
        server = HttpServer.create(new InetSocketAddress(cfg.host(), cfg.port()), 0);
        pool = Executors.newFixedThreadPool(16);
        server.setExecutor(pool);
        server.createContext("/", this::observe);
        server.start();
        ready.set(true);
        int port = server.getAddress().getPort();
        log.info("listening", "addr", server.getAddress().getHostString() + ":" + port);
        return port;
    }

    /** Stops accepting, lets requests in flight finish (10 s at most), then returns. */
    public void stop() {
        ready.set(false);
        log.info("shutting down");
        if (server != null) {
            server.stop(10);
        }
        if (pool != null) {
            pool.shutdown();
        }
    }

    private record Reply(int status, String type, String body) {}

    private static Reply json(int status, String body) {
        return new Reply(status, "application/json", body + "\n");
    }

    /** Routes by exact path and wraps every request in the one place it is traced, logged and counted. */
    private void observe(HttpExchange ex) throws IOException {
        Obs.Span span = Obs.Span.start(ex.getRequestHeaders().getFirst("traceparent"));
        ex.getResponseHeaders().set("traceparent", span.header());
        String method = ex.getRequestMethod();
        String path = ex.getRequestURI().getRawPath();
        String route = "unmatched";
        Reply reply;
        if (method.equals("GET") && path.equals("/healthz")) {
            route = path;
            reply = json(200, "{\"status\":\"ok\"}");
        } else if (method.equals("GET") && path.equals("/readyz")) {
            route = path;
            reply = ready.get()
                    ? json(200, "{\"status\":\"ready\"}")
                    : json(503, "{\"status\":\"not ready\"}");
        } else if (method.equals("GET") && path.equals("/metrics")) {
            route = path;
            reply = new Reply(200, "text/plain; version=0.0.4", metrics.render());
        } else if (method.equals("GET") && path.equals("/v1/hello")) {
            route = path;
            reply = hello(ex.getRequestURI().getRawQuery());
        } else {
            reply = json(404, "{\"error\":\"not found\"}");
        }
        byte[] body = Obs.utf8(reply.body());
        ex.getResponseHeaders().set("Content-Type", reply.type());
        ex.sendResponseHeaders(reply.status(), body.length);
        try (OutputStream out = ex.getResponseBody()) {
            out.write(body);
        }
        double ms = span.millis();
        if (span.parentId() != null) {
            log.info("span", "trace_id", span.traceId(), "span_id", span.spanId(),
                    "name", method + " " + route, "route", route, "status", reply.status(),
                    "duration_ms", ms, "parent_span_id", span.parentId());
        } else {
            log.info("span", "trace_id", span.traceId(), "span_id", span.spanId(),
                    "name", method + " " + route, "route", route, "status", reply.status(),
                    "duration_ms", ms);
        }
        log.info("request", "trace_id", span.traceId(), "method", method, "route", route,
                "status", reply.status(), "duration_ms", ms);
        metrics.inc(route, reply.status());
    }

    /**
     * Your first route: replace it. The name comes from the query string, so it is bounded and
     * echoed back, never logged.
     */
    private static Reply hello(String rawQuery) {
        String name = "world";
        if (rawQuery != null) {
            for (String pair : rawQuery.split("&")) {
                int eq = pair.indexOf('=');
                if (eq > 0 && pair.substring(0, eq).equals("name")) {
                    try {
                        name = URLDecoder.decode(pair.substring(eq + 1), StandardCharsets.UTF_8);
                    } catch (IllegalArgumentException e) {
                        return json(400, "{\"error\":\"name is at most 64 characters of text\"}");
                    }
                    break;
                }
            }
        }
        if (name.isEmpty()) {
            name = "world";
        }
        if (name.codePointCount(0, name.length()) > 64) {
            return json(400, "{\"error\":\"name is at most 64 characters of text\"}");
        }
        return json(200, "{\"message\":" + Obs.json("Hello, " + name) + "}");
    }
}
