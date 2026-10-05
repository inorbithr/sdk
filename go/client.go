package inorbit

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"log/slog"
	"net"
	"net/http"
	"net/url"
	"os"
	"runtime"
	"strings"
	"sync"
	"time"
)

// DefaultBaseURL is where the API is.
const DefaultBaseURL = "https://api.inorbit.hr"

// maxBody is the largest answer read, 16 MiB.
const maxBody = 16 << 20

// Operation is one call, as a generated surface builds it.
type Operation struct {
	// Name is what hooks see (radar.list_digests).
	Name string

	// Method is the HTTP method.
	Method string

	// Path is the path, parameters bound and encoded.
	Path string

	// Template is the path before binding (/v1/radar/digests/{digest_id}), for span
	// names and url.template; empty when not known.
	Template string

	// Query holds the query parameters.
	Query url.Values

	// Body is the JSON body, nil for none.
	Body any

	// Scopes are the scopes the operation needs, for the record.
	Scopes []string

	// Idempotent retries the call like an idempotent method although its method is not.
	Idempotent bool

	// IdempotencyKey marks an operation that takes an Idempotency-Key header (RFC 0033):
	// the call sends one, the caller's or a generated one, the same on every attempt, and
	// is retried like an idempotent method.
	IdempotencyKey bool

	// RPC is the RPC's full name (iohr.events.v1.EventsService/StreamEvents), which a
	// stream over the socket calls; empty for the gateway's own routes.
	RPC string

	// Fields are the path and query parameters as the request message's fields, typed and
	// in wire names, unset ones left out: the body of a socket call frame.
	Fields map[string]any
}

func (op Operation) retrySafe() bool {
	switch op.Method {
	case http.MethodGet, http.MethodPut, http.MethodDelete, http.MethodHead:
		return true
	}
	return op.Idempotent
}

// Client is a client for one credential. It is safe for concurrent use and holds no
// per-call state.
type Client struct {
	base          *url.URL
	provider      TokenProvider
	staticToken   bool // a refused static token from Load ends the call with an AuthError
	timeout       time.Duration
	totalTimeout  time.Duration
	maxRetries    int
	retryBase     time.Duration
	retryMax      time.Duration
	retryAfterMax time.Duration
	budget        *retryBudget
	rateLimit     RateLimitMode
	userAgent     string
	hooks         []Hook
	http          *http.Client
	streams       Streams
	streamIdle    time.Duration
	socket        *socketMgr
	log           *logSink
	tracer        Tracer
	meter         Meter
	tracing       bool
	metrics       bool
	profile       string
	pipeline      *Pipeline
	chain         http.RoundTripper
	config        *ResolvedConfig

	latestMu    sync.Mutex
	latest      *RateLimit
	latestReset time.Time
}

// NewClient returns a client configured by opts and nothing else: no environment, no
// file. It needs a credential: WithToken, WithKey with WithScopes, or WithTokenProvider.
// Libraries and tests use it; applications use Load.
func NewClient(opts ...Option) (*Client, error) {
	cfg := newConfig(opts)
	if err := checkExplicit(cfg); err != nil {
		return nil, err
	}
	res, err := resolve(cfg.input(true))
	if err != nil {
		return nil, err
	}
	return build(cfg, res, true)
}

