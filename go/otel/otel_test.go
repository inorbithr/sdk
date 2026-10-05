package inorbitotel_test

import (
	"context"
	"io"
	"net/http"
	"strings"
	"sync"
	"testing"

	inorbit "github.com/inorbithr/sdk/go"
	inorbitotel "github.com/inorbithr/sdk/go/otel"
	"go.opentelemetry.io/otel/attribute"
	sdkmetric "go.opentelemetry.io/otel/sdk/metric"
	"go.opentelemetry.io/otel/sdk/metric/metricdata"
	sdktrace "go.opentelemetry.io/otel/sdk/trace"
	"go.opentelemetry.io/otel/sdk/trace/tracetest"
	"go.opentelemetry.io/otel/trace"
)

// api answers a 503 first, then 200, and keeps the traceparent each attempt carried.
type api struct {
	mu          sync.Mutex
	n           int
	traceparent []string
}

func (a *api) RoundTrip(r *http.Request) (*http.Response, error) {
	a.mu.Lock()
	a.n++
	n := a.n
	a.traceparent = append(a.traceparent, r.Header.Get("Traceparent"))
	a.mu.Unlock()
	status, body := 200, `{"subject":"ak_1"}`
	if n == 1 {
		status, body = 503, `{"code":"unavailable","error":"try again"}`
	}
	return &http.Response{
		StatusCode: status, Header: http.Header{"Content-Type": {"application/json"}},
		Body: io.NopCloser(strings.NewReader(body)), Request: r,
	}, nil
}

func attr(kvs []attribute.KeyValue, key string) (attribute.Value, bool) {
	for _, kv := range kvs {
		if string(kv.Key) == key {
			return kv.Value, true
		}
	}
	return attribute.Value{}, false
}

func TestSpansAndMetricsFollowTheConventions(t *testing.T) {
	spans := tracetest.NewSpanRecorder()
	tp := sdktrace.NewTracerProvider(sdktrace.WithSpanProcessor(spans))
	reader := sdkmetric.NewManualReader()
	mp := sdkmetric.NewMeterProvider(sdkmetric.WithReader(reader))
	transport := &api{}
	c, err := inorbit.NewClient(
		inorbit.WithToken("t"), inorbit.WithBaseURL("https://api.test"), inorbit.WithTransport(transport),
		inorbit.WithRetryBaseDelay(1), inorbit.WithRetryMaxDelay(1),
		inorbitotel.Pipeline(inorbitotel.WithTracerProvider(tp), inorbitotel.WithMeterProvider(mp)),
	)
	if err != nil {
		t.Fatal(err)
	}
	op := inorbit.Operation{Name: "me", Method: "GET", Path: "/v1/me", Template: "/v1/me"}
	if _, err := c.Send(context.Background(), op); err != nil {
		t.Fatal(err)
	}
	ended := spans.Ended()
	if len(ended) != 3 {
		t.Fatalf("spans: %d, want 3 (one call, two attempts)", len(ended))
	}
	var call sdktrace.ReadOnlySpan
	var attempts []sdktrace.ReadOnlySpan
	for _, s := range ended {
		if s.SpanKind() == trace.SpanKindInternal {
			call = s
		} else {
			attempts = append(attempts, s)
		}
	}
	if call == nil || call.Name() != "me" {
		t.Fatalf("no call span named me: %v", ended)
	}
	for i, s := range attempts {
		if s.Name() != "GET /v1/me" || s.SpanKind() != trace.SpanKindClient {
			t.Fatalf("attempt %d: %s %v", i, s.Name(), s.SpanKind())
		}
		if s.Parent().SpanID() != call.SpanContext().SpanID() {
			t.Fatalf("attempt %d is not a child of the call span", i)
		}
		if want := "00-" + s.SpanContext().TraceID().String() + "-" + s.SpanContext().SpanID().String() + "-01"; transport.traceparent[i] != want {
			t.Fatalf("attempt %d sent traceparent %q, want %q", i, transport.traceparent[i], want)
		}
	}
	if v, _ := attr(attempts[0].Attributes(), "error.type"); v.AsString() != "503" {
		t.Fatalf("first attempt error.type %q", v.AsString())
	}
	if v, ok := attr(attempts[1].Attributes(), "http.request.resend_count"); !ok || v.AsInt64() != 1 {
		t.Fatalf("second attempt resend_count %v", v)
	}
	if v, _ := attr(attempts[1].Attributes(), "url.template"); v.AsString() != "/v1/me" {
		t.Fatalf("url.template %q", v.AsString())
	}

	var rm metricdata.ResourceMetrics
	if err := reader.Collect(context.Background(), &rm); err != nil {
		t.Fatal(err)
	}
	seen := map[string]bool{}
	for _, sm := range rm.ScopeMetrics {
		if sm.Scope.Name != inorbitotel.ScopeName {
			continue
		}
		for _, m := range sm.Metrics {
			seen[m.Name] = true
		}
	}
	for _, name := range []string{inorbit.MetricRequestDuration, inorbit.MetricCallDuration, inorbit.MetricRetries} {
		if !seen[name] {
			t.Fatalf("metric %s not recorded: %v", name, seen)
		}
	}
}

func TestTracingFalseSendsTheCallersTraceparent(t *testing.T) {
	spans := tracetest.NewSpanRecorder()
	tp := sdktrace.NewTracerProvider(sdktrace.WithSpanProcessor(spans))
	transport := &api{n: 1}
	c, err := inorbit.NewClient(
		inorbit.WithToken("t"), inorbit.WithBaseURL("https://api.test"), inorbit.WithTransport(transport),
		inorbit.WithTracing(false), inorbitotel.Pipeline(inorbitotel.WithTracerProvider(tp)),
	)
	if err != nil {
		t.Fatal(err)
	}
	const parent = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0cb902b7-01"
	ctx := inorbit.WithCallOptions(context.Background(), inorbit.CallOptions{Traceparent: parent})
	if _, err := c.Send(ctx, inorbit.Operation{Name: "me", Method: "GET", Path: "/v1/me"}); err != nil {
		t.Fatal(err)
	}
	if len(spans.Ended()) != 0 || transport.traceparent[0] != parent {
		t.Fatalf("spans %d, traceparent %q", len(spans.Ended()), transport.traceparent[0])
	}
}

func TestACallersTraceparentParentsTheCallSpan(t *testing.T) {
	spans := tracetest.NewSpanRecorder()
	tp := sdktrace.NewTracerProvider(sdktrace.WithSpanProcessor(spans))
	c, err := inorbit.NewClient(
		inorbit.WithToken("t"), inorbit.WithBaseURL("https://api.test"), inorbit.WithTransport(&api{n: 1}),
		inorbitotel.Pipeline(inorbitotel.WithTracerProvider(tp)),
	)
	if err != nil {
		t.Fatal(err)
	}
	ctx := inorbit.WithCallOptions(context.Background(), inorbit.CallOptions{
		Traceparent: "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0cb902b7-01",
	})
	if _, err := c.Send(ctx, inorbit.Operation{Name: "me", Method: "GET", Path: "/v1/me"}); err != nil {
		t.Fatal(err)
	}
	for _, s := range spans.Ended() {
		if s.SpanContext().TraceID().String() != "4bf92f3577b34da6a3ce929d0e0e4736" {
			t.Fatalf("span %s left the caller's trace", s.Name())
		}
	}
}
