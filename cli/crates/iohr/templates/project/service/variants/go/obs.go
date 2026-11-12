package main

import (
	"crypto/rand"
	"encoding/hex"
	"fmt"
	"io"
	"log/slog"
	"net/http"
	"regexp"
	"sort"
	"strings"
	"sync"
	"time"
)

// Observability on the standard library alone: JSON logs (log/slog), spans that follow the W3C
// traceparent header, and request counters in the Prometheus text format. Nothing here records
// a header, a query string or an environment value: a log line and a span name the route that
// matched, the status, the duration and the trace id.

var traceparentRe = regexp.MustCompile(`^00-([0-9a-f]{32})-([0-9a-f]{16})-([0-9a-f]{2})$`)

func randomHex(n int) string {
	b := make([]byte, n)
	if _, err := rand.Read(b); err != nil {
		panic(err) // the system's random source is gone: there is nothing safe to continue with
	}
	return hex.EncodeToString(b)
}

// newLogger writes one JSON object per line to w.
func newLogger(w io.Writer) *slog.Logger {
	return slog.New(slog.NewJSONHandler(w, nil))
}

// span is one request's trace: the id it joined (or started) and its own id.
type span struct {
	traceID, spanID, parentID string
	start                     time.Time
}

// startSpan joins the trace of a valid traceparent header, or starts a new one.
func startSpan(r *http.Request) span {
	s := span{spanID: randomHex(8), start: time.Now()}
	if m := traceparentRe.FindStringSubmatch(r.Header.Get("traceparent")); m != nil &&
		m[1] != strings.Repeat("0", 32) && m[2] != strings.Repeat("0", 16) {
		s.traceID, s.parentID = m[1], m[2]
	} else {
		s.traceID = randomHex(16)
	}
	return s
}

// header is the traceparent to send back (and on to callers): this span is the parent.
func (s span) header() string {
	return fmt.Sprintf("00-%s-%s-01", s.traceID, s.spanID)
}

// metrics counts requests by route and status. Routes are the matched patterns, never raw
// paths, so the label set is bounded by the code and holds no user input.
type metrics struct {
	mu     sync.Mutex
	counts map[[2]string]uint64
}

func newMetrics() *metrics { return &metrics{counts: map[[2]string]uint64{}} }

func (m *metrics) inc(route string, status int) {
	m.mu.Lock()
	m.counts[[2]string{route, fmt.Sprint(status)}]++
	m.mu.Unlock()
}

func (m *metrics) render() string {
	m.mu.Lock()
	defer m.mu.Unlock()
	keys := make([][2]string, 0, len(m.counts))
	for k := range m.counts {
		keys = append(keys, k)
	}
	sort.Slice(keys, func(i, j int) bool {
		if keys[i][0] != keys[j][0] {
			return keys[i][0] < keys[j][0]
		}
		return keys[i][1] < keys[j][1]
	})
	var b strings.Builder
	b.WriteString("# HELP http_requests_total Requests served, by route and status.\n# TYPE http_requests_total counter\n")
	for _, k := range keys {
		fmt.Fprintf(&b, "http_requests_total{route=%q,status=%q} %d\n", k[0], k[1], m.counts[k])
	}
	return b.String()
}