// checkExplicit holds NewClient to the checks and messages it always had.
func checkExplicit(cfg *config) error {
	str := func(k string) string { s, _ := cfg.code[k].(string); return s }
	if u, ok := cfg.code["base_url"].(string); ok {
		if _, err := checkURL("the base URL", u, true); err != nil {
			return err
		}
	}
	if u, ok := cfg.code["token_url"].(string); ok {
		if _, err := checkURL("the token URL", u, false); err != nil {
			return err
		}
	}
	if s, ok := cfg.code["streams"].(string); ok && s != string(StreamsSSE) && s != string(StreamsSocket) {
		return &ConfigError{Message: fmt.Sprintf("streams %q is not usable: sse or socket", s)}
	}
	if d, ok := cfg.code["stream_idle_timeout"].(time.Duration); ok && d <= 0 {
		return &ConfigError{Message: "the stream idle timeout must be above zero"}
	}
	switch {
	case cfg.provider != nil, str("token") != "", str("token_file") != "":
	case str("key_id") != "" && (str("key_secret") != "" || str("key_secret_file") != ""):
		if s, _ := cfg.code["scopes"].([]string); len(s) == 0 {
			return &ConfigError{Message: `no scopes: set INORBIT_SCOPES (space-separated, such as "identity:read account:read")`}
		}
	default:
		return &ConfigError{Message: "no credentials: set INORBIT_TOKEN, or INORBIT_KEY_ID and INORBIT_KEY_SECRET"}
	}
	return nil
}

// Load returns a client configured from code (opts), the environment, the iohr config
// file and the iohr login, each setting from the first that sets it (docs/config.md
// section 2). It reads the environment and files now, and checks that iohr exists when
// the credential chain reaches it; it contacts no host until the first call.
//
// A ConfigError lists every problem found, or every credential source tried.
func Load(ctx context.Context, opts ...Option) (*Client, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	cfg := newConfig(opts)
	res, err := resolve(cfg.input(false))
	if err != nil {
		return nil, err
	}
	return build(cfg, res, false)
}

// LoadConfig resolves the configuration Load would, without building a client: what
// iohr sdk config prints.
func LoadConfig(ctx context.Context, opts ...Option) (*ResolvedConfig, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	cfg := newConfig(opts)
	res, err := resolve(cfg.input(false))
	if err != nil {
		return nil, err
	}
	p := &Pipeline{}
	for _, name := range builtIns {
		p.slots = append(p.slots, Middleware{Name: name, Wrap: passThrough})
	}
	for _, edit := range cfg.pipeline {
		edit(p)
	}
	if p.err != nil {
		return nil, p.err
	}
	res.description.Pipeline = p.Names()
	return &ResolvedConfig{d: res.description}, nil
}

func passThrough(next http.RoundTripper) http.RoundTripper { return next }

func getenv(k string) string { return os.Getenv(k) }

// FromEnv returns a client from the environment. A named profile (its environment
// name, such as ACME_CI) reads INORBIT_ACME_CI_TOKEN, or INORBIT_ACME_CI_KEY_ID,
// _KEY_SECRET and _SCOPES, and nothing else; an empty profile reads the bare INORBIT_*
// names. INORBIT_BASE_URL and INORBIT_TOKEN_URL apply to every profile. opts apply
// after the environment. It is kept as it is; Load reads more and is the one to use.
func FromEnv(profile string, opts ...Option) (*Client, error) {
	prefix := "INORBIT_"
	if profile != "" {
		prefix = "INORBIT_" + profile + "_"
	}
	v := func(name string) string { return getenv(prefix + name) }
	shared := func(name string) string {
		if s := v(name); s != "" {
			return s
		}
		return getenv("INORBIT_" + name)
	}
	var env []Option
	if u := shared("BASE_URL"); u != "" {
		env = append(env, WithBaseURL(u))
	}
	if u := shared("TOKEN_URL"); u != "" {
		env = append(env, WithTokenURL(u))
	}
	switch {
	case v("TOKEN") != "":
		env = append(env, WithToken(v("TOKEN")))
	case v("KEY_ID") != "" && v("KEY_SECRET") != "":
		scopes := strings.Fields(v("SCOPES"))
		if len(scopes) == 0 {
			return nil, &ConfigError{Message: fmt.Sprintf(`no scopes: set %sSCOPES (space-separated, such as "identity:read account:read")`, prefix)}
		}
		env = append(env, WithKey(v("KEY_ID"), v("KEY_SECRET")), WithScopes(scopes...))
	default:
		return nil, &ConfigError{Message: fmt.Sprintf("no credentials: set %sTOKEN, or %sKEY_ID and %sKEY_SECRET", prefix, prefix, prefix)}
	}
	return NewClient(append(env, opts...)...)
}

