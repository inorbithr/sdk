package hr.inorbit.sdk;

import com.fasterxml.jackson.databind.JsonNode;
import hr.inorbit.sdk.auth.StaticToken;
import hr.inorbit.sdk.auth.TokenProvider;
import hr.inorbit.sdk.errors.ApiException;
import hr.inorbit.sdk.errors.AuthException;
import hr.inorbit.sdk.errors.ConfigException;
import hr.inorbit.sdk.errors.ConnectionException;
import hr.inorbit.sdk.errors.InOrbitException;
import hr.inorbit.sdk.errors.TimeoutException;
import hr.inorbit.sdk.errors.TooLargeException;
import hr.inorbit.sdk.middleware.Chain;
import hr.inorbit.sdk.middleware.Middleware;
import hr.inorbit.sdk.middleware.Pipeline;
import hr.inorbit.sdk.middleware.Request;
import hr.inorbit.sdk.middleware.Response;
import java.io.IOException;
import java.net.URI;
import java.net.http.HttpClient;
import java.net.http.HttpHeaders;
import java.net.http.HttpRequest;
import java.net.http.HttpResponse;
import java.net.http.HttpTimeoutException;
import java.nio.ByteBuffer;
import java.time.Duration;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.UUID;
import java.util.concurrent.CancellationException;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.CompletionStage;
import java.util.concurrent.ExecutionException;
import java.util.concurrent.Flow;
import java.util.concurrent.ThreadLocalRandom;
import java.util.concurrent.TimeUnit;

/**
 * What every built-in middleware of one client reads (settings, the retry budget, logging,
 * telemetry, hooks), the twelve built-ins of docs/config.md section 7.2, and the transport.
 */
final class Engine {

    final URI base;
    final String host;
    final String userAgent;
    final Duration timeout;
    final Duration totalTimeout;
    final Duration idle;
    final int maxRetries;
    final Duration retryBaseDelay;
    final Duration retryMaxDelay;
    final Duration retryAfterMax;
    final String rateLimitMode;
    final RetryBudget budget;
    final SdkLog log;
    final Telemetry telemetry;
    final List<Hook> hooks;
    final String profile;
    final String credentialSource;
    final HttpClient http;
    final NoProxy proxy;
    final boolean refuseStatic;
    private volatile RateLimit latest;

    Engine(
            URI base,
            String userAgent,
            Duration timeout,
            Duration totalTimeout,
            Duration idle,
            int maxRetries,
            Duration retryBaseDelay,
            Duration retryMaxDelay,
            Duration retryAfterMax,
            String rateLimitMode,
            RetryBudget budget,
            SdkLog log,
            Telemetry telemetry,
            List<Hook> hooks,
            String profile,
            String credentialSource,
            HttpClient http,
            NoProxy proxy,
            boolean refuseStatic) {
        this.base = base;
        this.host = base.getRawAuthority();
        this.userAgent = userAgent;
        this.timeout = timeout;
        this.totalTimeout = totalTimeout;
        this.idle = idle;
        this.maxRetries = maxRetries;
        this.retryBaseDelay = retryBaseDelay;
        this.retryMaxDelay = retryMaxDelay;
        this.retryAfterMax = retryAfterMax;
        this.rateLimitMode = rateLimitMode;
        this.budget = budget;
        this.log = log;
        this.telemetry = telemetry;
        this.hooks = List.copyOf(hooks);
        this.profile = profile;
        this.credentialSource = credentialSource;
        this.http = http;
        this.proxy = proxy;
        this.refuseStatic = refuseStatic;
    }

    RateLimit latest() {
        return latest;
    }

    void observe(RateLimit rl) {
        latest = rl;
    }

    /** The answer as callers and hooks see it. */
    RawResponse raw(Request req, Response resp) {
        Call c = Call.of(req);
        return new RawResponse(
                resp.status(),
                resp.headers(),
                resp.body() == null ? new byte[0] : resp.body(),
                req.info().requestId(),
                Math.max(req.info().attempt(), 1),
                req.info().idempotencyKey().orElse(null),
                c != null ? c.rateLimit : null);
    }

