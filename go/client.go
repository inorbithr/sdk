package inorbit

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/url"
	"os"
	"runtime"
	"strings"
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

	// Query holds the query parameters.
	Query url.Values

	// Body is the JSON body, nil for none.
	Body any

	// Scopes are the scopes the operation needs, for the record.
	Scopes []string

	// Idempotent retries the call like an idempotent method although its method is not.
	Idempotent bool
}

func (op Operation) retrySafe() bool {
	switch op.Method {
	case http.MethodGet, http.MethodPut, http.MethodDelete, http.MethodHead:
		return true
	}
	return op.Idempotent
}

// config is what the options set.
type config struct {
	token      string
	keyID      string
	keySecret  string
	scopes     []string
	provider   TokenProvider
	baseURL    string
	tokenURL   string
	timeout    time.Duration
	maxRetries int
	uaSuffix   string
	hooks      []Hook
	http       *http.Client
}

// Option configures a Client.
type Option func(*config)

// WithToken authenticates with an API token (from the console or iohr token create).
func WithToken(token string) Option { return func(c *config) { c.token = token } }

// WithKey authenticates with an API key, exchanged for short-lived tokens; it needs
// WithScopes.
func WithKey(id, secret string) Option {
	return func(c *config) { c.keyID, c.keySecret = id, secret }
}

// WithScopes sets the scopes to ask for with a key, a subset of the key's; there is no
// default.
func WithScopes(scopes ...string) Option {
	return func(c *config) { c.scopes = append([]string(nil), scopes...) }
}

// WithTokenProvider authenticates with your own token source.
func WithTokenProvider(p TokenProvider) Option { return func(c *config) { c.provider = p } }

// WithBaseURL sets the API's origin (default https://api.inorbit.hr; plain HTTP only to
// this machine).
func WithBaseURL(u string) Option { return func(c *config) { c.baseURL = u } }

// WithTokenURL sets the token endpoint a key is exchanged at.
func WithTokenURL(u string) Option { return func(c *config) { c.tokenURL = u } }

// WithTimeout sets how long each attempt may take (default 30 s).
func WithTimeout(d time.Duration) Option { return func(c *config) { c.timeout = d } }

// WithMaxRetries sets the retries after the first attempt (default 2; 0 disables).
func WithMaxRetries(n int) Option { return func(c *config) { c.maxRetries = n } }

// WithUserAgentSuffix appends s to the user agent.
func WithUserAgentSuffix(s string) Option { return func(c *config) { c.uaSuffix = s } }

// WithHook adds an observer of every attempt.
func WithHook(h Hook) Option { return func(c *config) { c.hooks = append(c.hooks, h) } }

// WithHTTPClient sets the HTTP client: your transport, proxy and TLS settings. The SDK
// does not follow redirects whatever the client's policy.
func WithHTTPClient(h *http.Client) Option { return func(c *config) { c.http = h } }

// Client is a client for one credential. It is safe for concurrent use and holds no
// per-call state.
type Client struct {
	base       *url.URL
	provider   TokenProvider
	timeout    time.Duration
	maxRetries int
	userAgent  string
	hooks      []Hook
	http       *http.Client
}

// NewClient returns a client configured by opts. It needs a credential: WithToken,
// WithKey with WithScopes, or WithTokenProvider.
func NewClient(opts ...Option) (*Client, error) {
	cfg := config{baseURL: DefaultBaseURL, tokenURL: DefaultTokenURL, timeout: 30 * time.Second, maxRetries: 2}
	for _, o := range opts {
		o(&cfg)
	}
	base, err := checkURL("the base URL", cfg.baseURL, true)
	if err != nil {
		return nil, err
	}
	tokenURL, err := checkURL("the token URL", cfg.tokenURL, false)
	if err != nil {
		return nil, err
	}
	hc := &http.Client{CheckRedirect: noRedirects}
	if cfg.http != nil {
		copied := *cfg.http
		copied.CheckRedirect = noRedirects
		hc = &copied
	}
	c := &Client{
		base: base, timeout: cfg.timeout, maxRetries: max(cfg.maxRetries, 0),
		userAgent: userAgent(cfg.uaSuffix), hooks: cfg.hooks, http: hc,
	}
	switch {
	case cfg.provider != nil:
		c.provider = cfg.provider
	case cfg.token != "":
		c.provider = NewStaticToken(cfg.token)
	case cfg.keyID != "" && cfg.keySecret != "":
		if len(cfg.scopes) == 0 {
			return nil, &ConfigError{Message: `no scopes: set INORBIT_SCOPES (space-separated, such as "identity:read account:read")`}
		}
		c.provider = NewClientCredentials(cfg.keyID, cfg.keySecret, cfg.scopes, tokenURL.String(), hc)
	default:
		return nil, &ConfigError{Message: "no credentials: set INORBIT_TOKEN, or INORBIT_KEY_ID and INORBIT_KEY_SECRET"}
	}
	return c, nil
}