// ResolvedConfig is what a client uses and where each value came from.
type ResolvedConfig struct {
	d Description
}

// Describe returns the configuration as docs/config.md section 2.6 documents it, secrets
// redacted; encoding/json writes it in that form.
func (r *ResolvedConfig) Describe() Description {
	d := r.d
	d.Settings = make(map[string]DescribedSetting, len(r.d.Settings))
	for k, v := range r.d.Settings {
		d.Settings[k] = v
	}
	d.Pipeline = append([]string(nil), r.d.Pipeline...)
	d.Ignored = append([]IgnoredSetting{}, r.d.Ignored...)
	return d
}

// MarshalJSON writes the Describe document.
func (r *ResolvedConfig) MarshalJSON() ([]byte, error) { return json.Marshal(r.Describe()) }

// BaseURL is the API's origin this client calls.
func (c *Client) BaseURL() string { return c.base.String() }

// Config is what this client uses and where each value came from (Describe), secrets
// redacted.
func (c *Client) Config() *ResolvedConfig { return c.config }

// RateLimit is the latest rate-limit snapshot any answer carried, or nil.
func (c *Client) RateLimit() *RateLimit {
	c.latestMu.Lock()
	defer c.latestMu.Unlock()
	return c.latest
}

// Call calls op on c and reads its JSON answer into a T. Per-call options travel in ctx
// (WithCallOptions).
func Call[T any](ctx context.Context, c *Client, op Operation) (*Response[T], error) {
	raw, err := c.Send(ctx, op)
	if err != nil {
		return nil, err
	}
	var value T
	if len(bytes.TrimSpace(raw.Body)) > 0 {
		if err := json.Unmarshal(raw.Body, &value); err != nil {
			return nil, &DecodeError{Reason: err.Error(), Raw: raw}
		}
	}
	return &Response[T]{Value: value, Raw: raw}, nil
}

// Send calls op and returns the answer as it came, a 2xx one only; anything else is an
// error.
func (c *Client) Send(ctx context.Context, op Operation) (*RawResponse, error) {
	started := time.Now()
	resp, st, err := c.do(ctx, op, callKind{})
	if err != nil {
		return nil, err
	}
	raw := c.rawOf(resp, st)
	if raw.Status >= 200 && raw.Status < 300 {
		c.log.emit(ctx, LogInfo, "call", c.callAttrs(st, raw, nil, started)...)
		return raw, nil
	}
	apiErr := newAPIError(raw)
	c.failed(ctx, op, st, apiErr, started)
	return nil, apiErr
}

// callKind says how a call's answer is read.
type callKind struct {
	stream bool
	header http.Header
	ok     func(int) bool // the statuses a stream accepts, its body left unread
}