    /** The {@link Hook.Attempt} hooks see. */
    Hook.Attempt attemptOf(Request req, String stage) {
        Call c = Call.of(req);
        Method m;
        try {
            m = Method.valueOf(req.method());
        } catch (IllegalArgumentException e) {
            m = Method.GET;
        }
        return new Hook.Attempt(
                req.info().operation(),
                m,
                c != null ? c.op.path() : req.uri().getRawPath(),
                Math.max(req.info().attempt(), 1),
                req.info().requestId(),
                req.info().idempotencyKey().orElse(null),
                stage);
    }

    /** {@code trace_id} and {@code span_id} of the attempt's span, for log records. */
    void ids(Request req, Map<String, Object> into) {
        Call c = Call.of(req);
        String[] ids = telemetry.ids(c != null ? c.attemptSpan : null);
        if (ids != null) {
            into.put("trace_id", ids[0]);
            into.put("span_id", ids[1]);
        }
    }

    /** The {@code error.type} attribute for an error: the status, or the error's class. */
    static String errorType(InOrbitException e) {
        return e instanceof ApiException a
                ? String.valueOf(a.status())
                : e.getClass().getSimpleName();
    }

    static Map<String, Object> fields(Object... kv) {
        Map<String, Object> m = new LinkedHashMap<>();
        for (int i = 0; i < kv.length; i += 2) {
            if (kv[i + 1] != null) {
                m.put((String) kv[i], kv[i + 1]);
            }
        }
        return m;
    }

    /** Waits {@code d}, never less: a timer that wakes early sleeps again for the rest. */
    static void sleep(Duration d, String host) {
        long until = System.nanoTime() + d.toNanos();
        try {
            for (long left = d.toNanos(); left > 0; left = until - System.nanoTime()) {
                Thread.sleep(left / 1_000_000, (int) (left % 1_000_000));
            }
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            throw new ConnectionException(host, "interrupted while waiting", e);
        }
    }

    /** Lets a stream's answer go without reading it. */
    static void discard(Response resp) {
        if (resp.stream() != null) {
            resp.stream().subscribe(HttpResponse.BodySubscribers.discarding());
        }
    }

    // --- the chain -----------------------------------------------------------------------

    /** The transport: one HTTP exchange. */
    interface Terminal {
        Response send(Request req);
    }

    /** One position in the pipeline. */
    static final class Link implements Chain {
        private final List<Middleware> ms;
        private final int index;
        private final Terminal terminal;
        private final hr.inorbit.sdk.middleware.CallInfo info;

        Link(List<Middleware> ms, int index, Terminal terminal, hr.inorbit.sdk.middleware.CallInfo info) {
            this.ms = ms;
            this.index = index;
            this.terminal = terminal;
            this.info = info;
        }

        @Override
        public Response proceed(Request request) {
            if (index >= ms.size()) {
                return terminal.send(request);
            }
            return ms.get(index).handle(request, new Link(ms, index + 1, terminal, request.info()));
        }

        @Override
        public hr.inorbit.sdk.middleware.CallInfo info() {
            return info;
        }
    }

    /** The default pipeline. */
    Pipeline builtins(TokenProvider provider) {
        List<Middleware> ms = List.of(
                new Builtin("request_id", this::requestId),
                new Builtin("user_agent", this::userAgentStep),
                new Builtin("idempotency_key", this::idempotencyKey),
                new Builtin("call_tracing", this::callTracing),
                new Builtin("deadline", this::deadline),
                new Builtin("retry", this::retry),
                new Builtin("auth", (r, c) -> auth(provider, r, c)),
                new Builtin("rate_limit", this::rateLimit),
                new Builtin("attempt_tracing", this::attemptTracing),
                new Builtin("logging", this::logging),
                new Builtin("hooks", this::hooksStep),
                new Builtin("timeout", this::timeoutStep));
        return new Pipeline(ms);
    }

