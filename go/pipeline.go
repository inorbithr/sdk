package inorbit

import (
	"context"
	"fmt"
	"net/http"
	"slices"
	"strings"
	"time"
)

// Stage is where a middleware runs: once per call, or on every attempt.
type Stage string

const (
	// StagePerCall runs once per call, outside the retry loop.
	StagePerCall Stage = "per_call"

	// StagePerRetry runs on every attempt, inside the retry loop.
	StagePerRetry Stage = "per_retry"
)

// Middleware is one named step of the pipeline (docs/config.md section 7). Wrap returns
// a RoundTripper that handles a request and calls next for the rest of the pipeline: it
// may change the request (its headers are a fresh copy per attempt), answer without
// calling next, call next once, or more (each call of next from the per-call stage is a
// fresh attempt), and inspect what comes back. CallInfoFrom(req.Context()) says what the
// call is. Existing RoundTripper wrappers, such as otelhttp.NewTransport, fit as they
// are.
//
// A middleware must not log secrets or bodies; one at the per-retry stage sees the
// finished request, its Authorization header included. When CallInfo.Stream is true the
// answer's body is the stream: a middleware must not read it.
type Middleware struct {
	// Name is unique in the pipeline.
	Name string

	// Wrap builds the step around next, once per client.
	Wrap func(next http.RoundTripper) http.RoundTripper
}

// RoundTripperFunc is a function that is an http.RoundTripper, for writing a
// middleware's Wrap.
type RoundTripperFunc func(*http.Request) (*http.Response, error)

// RoundTrip calls f.
func (f RoundTripperFunc) RoundTrip(r *http.Request) (*http.Response, error) { return f(r) }

// CallInfo is what a middleware may read about the call (docs/config.md section 7.13).
type CallInfo struct {
	// Operation is the operation (radar.list_digests), or the path for a raw call.
	Operation string

	// Template is the path template (/v1/radar/digests/{digest_id}), when the surface
	// knows it.
	Template string

	// Idempotent reports whether retry may repeat the call.
	Idempotent bool

	// IdempotencyKey is the Idempotency-Key the call is sent with, once the
	// idempotency_key middleware set it.
	IdempotencyKey string

	// RequestID is the call's x-request-id, once the request_id middleware set it.
	RequestID string

	// Attempt is the attempt, from 1; 0 in the per-call stage.
	Attempt int

	// RetryReason is why the attempt before this one was retried (503, connection,
	// timeout); empty on the first attempt.
	RetryReason string

	// Deadline is when the call must end, once the deadline middleware set it; the zero
	// time before.
	Deadline time.Time

	// Stream reports that the answer is a stream, whose body a middleware must not read.
	Stream bool

	// Profile is the configuration profile the client was loaded for.
	Profile string

	// Stage is the stage the middleware runs in.
	Stage Stage

	// Tracing and Metrics report whether the client's tracing and metrics settings are on.
	Tracing, Metrics bool
}

// callState is what one call carries between middlewares, out of the user's sight. One
// call runs its steps one after another, so the fields need no lock.
type callState struct {
	info          CallInfo
	idempotencyOp bool   // the operation takes Idempotency-Key
	callerKey     string // the caller's Idempotency-Key
	timeout       time.Duration
	traceparent   string
	refreshed     bool
	attempts      int
	rateLimit     *RateLimit
	traceID       string // the attempt span's ids, for log records
	spanID        string
	streamOK      func(int) bool // the statuses whose body is a stream, left unread
}

type callKey struct{}

type attemptInfo struct {
	attempt int
	reason  string
	stage   Stage
}

type attemptCtxKey struct{}

func withCall(ctx context.Context, st *callState) context.Context {
	return context.WithValue(ctx, callKey{}, st)
}

func callOf(ctx context.Context) *callState {
	st, _ := ctx.Value(callKey{}).(*callState)
	return st
}

// CallInfoFrom returns what a request's call is, from the request's context; false
// outside the pipeline.
func CallInfoFrom(ctx context.Context) (CallInfo, bool) {
	st := callOf(ctx)
	if st == nil {
		return CallInfo{}, false
	}
	info := st.info
	info.Stage = StagePerCall
	if a, ok := ctx.Value(attemptCtxKey{}).(attemptInfo); ok {
		info.Attempt, info.RetryReason, info.Stage = a.attempt, a.reason, a.stage
	}
	return info, true
}

func withStage(r *http.Request, stage Stage) *http.Request {
	a, _ := r.Context().Value(attemptCtxKey{}).(attemptInfo)
	if a.stage == stage {
		return r
	}
	a.stage = stage
	return r.WithContext(context.WithValue(r.Context(), attemptCtxKey{}, a))
}

// CallOptions are options for one call, passed with WithCallOptions.
type CallOptions struct {
	// IdempotencyKey is the Idempotency-Key to send, on an operation that takes one; the
	// SDK generates one otherwise. On any other operation it is a ConfigError.
	IdempotencyKey string

	// Traceparent is a W3C traceparent to continue: the call span's parent, or sent as it
	// is when tracing is off.
	Traceparent string

	// Timeout replaces the client's attempt timeout for this call and shortens its total
	// timeout; it never extends the total timeout. Zero keeps the client's.
	Timeout time.Duration
}