// FromEnv returns a client from the environment. A named profile (its environment
// name, such as ACME_CI) reads INORBIT_ACME_CI_TOKEN, or INORBIT_ACME_CI_KEY_ID,
// _KEY_SECRET and _SCOPES, and nothing else; an empty profile reads the bare INORBIT_*
// names. INORBIT_BASE_URL and INORBIT_TOKEN_URL apply to every profile. opts apply
// after the environment.
func FromEnv(profile string, opts ...Option) (*Client, error) {
	prefix := "INORBIT_"
	if profile != "" {
		prefix = "INORBIT_" + profile + "_"
	}
	v := func(name string) string { return os.Getenv(prefix + name) }
	shared := func(name string) string {
		if s := v(name); s != "" {
			return s
		}
		return os.Getenv("INORBIT_" + name)
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

// BaseURL is the API's origin this client calls.
func (c *Client) BaseURL() string { return c.base.String() }

// Call calls op on c and reads its JSON answer into a T.
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

// outcome is what one attempt came to.
type outcome int

const (
	done outcome = iota
	unauthorized
	retry
)

// Send calls op and returns the answer as it came, a 2xx one only; anything else is an
// error.
func (c *Client) Send(ctx context.Context, op Operation) (*RawResponse, error) {
	u, err := c.url(op)
	if err != nil {
		return nil, err
	}
	var body []byte
	if op.Body != nil {
		body, err = json.Marshal(op.Body)
		if err != nil {
			return nil, &ConfigError{Message: "the body could not be written as JSON: " + err.Error()}
		}
	}
	id := requestID()
	retries, refreshed := 0, false
	name := op.Name
	if name == "" {
		name = op.Path
	}
	for number := 1; ; number++ {
		a := Attempt{Operation: name, Method: op.Method, Path: op.Path, Number: number, RequestID: id}
		raw, wait, kind, err := c.attempt(ctx, op.Method, u, body, a)
		if kind == done && err == nil {
			return raw, nil
		}
		if kind == unauthorized && !refreshed {
			if inv, ok := c.provider.(Invalidator); ok {
				inv.Invalidate()
			}
			refreshed = true
			continue
		}
		if kind == retry && op.retrySafe() && retries < c.maxRetries {
			if wait < 0 {
				wait = backoff(retries)
			}
			if serr := sleep(ctx, wait); serr != nil {
				c.failed(a, serr)
				return nil, serr
			}
			retries++
			continue
		}
		if err == nil {
			err = newAPIError(raw)
		}
		c.failed(a, err)
		return nil, err
	}
}

func (c *Client) failed(a Attempt, err error) {
	for _, h := range c.hooks {
		h.OnError(a, err)
	}
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

// attempt sends one attempt. A wait below zero means no Retry-After was given.
func (c *Client) attempt(ctx context.Context, method string, u *url.URL, body []byte, a Attempt) (*RawResponse, time.Duration, outcome, error) {
	tok, err := c.provider.Token(ctx)
	if err != nil {
		var aerr *AuthError
		if !errors.As(err, &aerr) {
			err = &AuthError{Message: "the token provider failed: " + err.Error(), Err: err}
		}
		return nil, 0, done, err
	}
	actx, cancel := context.WithTimeout(ctx, c.timeout)
	defer cancel()
	var reader io.Reader
	if body != nil {
		reader = bytes.NewReader(body)
	}
	req, err := http.NewRequestWithContext(actx, method, u.String(), reader)
	if err != nil {
		return nil, 0, done, &ConfigError{Message: "the request could not be built: " + err.Error()}
	}
	req.Header.Set("Authorization", "Bearer "+tok.Access)
	req.Header.Set("Accept", "application/json")
	req.Header.Set("User-Agent", c.userAgent)
	req.Header.Set("X-Request-Id", a.RequestID)
	if body != nil {
		req.Header.Set("Content-Type", "application/json")
	}
	for _, h := range c.hooks {
		h.OnRequest(a)
	}
	resp, err := c.http.Do(req)
	if err != nil {
		if ctx.Err() != nil {
			return nil, 0, done, ctx.Err()
		}
		if errors.Is(err, context.DeadlineExceeded) || isTimeout(err) {
			return nil, -1, retry, &TimeoutError{Host: c.base.Host, Seconds: int(c.timeout.Round(time.Second) / time.Second)}
		}
		return nil, -1, retry, &ConnectionError{Host: c.base.Host, Err: errors.New(describe(err))}
	}
	defer func() { _ = resp.Body.Close() }()
	if resp.ContentLength > maxBody {
		return nil, 0, done, &TooLargeError{}
	}
	data, err := io.ReadAll(io.LimitReader(resp.Body, maxBody+1))
	if err != nil {
		if ctx.Err() != nil {
			return nil, 0, done, ctx.Err()
		}
		return nil, -1, retry, &ConnectionError{Host: c.base.Host, Err: errors.New(describe(err))}
	}
	if len(data) > maxBody {
		return nil, 0, done, &TooLargeError{}
	}
	raw := &RawResponse{
		Status: resp.StatusCode, Header: resp.Header, Body: data, RequestID: a.RequestID,
		ServerRequestID: resp.Header.Get("X-Request-Id"), Attempts: a.Number,
	}
	for _, h := range c.hooks {
		h.OnResponse(a, raw)
	}
	switch {
	case raw.Status == http.StatusUnauthorized:
		return raw, 0, unauthorized, nil
	case retryableStatus(raw.Status):
		wait, ok := retryAfter(raw.Header)
		if !ok {
			wait = -1
		}
		return raw, wait, retry, nil
	case raw.Status >= 200 && raw.Status < 300:
		return raw, 0, done, nil
	}
	return raw, 0, done, newAPIError(raw)
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

func userAgent(suffix string) string {
	ua := fmt.Sprintf("inorbithr-sdk-go/%s go/%s %s/%s", SDKVersion,
		strings.TrimPrefix(runtime.Version(), "go"), runtime.GOOS, runtime.GOARCH)
	if suffix != "" {
		ua += " " + suffix
	}
	return ua
}