    /** A built-in: a name and what it does. */
    record Builtin(String name, Middleware.Handler handler) implements Middleware {
        @Override
        public Response handle(Request request, Chain chain) {
            return handler.handle(request, chain);
        }

        @Override
        public String toString() {
            return "built-in " + name;
        }
    }

    // --- the built-ins -------------------------------------------------------------------

    private Response requestId(Request req, Chain next) {
        req.headers().set("x-request-id", req.info().requestId());
        return next.proceed(req);
    }

    private Response userAgentStep(Request req, Chain next) {
        req.headers().set("user-agent", userAgent);
        return next.proceed(req);
    }

    private Response idempotencyKey(Request req, Chain next) {
        Call c = Call.of(req);
        if (c == null || !c.op.takesIdempotencyKey() || !(req.info() instanceof Call.Info info)) {
            return next.proceed(req);
        }
        String key =
                c.idempotencyKey != null ? c.idempotencyKey : UUID.randomUUID().toString();
        c.idempotencyKey = key;
        Request keyed = req.withInfo(info.withKey(key));
        keyed.headers().set("idempotency-key", key);
        return next.proceed(keyed);
    }

    private Response callTracing(Request req, Chain next) {
        Call c = Call.of(req);
        if (!telemetry.tracing() || c == null) {
            return next.proceed(req);
        }
        Object span = telemetry.startCall(
                req.info().operation(),
                fields(
                        "inorbit.operation",
                        req.info().operation(),
                        "inorbit.request_id",
                        req.info().requestId()),
                c.traceparent);
        c.callSpan = span;
        Response resp;
        try {
            resp = next.proceed(req);
        } catch (InOrbitException e) {
            telemetry.end(span, errorType(e), Map.of());
            throw e;
        }
        if (resp.status() >= 300) {
            telemetry.end(span, String.valueOf(resp.status()), Map.of());
        } else if (req.info().stream()) {
            synchronized (c.onClose) {
                c.onClose.add(() -> telemetry.end(span, null, Map.of()));
            }
        } else {
            telemetry.end(span, null, Map.of());
        }
        return resp;
    }

    private Response deadline(Request req, Chain next) {
        Call c = Call.of(req);
        if (c == null || !(req.info() instanceof Call.Info info)) {
            return next.proceed(req);
        }
        Duration total = totalTimeout;
        if (c.timeout != null && c.timeout.compareTo(total) < 0) {
            total = c.timeout;
        }
        return next.proceed(req.withInfo(info.withDeadline(c.started + total.toNanos())));
    }

    private static long deadlineOf(Request req) {
        return req.info() instanceof Call.Info i ? i.deadlineNanos() : Long.MIN_VALUE;
    }

    /** What {@code retry} decided after an attempt. */
    private record Verdict(boolean retry, Duration delay, int cost, String reason) {
        static final Verdict NO = new Verdict(false, Duration.ZERO, 0, "");
    }

    private Verdict decide(Request req, Response resp, InOrbitException err, int retries) {
        boolean safe = req.info().idempotent() || req.headers().contains("idempotency-key");
        String reason;
        if (resp != null) {
            if (resp.status() != 429 && resp.status() != 503 && resp.status() != 504) {
                return Verdict.NO;
            }
            reason = String.valueOf(resp.status());
        } else if (err instanceof TimeoutException || err instanceof ConnectionException) {
            reason = err.kind();
        } else {
            return Verdict.NO;
        }
        if (!safe || retries >= maxRetries) {
            return Verdict.NO;
        }
        Duration asked = null;
        int cost = RetryBudget.COST_OTHER;
        if (resp != null) {
            asked = Retry.retryAfterUncapped(resp.headers()).orElse(null);
            boolean header = asked != null;
            if (asked == null) {
                asked = retryDetail(resp);
            }
            if (asked != null && asked.compareTo(retryAfterMax) > 0) {
                return Verdict.NO;
            }
            cost = resp.status() == 429 || (resp.status() == 503 && header)
                    ? RetryBudget.COST_THROTTLED
                    : RetryBudget.COST_OTHER;
        }
        Duration delay = asked != null ? asked : Retry.backoff(retries, retryBaseDelay, retryMaxDelay);
        long deadline = deadlineOf(req);
        if (deadline != Long.MIN_VALUE && System.nanoTime() + delay.toNanos() - deadline >= 0) {
            return Verdict.NO;
        }
        if (!budget.draw(cost)) {
            return Verdict.NO;
        }
        return new Verdict(true, delay, cost, reason);
    }

