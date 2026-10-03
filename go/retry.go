package inorbit

import (
	"context"
	crand "crypto/rand"
	"encoding/hex"
	mrand "math/rand/v2"
	"net/http"
	"strconv"
	"strings"
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

// retryAfter is the wait Retry-After asks for, in whole seconds only, capped at a
// minute; zero and false when there is none.
func retryAfter(h http.Header) (time.Duration, bool) {
	v := strings.TrimSpace(h.Get("Retry-After"))
	if v == "" {
		return 0, false
	}
	secs, err := strconv.ParseUint(v, 10, 32)
	if err != nil {
		return 0, false
	}
	return min(time.Duration(secs)*time.Second, retryAfterCap), true
}

// backoff is full jitter: a random wait up to 0.5 s doubled per retry, at most 8 s.
func backoff(retry int) time.Duration {
	ceiling := min(backoffBase<<min(retry, 5), backoffCap)
	// Jitter only spreads retries out; it is not a secret, so math/rand is right here.
	return time.Duration(mrand.Int64N(int64(ceiling) + 1)) //nolint:gosec // jitter, not a secret
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
