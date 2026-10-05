// Package inorbitotel connects the InOrbit SDK's tracing and metrics to OpenTelemetry
// (docs/config.md section 7.10). It is a module of its own, so the SDK's core keeps one
// dependency; install it only where OpenTelemetry is already set up:
//
//	client, err := inorbit.Load(ctx, inorbitotel.Pipeline())
//
// Pipeline switches on the call_tracing and attempt_tracing middlewares: one INTERNAL
// span per call, named for the operation, and one CLIENT span per attempt, named
// "{method} {url.template}", following the stable HTTP client semantic conventions, with
// traceparent and tracestate sent from the attempt span. The metrics are
// http.client.request.duration, inorbit.client.call.duration, inorbit.client.retries and
// inorbit.client.token.exchanges. The tracer and meter come from the global providers
// unless WithTracerProvider or WithMeterProvider names others; the SDK sends nothing
// anywhere itself. The tracing and metrics settings (INORBIT_TRACING, INORBIT_METRICS)
// still switch either off.
package inorbitotel

import (
	"context"

	inorbit "github.com/inorbithr/sdk/go"
	"go.opentelemetry.io/otel"
	"go.opentelemetry.io/otel/attribute"
	"go.opentelemetry.io/otel/codes"
	"go.opentelemetry.io/otel/metric"
	"go.opentelemetry.io/otel/propagation"
	"go.opentelemetry.io/otel/trace"
)

// ScopeName is the instrumentation scope the SDK's spans and metrics carry.
const ScopeName = "inorbithr"

// requestBuckets are the semantic conventions' buckets for http.client.request.duration.
var requestBuckets = []float64{0.005, 0.01, 0.025, 0.05, 0.075, 0.1, 0.25, 0.5, 0.75, 1, 2.5, 5, 7.5, 10}

type options struct {
	tp trace.TracerProvider
	mp metric.MeterProvider
}

// Option configures Pipeline.
type Option func(*options)

// WithTracerProvider records spans with tp instead of the global tracer provider.
func WithTracerProvider(tp trace.TracerProvider) Option { return func(o *options) { o.tp = tp } }

// WithMeterProvider records metrics with mp instead of the global meter provider.
func WithMeterProvider(mp metric.MeterProvider) Option { return func(o *options) { o.mp = mp } }

// Pipeline is the client option that records the SDK's spans and metrics with
// OpenTelemetry.
func Pipeline(opts ...Option) inorbit.Option {
	return inorbit.WithTelemetry(Telemetry(opts...))
}

// Telemetry is the tracer and meter Pipeline installs, for a program that composes its
// own options.
func Telemetry(opts ...Option) inorbit.Telemetry {
	o := options{}
	for _, f := range opts {
		f(&o)
	}
	if o.tp == nil {
		o.tp = otel.GetTracerProvider()
	}
	if o.mp == nil {
		o.mp = otel.GetMeterProvider()
	}
	t := &tracer{t: o.tp.Tracer(ScopeName, trace.WithInstrumentationVersion(inorbit.SDKVersion))}
	return inorbit.Telemetry{Tracer: t, Meter: newMeter(o.mp)}
}

type tracer struct {
	t trace.Tracer
}

// carrier is a map of headers for the W3C propagator.
type carrier map[string]string

func (c carrier) Get(k string) string { return c[k] }
func (c carrier) Set(k, v string)     { c[k] = v }
func (c carrier) Keys() []string {
	out := make([]string, 0, len(c))
	for k := range c {
		out = append(out, k)
	}
	return out
}

var traceContext = propagation.TraceContext{}

func (t *tracer) Start(ctx context.Context, s inorbit.SpanStart) (context.Context, inorbit.Span) {
	if s.Parent != "" {
		ctx = traceContext.Extract(ctx, carrier{"traceparent": s.Parent})
	}
	kind := trace.SpanKindInternal
	if s.Kind == inorbit.SpanKindClient {
		kind = trace.SpanKindClient
	}
	ctx, sp := t.t.Start(ctx, s.Name, trace.WithSpanKind(kind), trace.WithAttributes(attributes(s.Attributes)...))
	return ctx, span{sp}
}

type span struct {
	s trace.Span
}

func (s span) SetAttributes(attrs ...inorbit.Attribute) { s.s.SetAttributes(attributes(attrs)...) }

func (s span) SetError(errorType string) {
	s.s.SetAttributes(attribute.String("error.type", errorType))
	s.s.SetStatus(codes.Error, "")
}

func (s span) End() { s.s.End() }

func (s span) Traceparent() (string, string) {
	if !s.s.SpanContext().IsValid() {
		return "", ""
	}
	c := carrier{}
	traceContext.Inject(trace.ContextWithSpan(context.Background(), s.s), c)
	return c["traceparent"], c["tracestate"]
}

func attributes(attrs []inorbit.Attribute) []attribute.KeyValue {
	out := make([]attribute.KeyValue, 0, len(attrs))
	for _, a := range attrs {
		switch v := a.Value.(type) {
		case string:
			out = append(out, attribute.String(a.Key, v))
		case int64:
			out = append(out, attribute.Int64(a.Key, v))
		case int:
			out = append(out, attribute.Int(a.Key, v))
		case bool:
			out = append(out, attribute.Bool(a.Key, v))
		case float64:
			out = append(out, attribute.Float64(a.Key, v))
		}
	}
	return out
}

type meter struct {
	request   metric.Float64Histogram
	call      metric.Float64Histogram
	retries   metric.Int64Counter
	exchanges metric.Int64Counter
}

func newMeter(mp metric.MeterProvider) *meter {
	m := mp.Meter(ScopeName, metric.WithInstrumentationVersion(inorbit.SDKVersion))
	// An instrument that cannot be made is a no-op one: metrics never fail a call.
	request, _ := m.Float64Histogram(inorbit.MetricRequestDuration, metric.WithUnit("s"),
		metric.WithDescription("Duration of HTTP client requests."), metric.WithExplicitBucketBoundaries(requestBuckets...))
	call, _ := m.Float64Histogram(inorbit.MetricCallDuration, metric.WithUnit("s"),
		metric.WithDescription("Duration of SDK calls, every attempt and wait included."))
	retries, _ := m.Int64Counter(inorbit.MetricRetries, metric.WithUnit("{retry}"),
		metric.WithDescription("Retries the SDK made."))
	exchanges, _ := m.Int64Counter(inorbit.MetricTokenExchanges, metric.WithUnit("{exchange}"),
		metric.WithDescription("Token exchanges and refreshes the SDK made."))
	return &meter{request: request, call: call, retries: retries, exchanges: exchanges}
}

func (m *meter) Record(ctx context.Context, name string, value float64, attrs ...inorbit.Attribute) {
	set := metric.WithAttributes(attributes(attrs)...)
	switch name {
	case inorbit.MetricRequestDuration:
		if m.request != nil {
			m.request.Record(ctx, value, set)
		}
	case inorbit.MetricCallDuration:
		if m.call != nil {
			m.call.Record(ctx, value, set)
		}
	case inorbit.MetricRetries:
		if m.retries != nil {
			m.retries.Add(ctx, int64(value), set)
		}
	case inorbit.MetricTokenExchanges:
		if m.exchanges != nil {
			m.exchanges.Add(ctx, int64(value), set)
		}
	}
}