    private static Duration retryDetail(Response resp) {
        if (resp.body() == null || resp.body().length == 0) {
            return null;
        }
        try {
            JsonNode envelope = Json.MAPPER.readTree(resp.body());
            for (JsonNode d : envelope.path("details")) {
                if (d.path("type").asText("").equals("retry")
                        && d.path("after_seconds").isNumber()) {
                    return Duration.ofMillis(Math.round(d.path("after_seconds").asDouble() * 1000));
                }
            }
        } catch (IOException e) {
            return null;
        }
        return null;
    }

    private Response retry(Request req, Chain next) {
        int retries = 0;
        int lastCost = 0;
        while (true) {
            Request attempt = req.info() instanceof Call.Info info
                    ? req.withInfo(info.withAttempt(retries + 1))
                    : req.withInfo(req.info());
            Call c = Call.of(attempt);
            if (c != null) {
                c.attempts = retries + 1;
            }
            Response resp = null;
            InOrbitException err = null;
            try {
                resp = next.proceed(attempt);
            } catch (TimeoutException | ConnectionException e) {
                err = e;
            }
            Verdict v = decide(attempt, resp, err, retries);
            if (!v.retry()) {
                if (resp != null && resp.status() >= 200 && resp.status() < 300) {
                    budget.refund(retries > 0 ? lastCost : 1);
                }
                if (err != null) {
                    throw err;
                }
                return resp;
            }
            if (resp != null) {
                discard(resp);
            }
            Hook.Attempt a = attemptOf(attempt, "per_retry");
            for (Hook h : hooks) {
                h.onRetry(a, v.reason(), v.delay());
            }
            log.emit(
                    "warn",
                    "retry",
                    fields(
                            "operation", attempt.info().operation(),
                            "attempt", attempt.info().attempt(),
                            "reason", v.reason(),
                            "delay_ms", v.delay().toMillis(),
                            "request_id", attempt.info().requestId()));
            telemetry.record(
                    "retries",
                    1,
                    fields("inorbit.operation", attempt.info().operation(), "inorbit.retry.reason", v.reason()));
            sleep(v.delay(), host);
            retries++;
            lastCost = v.cost();
        }
    }

    private static String bearer(TokenProvider provider) {
        return "Bearer " + provider.token().access();
    }

    private Response auth(TokenProvider provider, Request req, Chain next) {
        req.headers().set("authorization", bearer(provider));
        Response resp = next.proceed(req);
        Call c = Call.of(req);
        if (resp.status() != 401 || c == null || c.refreshed) {
            return resp;
        }
        c.refreshed = true;
        discard(resp);
        if (refuseStatic && provider instanceof StaticToken) {
            throw new AuthException(
                    "the API refused the token (HTTP 401): it is expired or revoked; create a new API token "
                            + "in the console or with `iohr token create`, and set it again",
                    "HTTP 401",
                    null);
        }
        provider.invalidate();
        Request again = req.withInfo(req.info());
        again.headers().set("authorization", bearer(provider));
        return next.proceed(again);
    }

