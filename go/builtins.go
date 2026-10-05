package inorbit

import (
	"bytes"
	"context"
	crand "crypto/rand"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"log/slog"
	"math/rand/v2"
	"net/http"
	"net/url"
	"strconv"
	"strings"
	"sync"
	"time"
)

// The built-in middlewares of docs/config.md section 7.2, the transport after them, and
// the client a resolution builds.

// build makes the client res describes.
func build(cfg *config, res *resolution, explicit bool) (*Client, error) {
	v := res.values
	dur := func(k string) time.Duration { d, _ := v[k].(time.Duration); return d }
	str := func(k string) string { s, _ := v[k].(string); return s }
	boolean := func(k string, fallback bool) bool {
		if b, ok := v[k].(bool); ok {
			return b
		}
		return fallback
	}
	base, err := url.Parse(str("base_url"))
	if err != nil {
		return nil, &ConfigError{Message: "the base URL is not usable: " + err.Error()}
	}
	base.Path, base.RawPath, base.RawQuery = "", "", ""

	hc, err := httpClientFor(cfg, res, explicit)
	if err != nil {
		return nil, err
	}
	ua := userAgent(str("user_agent_suffix"))
	tracingOn := boolean("tracing", true) && cfg.telemetry.Tracer != nil
	metricsOn := boolean("metrics", boolean("tracing", true)) && cfg.telemetry.Meter != nil
	maxRetries, _ := v["max_retries"].(int)
	c := &Client{
		base: base, staticToken: !explicit && res.credential.kind == "static_token",
		timeout: dur("timeout"), totalTimeout: dur("total_timeout"),
		maxRetries: maxRetries, retryBase: dur("retry_base_delay"), retryMax: dur("retry_max_delay"),
		retryAfterMax: dur("retry_after_max"), rateLimit: RateLimitMode(str("rate_limit")),
		userAgent: ua, hooks: cfg.hooks, http: hc, streams: Streams(str("streams")),
		streamIdle: dur("stream_idle_timeout"), tracer: cfg.telemetry.Tracer, meter: cfg.telemetry.Meter,
		tracing: tracingOn, metrics: metricsOn, profile: res.profile,
	}
	if boolean("retry_budget", true) {
		capacity := cfg.budgetCapacity
		if capacity <= 0 {
			capacity = retryBudgetCapacity
		}
		c.budget = newRetryBudget(capacity)
	}
	allow, _ := v["log_allow_headers"].([]string)
	c.log = newLogSink(LogLevel(str("log")), cfg.logger, cfg.redact, res.profile, boolean("log_headers", false), allow)
	c.provider = c.providerFor(cfg, res, hc)
	c.socket = &socketMgr{c: c, calls: map[string]*sockCall{}}

	p := &Pipeline{}
	p.slots = append(p.slots, c.builtIns()...)
	for _, edit := range cfg.pipeline {
		edit(p)
	}
	if p.err != nil {
		return nil, p.err
	}
	c.pipeline = p
	c.chain = p.build(RoundTripperFunc(c.transport))
	d := res.description
	d.Pipeline = p.Names()
	c.config = &ResolvedConfig{d: d}
	return c, nil
}

// providerFor is the token provider the plan names; the cached ones report refreshes to
// the log and the meter.
func (c *Client) providerFor(cfg *config, res *resolution, hc *http.Client) TokenProvider {
	plan := res.credential
	scopes, _ := res.values["scopes"].([]string)
	tokenURL, _ := res.values["token_url"].(string)
	var cache *CachedToken
	var p TokenProvider
	switch plan.kind {
	case "custom":
		return cfg.provider
	case "static_token":
		return NewStaticToken(plan.token)
	case "token_file":
		return NewTokenFile(plan.path)
	case "cli":
		cli := NewCliToken(plan.profile, plan.program)
		cache, p = cli.cache, cli
	default:
		cc := NewClientCredentialsWith(ClientCredentialsConfig{
			KeyID: plan.keyID, KeySecret: plan.keySecret, KeySecretFile: plan.keySecretFile,
			Scopes: scopes, TokenURL: tokenURL, HTTPClient: hc, UserAgent: c.userAgent,
		})
		cache, p = cc.cache, cc
	}
	source := plan.source
	cache.onRefresh = func(err error) {
		if c.meter == nil || !c.metrics {
			return
		}
		attrs := []Attribute{{"inorbit.credential.source", source}}
		if err != nil {
			attrs = append(attrs, Attribute{"error.type", errorKind(err)})
		}
		c.meter.Record(context.Background(), MetricTokenExchanges, 1, attrs...)
	}
	cache.onRefreshFailed = func(err error) {
		c.log.emit(context.Background(), LogWarn, "token_refresh_failed", slog.String("reason", errorKind(err)))
	}
	return p
}

