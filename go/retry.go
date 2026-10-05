package inorbit

import (
	"context"
	crand "crypto/rand"
	"encoding/hex"
	mrand "math/rand/v2"
	"net/http"
	"strconv"
	"strings"
	"sync"
	"time"
)

const (
	retryAfterCap = 60 * time.Second
	backoffBase   = 500 * time.Millisecond
	backoffCap    = 8 * time.Second
)

// retryableStatus reports whether status is worth another attempt on an idempotent
// call.
func retryableStatus(status int) bool {
	return status == http.StatusTooManyRequests || status == http.StatusServiceUnavailable ||
		status == http.StatusGatewayTimeout
}

// retryAfter is the wait Retry-After asks for: delay-seconds or an HTTP date (RFC 9110
// section 10.2.3), a date in the past being no wait; zero and false when there is none.
func retryAfter(h http.Header, now time.Time) (time.Duration, bool) {
	v := strings.TrimSpace(h.Get("Retry-After"))
	if v == "" {
		return 0, false
	}
	if secs, err := strconv.ParseUint(v, 10, 32); err == nil {
		return time.Duration(secs) * time.Second, true
	}
	at, err := http.ParseTime(v)
	if err != nil {
		return 0, false
	}
	return max(at.Sub(now), 0), true
}

// backoff is full jitter: a random wait up to 0.5 s doubled per retry, at most 8 s.
func backoff(retry int) time.Duration { return backoffWith(retry, backoffBase, backoffCap) }

// backoffWith is full jitter: a random wait up to base doubled per retry, at most ceiling.
func backoffWith(retry int, base, ceiling time.Duration) time.Duration {
	limit := ceiling
	if retry < 30 && base<<retry > 0 {
		limit = min(base<<retry, ceiling)
	}
	// Jitter only spreads retries out; it is not a secret, so math/rand is right here.
	return time.Duration(mrand.Int64N(int64(limit) + 1)) //nolint:gosec // jitter, not a secret
}

// retryBudgetCapacity is the retry budget's capacity: AWS's standard retry mode.
const retryBudgetCapacity = 500

// retryBudget is a client's token bucket for retries (docs/config.md section 7.4).
type retryBudget struct {
	mu       sync.Mutex
	capacity int
	tokens   int
}

func newRetryBudget(capacity int) *retryBudget {
	return &retryBudget{capacity: capacity, tokens: capacity}
}

// take pays cost when the bucket can.
func (b *retryBudget) take(cost int) bool {
	if b == nil {
		return true
	}
	b.mu.Lock()
	defer b.mu.Unlock()
	if b.tokens < cost {
		return false
	}
	b.tokens -= cost
	return true
}

// give puts n back, up to the capacity.
func (b *retryBudget) give(n int) {
	if b == nil {
		return
	}
	b.mu.Lock()
	b.tokens = min(b.capacity, b.tokens+n)
	b.mu.Unlock()
}

// requestID is iohr-<16 hex>, the id each call is sent with.
func requestID() string {
	var b [8]byte
	_, _ = crand.Read(b[:])
	return "iohr-" + hex.EncodeToString(b[:])
}

// sleep waits d, or returns the context's error when it ends first.
func sleep(ctx context.Context, d time.Duration) error {
	t := time.NewTimer(d)
	defer t.Stop()
	select {
	case <-ctx.Done():
		return ctx.Err()
	case <-t.C:
		return nil
	}
}