    private Response rateLimit(Request req, Chain next) {
        if (rateLimitMode.equals("wait")) {
            RateLimit l = latest;
            Duration wait = l != null ? l.waitNeeded() : Duration.ZERO;
            if (!wait.isZero()) {
                wait = wait.plusMillis(ThreadLocalRandom.current().nextLong(101));
                long deadline = deadlineOf(req);
                if (deadline != Long.MIN_VALUE && System.nanoTime() + wait.toNanos() - deadline > 0) {
                    throw new TimeoutException(
                            host,
                            "the call's deadline passes while waiting for the rate-limit window (" + host + ")",
                            null);
                }
                log.emit(
                        "warn",
                        "rate_limit_wait",
                        fields(
                                "operation", req.info().operation(),
                                "attempt", req.info().attempt(),
                                "delay_ms", wait.toMillis(),
                                "request_id", req.info().requestId()));
                sleep(wait, host);
            }
        }
        Response resp = next.proceed(req);
        if (!rateLimitMode.equals("off")) {
            RateLimit rl = RateLimit.parse(resp.headers());
            if (rl != null) {
                observe(rl);
                Call c = Call.of(req);
                if (c != null) {
                    c.rateLimit = rl;
                }
            }
        }
        return resp;
    }

    private static int port(URI u) {
        return u.getPort() >= 0 ? u.getPort() : "https".equals(u.getScheme()) ? 443 : 80;
    }

    private Response attemptTracing(Request req, Chain next) {
        long started = System.nanoTime();
        Call c = Call.of(req);
        Object span = null;
        if (telemetry.tracing()) {
            String template = c != null ? c.template : null;
            URI u = req.uri();
            Map<String, Object> attrs = fields(
                    "http.request.method", req.method(),
                    "server.address", u.getHost(),
                    "server.port", port(u),
                    "url.full", redactedUrl(u),
                    "url.template", template,
                    "http.request.resend_count",
                            req.info().attempt() > 1 ? req.info().attempt() - 1 : null);
            span = telemetry.startAttempt(
                    c != null ? c.callSpan : null,
                    template != null ? req.method() + " " + template : req.method(),
                    attrs);
            if (c != null) {
                c.attemptSpan = span;
            }
            telemetry.inject(span, req.headers());
        }
        if (c != null && c.traceparent != null && !req.headers().contains("traceparent")) {
            // No span of ours to continue: the caller's goes as it came.
            req.headers().set("traceparent", c.traceparent);
        }
        Response resp;
        try {
            resp = next.proceed(req);
        } catch (InOrbitException e) {
            attemptDone(req, span, started, null, e);
            throw e;
        }
        attemptDone(req, span, started, resp, null);
        return resp;
    }

    private void attemptDone(Request req, Object span, long started, Response resp, InOrbitException err) {
        Integer status = resp != null ? resp.status() : null;
        String etype = err != null ? errorType(err) : status != null && status >= 400 ? String.valueOf(status) : null;
        boolean replayed = resp != null
                && resp.headers()
                        .firstValue("idempotency-replayed")
                        .map(v -> v.strip().equalsIgnoreCase("true"))
                        .orElse(false);
        telemetry.end(
                span,
                etype,
                fields(
                        "http.response.status_code", status,
                        "inorbit.server_request_id",
                                resp != null
                                        ? resp.headers()
                                                .firstValue("x-request-id")
                                                .orElse(null)
                                        : null,
                        "inorbit.idempotency_replayed", replayed ? Boolean.TRUE : null));
        URI u = req.uri();
        telemetry.record(
                "attempt",
                (System.nanoTime() - started) / 1e9,
                fields(
                        "http.request.method", req.method(),
                        "server.address", u.getHost(),
                        "server.port", port(u),
                        "http.response.status_code", status,
                        "error.type", etype));
    }

    /** A URL with every query value replaced by {@code REDACTED}. */
    static String redactedUrl(URI u) {
        String q = u.getRawQuery();
        String head = u.getScheme() + "://" + u.getRawAuthority() + (u.getRawPath() == null ? "" : u.getRawPath());
        if (q == null || q.isEmpty()) {
            return head;
        }
        StringBuilder out = new StringBuilder(head).append('?');
        String[] pairs = q.split("&");
        for (int i = 0; i < pairs.length; i++) {
            int eq = pairs[i].indexOf('=');
            out.append(i > 0 ? "&" : "")
                    .append(eq < 0 ? pairs[i] : pairs[i].substring(0, eq))
                    .append("=REDACTED");
        }
        return out.toString();
    }