// bufferedBody is an answer's body read whole, which middlewares may read again.
type bufferedBody struct {
	*bytes.Reader
	data []byte
}

func (*bufferedBody) Close() error { return nil }

func buffered(data []byte) *bufferedBody {
	return &bufferedBody{Reader: bytes.NewReader(data), data: data}
}

// ctxErr is the error a done context stands for: the SDK's timeout that ended it, or the
// caller's cancellation.
func ctxErr(ctx context.Context) error {
	var te *TimeoutError
	if cause := context.Cause(ctx); errors.As(cause, &te) {
		cp := *te
		return &cp
	}
	return ctx.Err()
}

// transport is the innermost step: it sends the request with the HTTP client and reads
// the answer whole (16 MiB at most), unless it is a stream.
func (c *Client) transport(req *http.Request) (*http.Response, error) {
	ctx := req.Context()
	resp, err := c.http.Do(req)
	if err != nil {
		if ctx.Err() != nil {
			return nil, ctxErr(ctx)
		}
		if isTimeout(err) {
			return nil, &TimeoutError{Host: c.base.Host, Seconds: seconds(c.timeout)}
		}
		return nil, &ConnectionError{Host: c.base.Host, Err: errors.New(describe(err))}
	}
	if st := callOf(ctx); st != nil && st.info.Stream && st.streamOK != nil && st.streamOK(resp.StatusCode) {
		return resp, nil
	}
	if resp.ContentLength > maxBody {
		_ = resp.Body.Close()
		return nil, &TooLargeError{}
	}
	data, err := io.ReadAll(io.LimitReader(resp.Body, maxBody+1))
	_ = resp.Body.Close()
	if err != nil {
		if ctx.Err() != nil {
			return nil, ctxErr(ctx)
		}
		return nil, &ConnectionError{Host: c.base.Host, Err: errors.New(describe(err))}
	}
	if len(data) > maxBody {
		return nil, &TooLargeError{}
	}
	resp.Body = buffered(data)
	resp.ContentLength = int64(len(data))
	return resp, nil
}

// seconds is d in whole seconds, at least 1.
func seconds(d time.Duration) int {
	return max(1, int((d+time.Second/2)/time.Second))
}

func mw(name string, wrap func(next http.RoundTripper) http.RoundTripper) Middleware {
	return Middleware{Name: name, Wrap: wrap}
}