// do runs op through the pipeline. A failure is reported to the hooks and the log here;
// an answer comes back whatever its status, its body buffered, or the stream's when
// kind accepts its status.
func (c *Client) do(ctx context.Context, op Operation, kind callKind) (*http.Response, *callState, error) {
	started := time.Now()
	name := op.Name
	if name == "" {
		name = op.Path
	}
	opts := callOptionsFrom(ctx)
	st := &callState{
		info: CallInfo{
			Operation: name, Template: op.Template, Idempotent: op.retrySafe(), Stream: kind.stream,
			Profile: c.profile, Tracing: c.tracing, Metrics: c.metrics,
		},
		idempotencyOp: op.IdempotencyKey, callerKey: opts.IdempotencyKey,
		timeout: opts.Timeout, traceparent: opts.Traceparent, streamOK: kind.ok,
	}
	if opts.IdempotencyKey != "" && !op.IdempotencyKey {
		return nil, st, &ConfigError{Message: name + " does not take an idempotency key: the API would ignore it, so repeating the call would not be safe"}
	}
	u, err := c.url(op)
	if err != nil {
		return nil, st, err
	}
	var body []byte
	if op.Body != nil {
		body, err = json.Marshal(op.Body)
		if err != nil {
			return nil, st, &ConfigError{Message: "the body could not be written as JSON: " + err.Error()}
		}
	}
	var reader io.Reader
	if body != nil {
		reader = bytes.NewReader(body)
	}
	req, err := http.NewRequestWithContext(withCall(ctx, st), op.Method, u.String(), reader)
	if err != nil {
		return nil, st, &ConfigError{Message: "the request could not be built: " + err.Error()}
	}
	req.Header.Set("Accept", "application/json")
	if body != nil {
		req.Header.Set("Content-Type", "application/json")
	}
	for k, v := range kind.header {
		req.Header[k] = v
	}
	if opts.Traceparent != "" {
		req.Header.Set("Traceparent", opts.Traceparent)
	}
	resp, err := c.chain.RoundTrip(req)
	if err != nil {
		err = annotate(err, st)
		c.failed(ctx, op, st, err, started)
		return nil, st, err
	}
	return resp, st, nil
}

// annotate puts the call's request id and idempotency key on err.
func annotate(err error, st *callState) error {
	var ce *ConnectionError
	var te *TimeoutError
	switch {
	case errors.As(err, &ce):
		if ce.RequestID == "" {
			ce.RequestID = st.info.RequestID
		}
		if ce.IdempotencyKey == "" {
			ce.IdempotencyKey = st.info.IdempotencyKey
		}
	case errors.As(err, &te):
		if te.RequestID == "" {
			te.RequestID = st.info.RequestID
		}
		if te.IdempotencyKey == "" {
			te.IdempotencyKey = st.info.IdempotencyKey
		}
	}
	return err
}

// rawOf is the raw answer resp stands for, its body read.
func (c *Client) rawOf(resp *http.Response, st *callState) *RawResponse {
	var data []byte
	if b, ok := resp.Body.(*bufferedBody); ok {
		data = b.data
	} else if resp.Body != nil {
		data, _ = io.ReadAll(io.LimitReader(resp.Body, maxBody))
		_ = resp.Body.Close()
	}
	return &RawResponse{
		Status: resp.StatusCode, Header: resp.Header, Body: data, RequestID: st.info.RequestID,
		ServerRequestID: resp.Header.Get("X-Request-Id"), Attempts: max(st.attempts, 1),
		IdempotencyKey:      st.info.IdempotencyKey,
		IdempotencyReplayed: strings.TrimSpace(resp.Header.Get("Idempotency-Replayed")) == "true",
		RateLimit:           st.rateLimit,
	}
}

func (c *Client) callAttrs(st *callState, raw *RawResponse, err error, started time.Time) []slog.Attr {
	attrs := []slog.Attr{slog.String("operation", st.info.Operation)}
	if raw != nil {
		attrs = append(attrs, slog.Int("status", raw.Status))
	}
	if err != nil {
		attrs = append(attrs, slog.String("error_kind", errorKind(err)))
		if a, ok := asAPIError(err); ok {
			attrs = append(attrs, slog.String("error_code", string(a.Code)), slog.Int("status", a.Status))
			raw = a.Raw
		}
	}
	attrs = append(attrs,
		slog.Int("attempts", max(st.attempts, 1)),
		slog.Int64("duration_ms", time.Since(started).Milliseconds()),
		slog.String("request_id", st.info.RequestID),
	)
	if raw != nil && raw.ServerRequestID != "" {
		attrs = append(attrs, slog.String("server_request_id", raw.ServerRequestID))
	}
	return attrs
}

