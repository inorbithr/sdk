package main

import (
	"bytes"
	"context"
	"encoding/json"
	"io"
	"net"
	"net/http"
	"os"
	"strings"
	"sync"
	"testing"
	"time"
)

// syncBuffer is a log sink the server writes to while the test reads it.
type syncBuffer struct {
	mu sync.Mutex
	b  bytes.Buffer
}

func (s *syncBuffer) Write(p []byte) (int, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.b.Write(p)
}

func (s *syncBuffer) String() string {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.b.String()
}

// boot starts the service on a free port and returns its base URL and its log.
func boot(t *testing.T) (string, *syncBuffer) {
	t.Helper()
	ln, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	logs := &syncBuffer{}
	svc := newService(newLogger(logs))
	ctx, cancel := context.WithCancel(context.Background())
	done := make(chan error, 1)
	go func() { done <- serve(ctx, ln, svc) }()
	t.Cleanup(func() {
		cancel()
		if err := <-done; err != nil {
			t.Errorf("shutdown: %v", err)
		}
	})
	base := "http://" + ln.Addr().String()
	for i := 0; i < 100 && !svc.ready.Load(); i++ {
		time.Sleep(10 * time.Millisecond)
	}
	return base, logs
}

func get(t *testing.T, url string, headers map[string]string) (int, string, http.Header) {
	t.Helper()
	req, _ := http.NewRequest(http.MethodGet, url, nil)
	for k, v := range headers {
		req.Header.Set(k, v)
	}
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		t.Fatal(err)
	}
	defer resp.Body.Close()
	body, _ := io.ReadAll(resp.Body)
	return resp.StatusCode, string(body), resp.Header
}

func records(t *testing.T, logs *syncBuffer) []map[string]any {
	t.Helper()
	var out []map[string]any
	for _, line := range strings.Split(strings.TrimSpace(logs.String()), "\n") {
		var m map[string]any
		if err := json.Unmarshal([]byte(line), &m); err != nil {
			t.Fatalf("a log line is not JSON: %q", line)
		}
		out = append(out, m)
	}
	return out
}

func TestProbesAnswer(t *testing.T) {
	base, _ := boot(t)
	for _, path := range []string{"/healthz", "/readyz"} {
		if code, _, _ := get(t, base+path, nil); code != 200 {
			t.Errorf("%s answered %d", path, code)
		}
	}
	if code, body, _ := get(t, base+"/v1/hello?name=Ada", nil); code != 200 || !strings.Contains(body, "Hello, Ada") {
		t.Errorf("hello answered %d %q", code, body)
	}
	if code, _, _ := get(t, base+"/nope", nil); code != 404 {
		t.Errorf("an unknown path answered %d", code)
	}
}

func TestOneSpanWithTheRouteAndTheTraceIDInTheLog(t *testing.T) {
	canary := "canary-value-must-not-be-logged"
	t.Setenv("SERVICE_CANARY", canary)
	base, logs := boot(t)
	trace := "4bf92f3577b34da6a3ce929d0e0e4736"
	_, _, h := get(t, base+"/v1/hello?name=secret-query-value", map[string]string{
		"traceparent":   "00-" + trace + "-00f067aa0ba902b7-01",
		"Authorization": "Bearer secret-header-value",
	})
	if !strings.HasPrefix(h.Get("traceparent"), "00-"+trace+"-") {
		t.Errorf("the response does not continue the trace: %q", h.Get("traceparent"))
	}
	var spans, requests int
	for _, r := range records(t, logs) {
		switch r["msg"] {
		case "span":
			spans++
			if r["route"] != "/v1/hello" || r["trace_id"] != trace || r["parent_span_id"] != "00f067aa0ba902b7" {
				t.Errorf("the span is wrong: %v", r)
			}
		case "request":
			requests++
			if r["trace_id"] != trace {
				t.Errorf("the request log has no trace id: %v", r)
			}
		}
	}
	if spans != 1 || requests != 1 {
		t.Errorf("want one span and one request line, got %d and %d", spans, requests)
	}
	all := logs.String()
	for _, secret := range []string{canary, "secret-query-value", "secret-header-value"} {
		if strings.Contains(all, secret) {
			t.Errorf("the log holds %q", secret)
		}
	}
	for _, kv := range os.Environ() {
		if _, v, _ := strings.Cut(kv, "="); len(v) >= 12 && strings.Contains(all, v) {
			t.Errorf("the log holds an environment value: %.12s...", v)
		}
	}
}

func TestAnUnknownPathIsNeverLoggedAndMetricsCountByRoute(t *testing.T) {
	base, logs := boot(t)
	get(t, base+"/private/user-1234", nil)
	get(t, base+"/healthz", nil)
	if strings.Contains(logs.String(), "user-1234") {
		t.Error("the log holds a raw path")
	}
	_, body, _ := get(t, base+"/metrics", nil)
	for _, want := range []string{`route="unmatched",status="404"`, `route="/healthz",status="200"`} {
		if !strings.Contains(body, want) {
			t.Errorf("metrics lack %s:\n%s", want, body)
		}
	}
}

func TestConfig(t *testing.T) {
	if _, err := loadConfig(func(k string) string { return map[string]string{"PORT": "99999"}[k] }); err == nil {
		t.Error("a port out of range was accepted")
	}
	c, err := loadConfig(func(string) string { return "" })
	if err != nil || c.port != 8080 || c.host != "127.0.0.1" {
		t.Errorf("defaults: %v %v", c, err)
	}
}