// builtIns are the client's built-in middlewares, outermost first.
func (c *Client) builtIns() []Middleware {
	return []Middleware{
		mw("request_id", func(next http.RoundTripper) http.RoundTripper {
			return RoundTripperFunc(func(req *http.Request) (*http.Response, error) {
				id := requestID()
				req.Header.Set("X-Request-Id", id)
				if st := callOf(req.Context()); st != nil {
					st.info.RequestID = id
				}
				return next.RoundTrip(req)
			})
		}),
		mw("user_agent", func(next http.RoundTripper) http.RoundTripper {
			return RoundTripperFunc(func(req *http.Request) (*http.Response, error) {
				req.Header.Set("User-Agent", c.userAgent)
				return next.RoundTrip(req)
			})
		}),
		mw("idempotency_key", func(next http.RoundTripper) http.RoundTripper {
			return RoundTripperFunc(func(req *http.Request) (*http.Response, error) {
				st := callOf(req.Context())
				if st == nil || !st.idempotencyOp {
					return next.RoundTrip(req)
				}
				key := st.callerKey
				if key == "" {
					key = uuid4()
				}
				req.Header.Set("Idempotency-Key", key)
				st.info.IdempotencyKey, st.info.Idempotent = key, true
				return next.RoundTrip(req)
			})
		}),
		mw("call_tracing", c.callTracing),
		mw("deadline", c.deadline),
		mw("retry", c.retry),
		mw("auth", c.auth),
		mw("rate_limit", c.rateLimiting),
		mw("attempt_tracing", c.attemptTracing),
		mw("logging", c.logging),
		mw("hooks", c.hooking),
		mw("timeout", c.attemptTimeout),
	}
}

// uuid4 is a random UUID, version 4.
func uuid4() string {
	var b [16]byte
	_, _ = crand.Read(b[:])
	b[6] = b[6]&0x0f | 0x40
	b[8] = b[8]&0x3f | 0x80
	return fmt.Sprintf("%x-%x-%x-%x-%x", b[0:4], b[4:6], b[6:8], b[8:10], b[10:16])
}

// bounded runs next with ctx ending after d, the end's cause being cause. A stream's
// context lives on until its body is closed; any other ends when next returns.
func bounded(req *http.Request, next http.RoundTripper, d time.Duration, cause error) (*http.Response, error) {
	ctx, cancel := context.WithCancelCause(req.Context())
	timer := time.AfterFunc(d, func() { cancel(cause) })
	resp, err := next.RoundTrip(req.WithContext(ctx))
	timer.Stop()
	if err != nil {
		if errors.Is(err, context.Canceled) && errors.Is(context.Cause(ctx), cause) {
			err = ctxErr(ctx)
		}
		cancel(nil)
		return nil, err
	}
	if _, ok := resp.Body.(*bufferedBody); ok || resp.Body == nil {
		cancel(nil)
		return resp, nil
	}
	resp.Body = &onClose{ReadCloser: resp.Body, done: func() { cancel(nil) }}
	return resp, nil
}

// onClose runs done once, when the body is closed.
type onClose struct {
	io.ReadCloser
	once sync.Once
	done func()
}

func (o *onClose) Close() error {
	err := o.ReadCloser.Close()
	o.once.Do(o.done)
	return err
}

// Write lets an upgraded connection's body stay writable.
func (o *onClose) Write(p []byte) (int, error) {
	if w, ok := o.ReadCloser.(io.Writer); ok {
		return w.Write(p)
	}
	return 0, errors.New("the body is not writable")
}

func (c *Client) deadline(next http.RoundTripper) http.RoundTripper {
	return RoundTripperFunc(func(req *http.Request) (*http.Response, error) {
		st := callOf(req.Context())
		total := c.totalTimeout
		if st != nil && st.timeout > 0 && st.timeout < total {
			total = st.timeout
		}
		at := time.Now().Add(total)
		if d, ok := req.Context().Deadline(); ok && d.Before(at) {
			at = d
		}
		if st != nil {
			st.info.Deadline = at
		}
		return bounded(req, next, total, &TimeoutError{Host: c.base.Host, Seconds: seconds(total), total: true})
	})
}

func (c *Client) attemptTimeout(next http.RoundTripper) http.RoundTripper {
	return RoundTripperFunc(func(req *http.Request) (*http.Response, error) {
		d := c.timeout
		st := callOf(req.Context())
		if st != nil && st.timeout > 0 {
			d = st.timeout
		}
		limit := d
		if st != nil && !st.info.Deadline.IsZero() {
			limit = max(0, min(d, time.Until(st.info.Deadline)))
		}
		return bounded(req, next, limit, &TimeoutError{Host: c.base.Host, Seconds: seconds(d)})
	})
}