    private Response logging(Request req, Chain next) {
        long started = System.nanoTime();
        if (log.on("debug")) {
            Map<String, Object> f = fields(
                    "operation", req.info().operation(),
                    "method", req.method(),
                    "path", req.uri().getRawPath(),
                    "attempt", req.info().attempt(),
                    "request_id", req.info().requestId(),
                    "headers", log.headers(req.headers()));
            ids(req, f);
            log.emit("debug", "request", f);
        }
        Response resp = next.proceed(req);
        if (log.on("debug")) {
            Map<String, Object> f = fields(
                    "operation", req.info().operation(),
                    "method", req.method(),
                    "path", req.uri().getRawPath(),
                    "attempt", req.info().attempt(),
                    "request_id", req.info().requestId(),
                    "status", resp.status(),
                    "duration_ms", (System.nanoTime() - started) / 1_000_000,
                    "server_request_id",
                            resp.headers().firstValue("x-request-id").orElse(null),
                    "headers", log.headers(resp.headers()));
            ids(req, f);
            log.emit("debug", "response", f);
        }
        return resp;
    }

    private Response hooksStep(Request req, Chain next) {
        Hook.Attempt attempt = attemptOf(req, "per_retry");
        Call c = Call.of(req);
        if (c != null) {
            c.lastAttempt = attempt;
        }
        for (Hook h : hooks) {
            h.onRequest(attempt);
        }
        Response resp = next.proceed(req);
        if (c != null) {
            c.serverRequestId = resp.headers().firstValue("x-request-id").orElse(null);
        }
        if (!hooks.isEmpty()) {
            RawResponse raw = raw(req, resp);
            for (Hook h : hooks) {
                h.onResponse(attempt, raw);
            }
        }
        return resp;
    }

    private Response timeoutStep(Request req, Chain next) {
        Call c = Call.of(req);
        Duration t = c != null && c.timeout != null ? c.timeout : timeout;
        long deadline = deadlineOf(req);
        if (deadline != Long.MIN_VALUE) {
            long left = deadline - System.nanoTime();
            if (left <= 0) {
                throw new TimeoutException(
                        host, (c != null && c.timeout != null ? c.timeout : totalTimeout).toSeconds(), null);
            }
            if (left < t.toNanos()) {
                t = Duration.ofNanos(left);
            }
        }
        if (c != null) {
            c.attemptTimeout = t;
        }
        return next.proceed(req);
    }

    // --- the transport -------------------------------------------------------------------

    /** The innermost step: one HTTP exchange, the body read under the attempt's limit. */
    Response send(Request req) {
        Call c = Call.of(req);
        Duration t = c != null && c.attemptTimeout != null ? c.attemptTimeout : timeout;
        HttpRequest.Builder b;
        try {
            b = HttpRequest.newBuilder(req.uri()).timeout(t);
            for (Map.Entry<String, String> h : req.headers().asMap().entrySet()) {
                b.header(h.getKey(), h.getValue());
            }
            if (proxy != null) {
                String auth = proxy.authorization();
                if (auth != null && proxy.proxyFor(req.uri()) != null) {
                    b.header("proxy-authorization", auth);
                }
            }
            b.method(
                    req.method(),
                    req.body() == null
                            ? HttpRequest.BodyPublishers.noBody()
                            : HttpRequest.BodyPublishers.ofByteArray(req.body()));
        } catch (IllegalArgumentException e) {
            throw new ConfigException("the request cannot be sent: " + e.getMessage());
        }
        HttpRequest request = b.build();
        long until = System.nanoTime() + t.toNanos();
        if (req.info().stream()) {
            HttpResponse<Flow.Publisher<List<ByteBuffer>>> resp =
                    await(http.sendAsync(request, HttpResponse.BodyHandlers.ofPublisher()), until, t);
            refuseRedirect(resp);
            if (resp.statusCode() >= 200 && resp.statusCode() < 300) {
                return Response.ofStream(resp.statusCode(), resp.headers(), resp.body());
            }
            Capped capped = new Capped();
            resp.body().subscribe(capped);
            byte[] body = await(capped.getBody().toCompletableFuture(), until, t);
            return new Response(resp.statusCode(), resp.headers(), body);
        }
        HttpResponse<byte[]> resp = await(http.sendAsync(request, info -> new Capped()), until, t);
        refuseRedirect(resp);
        return new Response(resp.statusCode(), resp.headers(), resp.body());
    }