// failed tells the hooks and the log that a call failed for good.
func (c *Client) failed(ctx context.Context, op Operation, st *callState, err error, started time.Time) {
	if c.pipeline.has("hooks") {
		a := Attempt{
			Operation: st.info.Operation, Method: op.Method, Path: op.Path, Number: max(st.attempts, 1),
			RequestID: st.info.RequestID, IdempotencyKey: st.info.IdempotencyKey, Stage: StagePerRetry,
		}
		for _, h := range c.hooks {
			h.OnError(a, err)
		}
	}
	if !c.log.on(LogError) {
		return
	}
	c.log.emit(ctx, LogInfo, "call", c.callAttrs(st, nil, err, started)...)
	attrs := []slog.Attr{slog.String("operation", st.info.Operation), slog.String("error_kind", errorKind(err))}
	if a, ok := asAPIError(err); ok {
		attrs = append(attrs, slog.String("error_code", string(a.Code)), slog.Int("status", a.Status))
	}
	attrs = append(attrs, slog.String("request_id", st.info.RequestID))
	c.log.emit(ctx, LogError, "call_failed", attrs...)
}

func (c *Client) url(op Operation) (*url.URL, error) {
	p := op.Path
	if !strings.HasPrefix(p, "/") || strings.HasPrefix(p, "//") || strings.ContainsAny(p, "?#") {
		return nil, &ConfigError{Message: fmt.Sprintf("the path %q is not usable: it starts with one / and has no query", p)}
	}
	u := *c.base
	u.RawPath = p
	path, err := url.PathUnescape(p)
	if err != nil {
		return nil, &ConfigError{Message: fmt.Sprintf("the path %q is not usable: %v", p, err)}
	}
	u.Path = path
	if len(op.Query) > 0 {
		u.RawQuery = op.Query.Encode()
	}
	return &u, nil
}

func isTimeout(err error) bool {
	var ne net.Error
	return errors.As(err, &ne) && ne.Timeout()
}

func checkURL(what, raw string, originOnly bool) (*url.URL, error) {
	u, err := url.Parse(raw)
	if err != nil {
		return nil, &ConfigError{Message: fmt.Sprintf("%s is not usable: %v", what, err)}
	}
	host := u.Hostname()
	loopback := host == "localhost" || host == "::1" || strings.HasPrefix(host, "127.")
	if u.Scheme != "https" && (u.Scheme != "http" || !loopback) {
		return nil, &ConfigError{Message: what + " is not usable: it must use https (plain http only to this machine)"}
	}
	if u.User != nil || u.Fragment != "" || host == "" {
		return nil, &ConfigError{Message: what + " is not usable: it needs a host and must not carry credentials or a fragment"}
	}
	if originOnly && (strings.TrimSuffix(u.Path, "/") != "" || u.RawQuery != "") {
		return nil, &ConfigError{Message: what + " is not usable: it is an origin only, such as https://api.inorbit.hr"}
	}
	if originOnly {
		u.Path, u.RawPath = "", ""
	}
	return u, nil
}

// userAgent is inorbithr-sdk-go/<version> go/<version> <os>/<arch>[ <suffix>], in the
// vocabulary every runtime shares (docs/config.md section 7.6).
func userAgent(suffix string) string {
	ua := fmt.Sprintf("inorbithr-sdk-go/%s go/%s %s/%s", SDKVersion,
		strings.TrimPrefix(runtime.Version(), "go"), uaOS(runtime.GOOS), uaArch(runtime.GOARCH))
	if suffix != "" {
		ua += " " + suffix
	}
	return ua
}

func uaOS(goos string) string {
	switch goos {
	case "linux", "windows", "freebsd", "android", "ios":
		return goos
	case "darwin":
		return "macos"
	}
	return "other"
}

func uaArch(goarch string) string {
	switch goarch {
	case "amd64":
		return "x86_64"
	case "arm64":
		return "aarch64"
	case "386":
		return "x86"
	case "arm", "riscv64":
		return goarch
	}
	return "other"
}