// retriable reports whether err is worth another attempt: a connection failure, or an
// attempt (not the call) that ran out of time.
func retriable(err error) bool {
	var ce *ConnectionError
	var te *TimeoutError
	return errors.As(err, &ce) || errors.As(err, &te) && !te.total
}

// askedWait is the wait a retry-worthy answer asks for: Retry-After, else the envelope's
// retry detail.
func askedWait(resp *http.Response) (time.Duration, bool) {
	if d, ok := retryAfter(resp.Header, time.Now()); ok {
		return d, true
	}
	b, ok := resp.Body.(*bufferedBody)
	if !ok {
		return 0, false
	}
	var env struct {
		Details []struct {
			Type         string      `json:"type"`
			AfterSeconds json.Number `json:"after_seconds"`
		} `json:"details"`
	}
	if json.Unmarshal(b.data, &env) != nil {
		return 0, false
	}
	for _, d := range env.Details {
		if d.Type == DetailRetry {
			if s, err := d.AfterSeconds.Float64(); err == nil && s >= 0 {
				return time.Duration(s * float64(time.Second)), true
			}
		}
	}
	return 0, false
}

func (c *Client) retry(next http.RoundTripper) http.RoundTripper {
	return RoundTripperFunc(func(req *http.Request) (*http.Response, error) {
		ctx := req.Context()
		st := callOf(ctx)
		if st == nil {
			st = &callState{}
		}
		retries, lastCost, reason := 0, 0, ""
		for attempt := 1; ; attempt++ {
			st.attempts = attempt
			areq := req.Clone(context.WithValue(ctx, attemptCtxKey{}, attemptInfo{attempt: attempt, reason: reason, stage: StagePerRetry}))
			if req.GetBody != nil && req.Body != nil && req.Body != http.NoBody {
				areq.Body, _ = req.GetBody()
			}
			resp, err := next.RoundTrip(areq)
			var wait time.Duration
			cost := 10
			if err == nil {
				if !retryableStatus(resp.StatusCode) || !st.info.Idempotent {
					if resp.StatusCode >= 200 && resp.StatusCode < 300 {
						if retries == 0 {
							c.budget.give(1)
						} else {
							c.budget.give(lastCost)
						}
					}
					return resp, nil
				}
				reason = strconv.Itoa(resp.StatusCode)
				asked, has := askedWait(resp)
				if has && asked > c.retryAfterMax {
					return resp, nil
				}
				if resp.StatusCode == http.StatusTooManyRequests || resp.StatusCode == http.StatusServiceUnavailable && has {
					cost = 5
				}
				wait = asked
				if !has {
					wait = backoffWith(retries, c.retryBase, c.retryMax)
				}
			} else {
				if !retriable(err) || !st.info.Idempotent || ctx.Err() != nil {
					return nil, err
				}
				reason = errorKind(err)
				wait = backoffWith(retries, c.retryBase, c.retryMax)
			}
			if retries >= c.maxRetries {
				return resp, err
			}
			deadline := st.info.Deadline
			if d, ok := ctx.Deadline(); ok && (deadline.IsZero() || d.Before(deadline)) {
				deadline = d
			}
			if !deadline.IsZero() && time.Now().Add(wait).After(deadline) {
				return resp, err
			}
			if !c.budget.take(cost) {
				return resp, err
			}
			lastCost = cost
			a := attemptFor(areq, st)
			for _, h := range c.hooks {
				if rh, ok := h.(RetryHook); ok {
					rh.OnRetry(a, reason, wait)
				}
			}
			c.log.emit(ctx, LogWarn, "retry",
				slog.String("operation", st.info.Operation), slog.Int("attempt", attempt),
				slog.String("reason", reason), slog.Int64("delay_ms", wait.Milliseconds()),
				slog.String("request_id", st.info.RequestID))
			if c.meter != nil && c.metrics {
				c.meter.Record(ctx, MetricRetries, 1,
					Attribute{"inorbit.operation", st.info.Operation}, Attribute{"inorbit.retry.reason", reason})
			}
			if resp != nil {
				_ = resp.Body.Close()
			}
			if serr := sleep(ctx, wait); serr != nil {
				return nil, ctxErr(ctx)
			}
			retries++
		}
	})
}

