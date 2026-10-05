package inorbit

import "time"

// Attempt is one attempt of a call, as hooks see it: never a header or a body.
type Attempt struct {
	// Operation is the operation's name (radar.list_digests), or the path for a raw call.
	Operation string

	// Method is the HTTP method.
	Method string

	// Path is the path, parameters bound.
	Path string

	// Number is 1 for the first attempt.
	Number int

	// RequestID is the x-request-id sent.
	RequestID string

	// IdempotencyKey is the Idempotency-Key sent, on an operation that takes one.
	IdempotencyKey string

	// Stage is the pipeline stage the hooks middleware runs in: StagePerRetry, unless the
	// pipeline was rearranged.
	Stage Stage
}

// Hook observes calls, for logging, metrics or tracing. Embed NopHook to implement
// only the methods you need.
type Hook interface {
	// OnRequest runs before an attempt is sent.
	OnRequest(a Attempt)

	// OnResponse runs after an answer arrived, whatever its status.
	OnResponse(a Attempt, r *RawResponse)

	// OnError runs when the call fails for good.
	OnError(a Attempt, err error)
}

// RetryHook is a Hook that also hears of every retry, before its wait (docs/config.md
// section 7.7). It is a separate interface, found by type assertion as Invalidator is,
// so adding it broke no Hook.
type RetryHook interface {
	// OnRetry runs before a retry's wait: the attempt that failed, why (429, 503,
	// connection, timeout) and how long the wait is.
	OnRetry(a Attempt, reason string, delay time.Duration)
}

// NopHook does nothing; embed it in a Hook that needs only some of the methods.
type NopHook struct{}

// OnRequest does nothing.
func (NopHook) OnRequest(Attempt) {}

// OnResponse does nothing.
func (NopHook) OnResponse(Attempt, *RawResponse) {}

// OnError does nothing.
func (NopHook) OnError(Attempt, error) {}