type callOptionsKey struct{}

// WithCallOptions returns a context whose calls use o. Cancellation and deadlines come
// from ctx as for any call.
func WithCallOptions(ctx context.Context, o CallOptions) context.Context {
	return context.WithValue(ctx, callOptionsKey{}, o)
}

func callOptionsFrom(ctx context.Context) CallOptions {
	o, _ := ctx.Value(callOptionsKey{}).(CallOptions)
	return o
}

// kept are the built-ins that can be replaced but not removed.
var kept = []string{"retry", "auth", "timeout"}

// Pipeline is a client's middlewares, edited by name at construction (WithPipeline): the
// built-ins of docs/config.md section 7.2, outermost first, and what you add. Settings
// switch built-ins off without removing them (rate_limit off, tracing false, log off).
// The first mistake (a name that exists or does not, removing retry, auth or timeout)
// fails Load or NewClient with a ConfigError.
type Pipeline struct {
	slots []Middleware
	err   error
}

func (p *Pipeline) fail(msg string) {
	if p.err == nil {
		p.err = &ConfigError{Message: msg}
	}
}

func (p *Pipeline) index(name string) int {
	for i, m := range p.slots {
		if m.Name == name {
			return i
		}
	}
	p.fail(fmt.Sprintf("the pipeline has no middleware named %q; it has %s", name, strings.Join(p.Names(), ", ")))
	return -1
}

func (p *Pipeline) insert(at int, m Middleware) *Pipeline {
	if at < 0 {
		return p
	}
	if m.Name == "" || m.Wrap == nil {
		p.fail("a middleware needs a Name and a Wrap function")
		return p
	}
	if slices.ContainsFunc(p.slots, func(s Middleware) bool { return s.Name == m.Name }) {
		p.fail(fmt.Sprintf("the pipeline already has a middleware named %q", m.Name))
		return p
	}
	p.slots = slices.Insert(p.slots, at, m)
	return p
}

// AddPerCall adds m once per call, just before retry, after earlier additions.
func (p *Pipeline) AddPerCall(m Middleware) *Pipeline { return p.insert(p.index("retry"), m) }

// AddPerRetry adds m on every attempt, just before timeout, after earlier additions.
func (p *Pipeline) AddPerRetry(m Middleware) *Pipeline { return p.insert(p.index("timeout"), m) }

// InsertBefore adds m just before the middleware named name.
func (p *Pipeline) InsertBefore(name string, m Middleware) *Pipeline {
	return p.insert(p.index(name), m)
}

// InsertAfter adds m just after the middleware named name.
func (p *Pipeline) InsertAfter(name string, m Middleware) *Pipeline {
	i := p.index(name)
	if i < 0 {
		return p
	}
	return p.insert(i+1, m)
}

// Replace swaps the middleware named name for m, which takes its name and place.
func (p *Pipeline) Replace(name string, m Middleware) *Pipeline {
	i := p.index(name)
	if i < 0 {
		return p
	}
	if m.Wrap == nil {
		p.fail("a middleware needs a Wrap function")
		return p
	}
	m.Name = name
	p.slots[i] = m
	return p
}

// Remove drops the middleware named name; retry, auth and timeout can be replaced only.
func (p *Pipeline) Remove(name string) *Pipeline {
	if slices.Contains(kept, name) {
		why := map[string]string{
			"retry":   "use WithMaxRetries(0) to retry nothing",
			"auth":    "without it no credential is sent",
			"timeout": "without it an attempt could wait forever",
		}[name]
		p.fail(name + " cannot be removed, only replaced: " + why)
		return p
	}
	if i := p.index(name); i >= 0 {
		p.slots = slices.Delete(p.slots, i, i+1)
	}
	return p
}

// Names lists the middlewares by name, outermost first.
func (p *Pipeline) Names() []string {
	out := make([]string, len(p.slots))
	for i, m := range p.slots {
		out[i] = m.Name
	}
	return out
}

func (p *Pipeline) has(name string) bool {
	return slices.ContainsFunc(p.slots, func(m Middleware) bool { return m.Name == name })
}

// build composes the pipeline over transport; each step sees its stage in CallInfo.
func (p *Pipeline) build(transport http.RoundTripper) http.RoundTripper {
	retryAt := slices.IndexFunc(p.slots, func(m Middleware) bool { return m.Name == "retry" })
	next := transport
	for i := len(p.slots) - 1; i >= 0; i-- {
		stage := StagePerCall
		if retryAt >= 0 && i > retryAt {
			stage = StagePerRetry
		}
		inner := p.slots[i].Wrap(next)
		next = RoundTripperFunc(func(r *http.Request) (*http.Response, error) {
			return inner.RoundTrip(withStage(r, stage))
		})
	}
	return next
}