// attemptFor is an attempt as hooks see it.
func attemptFor(req *http.Request, st *callState) Attempt {
	info, _ := CallInfoFrom(req.Context())
	return Attempt{
		Operation: st.info.Operation, Method: req.Method, Path: req.URL.EscapedPath(),
		Number: max(1, info.Attempt), RequestID: st.info.RequestID,
		IdempotencyKey: st.info.IdempotencyKey, Stage: info.Stage,
	}
}

func (c *Client) token(ctx context.Context) (Token, error) {
	tok, err := c.provider.Token(ctx)
	if err == nil {
		return tok, nil
	}
	if ctx.Err() != nil {
		return Token{}, ctxErr(ctx)
	}
	var aerr *AuthError
	if !errors.As(err, &aerr) {
		err = &AuthError{Message: "the token provider failed: " + err.Error(), Err: err}
	}
	return Token{}, err
}

func (c *Client) auth(next http.RoundTripper) http.RoundTripper {
	return RoundTripperFunc(func(req *http.Request) (*http.Response, error) {
		send := func(r *http.Request) (*http.Response, error) {
			tok, err := c.token(r.Context())
			if err != nil {
				return nil, err
			}
			r.Header.Set("Authorization", "Bearer "+tok.Access)
			return next.RoundTrip(r)
		}
		again := req.Clone(req.Context())
		if req.GetBody != nil && req.Body != nil && req.Body != http.NoBody {
			again.Body, _ = req.GetBody()
		}
		resp, err := send(req)
		st := callOf(req.Context())
		if err != nil || resp.StatusCode != http.StatusUnauthorized || st == nil || st.refreshed {
			return resp, err
		}
		// One fresh token after a 401: not a retry, so it draws nothing from the budget.
		st.refreshed = true
		_ = resp.Body.Close()
		if c.staticToken {
			return nil, &AuthError{
				Message:    "the API refused the token (HTTP 401): it has expired or was revoked; create a new one in the console or with `iohr token create`, and set it again",
				OAuthError: "unauthenticated",
			}
		}
		if inv, ok := c.provider.(Invalidator); ok {
			inv.Invalidate()
		}
		return send(again)
	})
}

func (c *Client) rateLimiting(next http.RoundTripper) http.RoundTripper {
	return RoundTripperFunc(func(req *http.Request) (*http.Response, error) {
		if c.rateLimit == RateLimitOff {
			return next.RoundTrip(req)
		}
		ctx := req.Context()
		st := callOf(ctx)
		if c.rateLimit == RateLimitWait {
			c.latestMu.Lock()
			latest, reset := c.latest, c.latestReset
			c.latestMu.Unlock()
			now := time.Now()
			if latest != nil && latest.Remaining != nil && *latest.Remaining == 0 && !reset.IsZero() && reset.After(now) {
				wait := reset.Sub(now) + time.Duration(rand.Int64N(int64(100*time.Millisecond)+1)) //nolint:gosec // jitter, not a secret
				if st != nil && !st.info.Deadline.IsZero() && now.Add(wait).After(st.info.Deadline) {
					return nil, &TimeoutError{Host: c.base.Host, Seconds: seconds(c.totalTimeout), Waiting: "the rate-limit window to reset", total: true}
				}
				info, _ := CallInfoFrom(ctx)
				id := ""
				if st != nil {
					id = st.info.RequestID
				}
				c.log.emit(ctx, LogWarn, "rate_limit_wait", slog.Int("attempt", max(1, info.Attempt)),
					slog.String("reason", "rate_limit"), slog.Int64("delay_ms", wait.Milliseconds()), slog.String("request_id", id))
				if err := sleep(ctx, wait); err != nil {
					return nil, ctxErr(ctx)
				}
			}
		}
		resp, err := next.RoundTrip(req)
		if err != nil {
			return nil, err
		}
		snap := ParseRateLimit(resp.Header)
		if snap == nil {
			return resp, nil
		}
		c.latestMu.Lock()
		c.latest = snap
		c.latestReset = time.Time{}
		if snap.Reset != nil {
			c.latestReset = time.Now().Add(*snap.Reset)
		}
		c.latestMu.Unlock()
		if st != nil {
			st.rateLimit = snap
		}
		return resp, nil
	})
}

