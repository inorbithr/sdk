package inorbit

import (
	"context"
	"errors"
	"io"
	"net/http"
	"strconv"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"testing/synctest"
	"time"
)

// The tests in this file run inside a synctest bubble: time.Sleep, timers and
// time.Since use a virtual clock that jumps whenever every goroutine in the bubble is
// blocked, so a wait of a minute takes no real time and is measured exactly. The
// transport answers in memory, because a goroutine waiting on a real socket is not
// blocked in the bubble's sense and would stop the clock.

// roundTrip answers a request in memory.
type roundTrip func(*http.Request) (*http.Response, error)

func (f roundTrip) RoundTrip(r *http.Request) (*http.Response, error) { return f(r) }

func answer(status int, body string, header ...string) *http.Response {
	h := http.Header{}
	for i := 0; i+1 < len(header); i += 2 {
		h.Set(header[i], header[i+1])
	}
	return &http.Response{
		StatusCode: status, Header: h, Body: io.NopCloser(strings.NewReader(body)),
	}
}

// fakeAPI is a token endpoint and an API in one transport. api answers the nth API
// request (from 1); the token endpoint answers expires_in seconds and takes exchangeTook.
type fakeAPI struct {
	expiresIn    int
	exchangeTook time.Duration
	api          func(n int, r *http.Request) *http.Response

	exchanges atomic.Int32
	calls     atomic.Int32
}

func (f *fakeAPI) client(t *testing.T, opts ...Option) *Client {
	t.Helper()
	transport := roundTrip(func(r *http.Request) (*http.Response, error) {
		if r.URL.Path == "/oauth2/token" {
			f.exchanges.Add(1)
			time.Sleep(f.exchangeTook)
			return answer(200, `{"access_token":"tok","expires_in":`+strconv.Itoa(f.expiresIn)+`}`), nil
		}
		n := int(f.calls.Add(1))
		if r.Header.Get("Authorization") != "Bearer tok" {
			return answer(401, ""), nil
		}
		if f.api == nil {
			return answer(200, `{}`), nil
		}
		return f.api(n, r), nil
	})
	all := append([]Option{
		WithBaseURL("https://api.test"), WithTokenURL("https://auth.test/oauth2/token"),
		WithKey("ak_test", "s3cr3t"), WithScopes("identity:read"),
		WithHTTPClient(&http.Client{Transport: transport}),
	}, opts...)
	c, err := NewClient(all...)
	if err != nil {
		t.Fatal(err)
	}
	return c
}

func get(ctx context.Context, c *Client) error {
	_, err := Call[map[string]any](ctx, c, Operation{Name: "me", Method: "GET", Path: "/v1/me"})
	return err
}

func TestRetryAfterIsWaitedExactly(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := &fakeAPI{expiresIn: 900, api: func(n int, _ *http.Request) *http.Response {
			if n == 1 {
				return answer(429, "", "Retry-After", "3")
			}
			return answer(200, `{}`)
		}}
		c := f.client(t)
		start := time.Now()
		if err := get(t.Context(), c); err != nil {
			t.Fatal(err)
		}
		if took := time.Since(start); took != 3*time.Second {
			t.Fatalf("waited %v, want exactly 3s", took)
		}
		if f.calls.Load() != 2 {
			t.Fatalf("attempts %d, want 2", f.calls.Load())
		}
	})
}

func TestRetryAfterIsCappedAtAMinute(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := &fakeAPI{expiresIn: 900, api: func(n int, _ *http.Request) *http.Response {
			if n == 1 {
				return answer(503, "", "Retry-After", "600")
			}
			return answer(200, `{}`)
		}}
		start := time.Now()
		if err := get(t.Context(), f.client(t)); err != nil {
			t.Fatal(err)
		}
		if took := time.Since(start); took != time.Minute {
			t.Fatalf("waited %v, want the 1m cap", took)
		}
	})
}

func TestBackoffStaysUnderItsCeiling(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := &fakeAPI{expiresIn: 900, api: func(n int, _ *http.Request) *http.Response {
			if n <= 2 {
				return answer(503, "")
			}
			return answer(200, `{}`)
		}}
		start := time.Now()
		if err := get(t.Context(), f.client(t)); err != nil {
			t.Fatal(err)
		}
		// Full jitter: at most 0.5 s before the first retry and 1 s before the second.
		if took := time.Since(start); took > 1500*time.Millisecond {
			t.Fatalf("waited %v, more than the 1.5s the two ceilings allow", took)
		}
		if f.calls.Load() != 3 {
			t.Fatalf("attempts %d, want 3", f.calls.Load())
		}
	})
}

func TestAPostIsNotRetried(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := &fakeAPI{expiresIn: 900, api: func(int, *http.Request) *http.Response {
			return answer(503, "")
		}}
		c := f.client(t)
		start := time.Now()
		_, err := c.Send(t.Context(), Operation{Name: "events.create_endpoint", Method: "POST", Path: "/v1/webhooks/endpoints", Body: map[string]string{}})
		var api *APIError
		if !errors.As(err, &api) || api.Status != 503 {
			t.Fatalf("got %v, want the 503", err)
		}
		if f.calls.Load() != 1 || time.Since(start) != 0 {
			t.Fatalf("attempts %d after %v, want one at once", f.calls.Load(), time.Since(start))
		}
	})
}

func TestAnAttemptTimesOut(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := &fakeAPI{expiresIn: 900}
		hang := roundTrip(func(r *http.Request) (*http.Response, error) {
			if r.URL.Path == "/oauth2/token" {
				return answer(200, `{"access_token":"tok","expires_in":900}`), nil
			}
			<-r.Context().Done()
			return nil, r.Context().Err()
		})
		c := f.client(t, WithHTTPClient(&http.Client{Transport: hang}), WithTimeout(30*time.Second), WithMaxRetries(0))
		start := time.Now()
		err := get(t.Context(), c)
		var timeout *TimeoutError
		if !errors.As(err, &timeout) {
			t.Fatalf("got %v, want a TimeoutError", err)
		}
		if took := time.Since(start); took != 30*time.Second {
			t.Fatalf("gave up after %v, want 30s", took)
		}
	})
}

func TestAKeyTokenIsReusedUntilAFifthOfItsLifeIsLeft(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := &fakeAPI{expiresIn: 100}
		c := f.client(t)
		for _, step := range []struct {
			wait      time.Duration
			exchanges int32
		}{{0, 1}, {79 * time.Second, 1}, {2 * time.Second, 2}} {
			time.Sleep(step.wait)
			if err := get(t.Context(), c); err != nil {
				t.Fatal(err)
			}
			if got := f.exchanges.Load(); got != step.exchanges {
				t.Fatalf("after %v: %d exchanges, want %d", step.wait, got, step.exchanges)
			}
		}
	})
}

func TestConcurrentCallersShareOneExchange(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		f := &fakeAPI{expiresIn: 900, exchangeTook: 5 * time.Second}
		c := f.client(t)
		start := time.Now()
		var wg sync.WaitGroup
		for range 10 {
			wg.Go(func() {
				if err := get(t.Context(), c); err != nil {
					t.Errorf("call: %v", err)
				}
			})
		}
		wg.Wait()
		if f.exchanges.Load() != 1 || f.calls.Load() != 10 {
			t.Fatalf("exchanges %d calls %d, want 1 and 10", f.exchanges.Load(), f.calls.Load())
		}
		if took := time.Since(start); took != 5*time.Second {
			t.Fatalf("took %v, want the one 5s exchange", took)
		}
	})
}