    private void refuseRedirect(HttpResponse<?> resp) {
        if (resp.previousResponse().isPresent()) {
            // A caller's client followed a redirect: the SDK never does (docs/config.md 6.5).
            HttpResponse<?> first = resp.previousResponse().get();
            while (first.previousResponse().isPresent()) {
                first = first.previousResponse().get();
            }
            throw ApiException.of(new RawResponse(first.statusCode(), first.headers(), new byte[0], "", 1));
        }
    }

    private <T> T await(CompletableFuture<T> f, long until, Duration t) {
        long left = until - System.nanoTime();
        try {
            return f.get(Math.max(left, 1), TimeUnit.NANOSECONDS);
        } catch (java.util.concurrent.TimeoutException e) {
            f.cancel(true);
            throw new TimeoutException(host, Math.max(t.toSeconds(), 1), e);
        } catch (InterruptedException e) {
            f.cancel(true);
            Thread.currentThread().interrupt();
            throw new ConnectionException(host, "interrupted", e);
        } catch (CancellationException e) {
            throw new ConnectionException(host, "cancelled", e);
        } catch (ExecutionException e) {
            Throwable cause = e.getCause();
            if (cause instanceof InOrbitException x) {
                throw x;
            }
            if (cause instanceof HttpTimeoutException) {
                throw new TimeoutException(host, Math.max(t.toSeconds(), 1), cause);
            }
            if (cause instanceof IOException && cause.getCause() instanceof TooLargeException tl) {
                throw tl;
            }
            throw new ConnectionException(host, describe(cause), cause);
        }
    }

    static String describe(Throwable e) {
        String m = e == null ? null : e.getMessage();
        return m == null || m.isEmpty() ? (e == null ? "unknown" : e.getClass().getSimpleName()) : m;
    }

    /** A body read whole, refused past {@link Client#MAX_BODY}. */
    static final class Capped implements HttpResponse.BodySubscriber<byte[]> {
        private final CompletableFuture<byte[]> result = new CompletableFuture<>();
        private final List<ByteBuffer> parts = new ArrayList<>();
        private long size;
        private Flow.Subscription subscription;

        @Override
        public CompletionStage<byte[]> getBody() {
            return result;
        }

        @Override
        public void onSubscribe(Flow.Subscription s) {
            subscription = s;
            s.request(Long.MAX_VALUE);
        }

        @Override
        public void onNext(List<ByteBuffer> item) {
            for (ByteBuffer b : item) {
                size += b.remaining();
                parts.add(b);
            }
            if (size > Client.MAX_BODY) {
                subscription.cancel();
                result.completeExceptionally(new TooLargeException());
            }
        }

        @Override
        public void onError(Throwable t) {
            result.completeExceptionally(t);
        }

        @Override
        public void onComplete() {
            if (result.isDone()) {
                return;
            }
            byte[] out = new byte[(int) size];
            int at = 0;
            for (ByteBuffer b : parts) {
                int n = b.remaining();
                b.get(out, at, n);
                at += n;
            }
            result.complete(out);
        }
    }

    /** Headers of no answer. */
    static final HttpHeaders NO_HEADERS = HttpHeaders.of(Map.of(), (a, b) -> true);
}