func (c *Client) callTracing(next http.RoundTripper) http.RoundTripper {
	return RoundTripperFunc(func(req *http.Request) (*http.Response, error) {
		if !c.tracing && !c.metrics {
			return next.RoundTrip(req)
		}
		ctx := req.Context()
		st := callOf(ctx)
		if st == nil {
			return next.RoundTrip(req)
		}
		started := time.Now()
		record := func(errType string) {
			if !c.metrics {
				return
			}
			attrs := []Attribute{{"inorbit.operation", st.info.Operation}}
			if errType != "" {
				attrs = append(attrs, Attribute{"error.type", errType})
			}
			c.meter.Record(ctx, MetricCallDuration, time.Since(started).Seconds(), attrs...)
		}
		var span Span
		if c.tracing {
			attrs := []Attribute{{"inorbit.operation", st.info.Operation}}
			if st.info.RequestID != "" {
				attrs = append(attrs, Attribute{"inorbit.request_id", st.info.RequestID})
			}
			parent := ""
			if _, _, ok := ParseTraceparent(st.traceparent); ok {
				parent = strings.TrimSpace(st.traceparent)
			}
			var sctx context.Context
			sctx, span = c.tracer.Start(ctx, SpanStart{Name: st.info.Operation, Kind: SpanKindInternal, Attributes: attrs, Parent: parent})
			req = req.WithContext(sctx)
		}
		end := func(errType string) {
			record(errType)
			if span != nil {
				if errType != "" {
					span.SetError(errType)
				}
				span.End()
			}
		}
		resp, err := next.RoundTrip(req)
		if err != nil {
			end(errorType(err))
			return nil, err
		}
		errType := ""
		if resp.StatusCode >= 400 {
			errType = strconv.Itoa(resp.StatusCode)
		}
		if _, ok := resp.Body.(*bufferedBody); !ok && resp.Body != nil {
			// A stream's span covers it to its end.
			resp.Body = &onClose{ReadCloser: resp.Body, done: func() { end(errType) }}
			return resp, nil
		}
		end(errType)
		return resp, nil
	})
}

func (c *Client) attemptTracing(next http.RoundTripper) http.RoundTripper {
	return RoundTripperFunc(func(req *http.Request) (*http.Response, error) {
		if !c.tracing && !c.metrics {
			return next.RoundTrip(req)
		}
		ctx := req.Context()
		st := callOf(ctx)
		info, _ := CallInfoFrom(ctx)
		port := int64(defaultPort(req.URL))
		host := req.URL.Hostname()
		common := []Attribute{{"http.request.method", req.Method}, {"server.address", host}, {"server.port", port}}
		started := time.Now()
		measure := func(status int, errType string) {
			if !c.metrics {
				return
			}
			attrs := append([]Attribute(nil), common...)
			if status != 0 {
				attrs = append(attrs, Attribute{"http.response.status_code", int64(status)})
			}
			if errType != "" {
				attrs = append(attrs, Attribute{"error.type", errType})
			}
			c.meter.Record(ctx, MetricRequestDuration, time.Since(started).Seconds(), attrs...)
		}
		var span Span
		if c.tracing {
			attrs := append(append([]Attribute(nil), common...), Attribute{"url.full", redactedURL(req.URL)})
			name := req.Method
			if info.Template != "" {
				attrs = append(attrs, Attribute{"url.template", info.Template})
				name += " " + info.Template
			}
			if info.Attempt > 1 {
				attrs = append(attrs, Attribute{"http.request.resend_count", int64(info.Attempt - 1)})
			}
			var sctx context.Context
			sctx, span = c.tracer.Start(ctx, SpanStart{Name: name, Kind: SpanKindClient, Attributes: attrs})
			req = req.WithContext(sctx)
			if tp, ts := span.Traceparent(); tp != "" {
				req.Header.Set("Traceparent", tp)
				if ts != "" {
					req.Header.Set("Tracestate", ts)
				}
				if st != nil {
					st.traceID, st.spanID, _ = ParseTraceparent(tp)
				}
			}
		}
		resp, err := next.RoundTrip(req)
		if err != nil {
			measure(0, errorType(err))
			if span != nil {
				span.SetError(errorType(err))
				span.End()
			}
			return nil, err
		}
		errType := ""
		if resp.StatusCode >= 400 {
			errType = strconv.Itoa(resp.StatusCode)
		}
		measure(resp.StatusCode, errType)
		if span != nil {
			span.SetAttributes(Attribute{"http.response.status_code", int64(resp.StatusCode)})
			if id := resp.Header.Get("X-Request-Id"); id != "" {
				span.SetAttributes(Attribute{"inorbit.server_request_id", id})
			}
			if strings.TrimSpace(resp.Header.Get("Idempotency-Replayed")) == "true" {
				span.SetAttributes(Attribute{"inorbit.idempotency_replayed", true})
			}
			if errType != "" {
				span.SetError(errType)
			}
			span.End()
		}
		return resp, nil
	})
}

func (c *Client) logging(next http.RoundTripper) http.RoundTripper {
	return RoundTripperFunc(func(req *http.Request) (*http.Response, error) {
		if !c.log.on(LogDebug) {
			return next.RoundTrip(req)
		}
		ctx := req.Context()
		info, _ := CallInfoFrom(ctx)
		st := callOf(ctx)
		base := []slog.Attr{
			slog.String("operation", info.Operation), slog.String("method", req.Method),
			slog.String("path", req.URL.EscapedPath()), slog.Int("attempt", max(1, info.Attempt)),
			slog.String("request_id", info.RequestID),
		}
		if st != nil && st.traceID != "" {
			base = append(base, slog.String("trace_id", st.traceID), slog.String("span_id", st.spanID))
		}
		attrs := append([]slog.Attr(nil), base...)
		if c.log.headers {
			attrs = append(attrs, c.log.show(req.Header, false))
		}
		c.log.emit(ctx, LogDebug, "request", attrs...)
		started := time.Now()
		resp, err := next.RoundTrip(req)
		if err != nil {
			return nil, err
		}
		attrs = append(append([]slog.Attr(nil), base...),
			slog.Int("status", resp.StatusCode), slog.Int64("duration_ms", time.Since(started).Milliseconds()))
		if id := resp.Header.Get("X-Request-Id"); id != "" {
			attrs = append(attrs, slog.String("server_request_id", id))
		}
		if c.log.headers {
			attrs = append(attrs, c.log.show(resp.Header, true))
		}
		c.log.emit(ctx, LogDebug, "response", attrs...)
		return resp, nil
	})
}

func (c *Client) hooking(next http.RoundTripper) http.RoundTripper {
	return RoundTripperFunc(func(req *http.Request) (*http.Response, error) {
		st := callOf(req.Context())
		if len(c.hooks) == 0 || st == nil {
			return next.RoundTrip(req)
		}
		a := attemptFor(req, st)
		for _, h := range c.hooks {
			h.OnRequest(a)
		}
		resp, err := next.RoundTrip(req)
		if err != nil {
			return nil, err
		}
		raw := &RawResponse{
			Status: resp.StatusCode, Header: resp.Header, RequestID: st.info.RequestID,
			ServerRequestID: resp.Header.Get("X-Request-Id"), Attempts: a.Number,
			IdempotencyKey: st.info.IdempotencyKey,
		}
		if b, ok := resp.Body.(*bufferedBody); ok {
			raw.Body = b.data
		}
		for _, h := range c.hooks {
			h.OnResponse(a, raw)
		}
		return resp, nil
	})
}
