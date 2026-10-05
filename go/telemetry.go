package inorbit

import (
	"context"
	"net/url"
	"regexp"
	"strings"
)

// Tracing and metrics (docs/config.md section 7.10). The module has no OpenTelemetry
// dependency: the call_tracing and attempt_tracing built-ins record spans and metrics
// through the small Tracer and Meter interfaces, which the optional module
// github.com/inorbithr/sdk/go/otel implements over OpenTelemetry (inorbitotel.Pipeline).
// Without one they do nothing, and a traceparent passed with CallOptions is sent as it
// is.

// SpanKind is a span's kind, as OpenTelemetry names them.
type SpanKind int

const (
	// SpanKindInternal is the call's span.
	SpanKindInternal SpanKind = iota

	// SpanKindClient is one attempt's HTTP client span.
	SpanKindClient
)

// Attribute is one span or metric attribute; Value is a string, an int64 or a bool.
type Attribute struct {
	Key   string
	Value any
}

// SpanStart is what a span starts with.
type SpanStart struct {
	// Name is the span's name.
	Name string

	// Kind is its kind.
	Kind SpanKind

	// Attributes are set at the start.
	Attributes []Attribute

	// Parent is a W3C traceparent the span continues instead of the span in the context;
	// empty for the context's.
	Parent string
}

// Span is one span a Tracer started.
type Span interface {
	// SetAttributes adds attributes.
	SetAttributes(attrs ...Attribute)

	// SetError marks the span failed, with errorType as error.type.
	SetError(errorType string)

	// End ends the span.
	End()

	// Traceparent is the span's W3C traceparent and tracestate, to send; empty when the
	// span is not recording a valid context.
	Traceparent() (traceparent, tracestate string)
}

// Tracer starts spans. The context it returns carries the span, so a span started from
// it is its child.
type Tracer interface {
	// Start starts a span.
	Start(ctx context.Context, s SpanStart) (context.Context, Span)
}

// Meter records the SDK's metrics: http.client.request.duration and
// inorbit.client.call.duration (seconds, histograms), inorbit.client.retries and
// inorbit.client.token.exchanges (counters, value 1).
type Meter interface {
	// Record records value for the metric name.
	Record(ctx context.Context, name string, value float64, attrs ...Attribute)
}

// Telemetry is a tracer and a meter for WithTelemetry; either may be nil.
type Telemetry struct {
	Tracer Tracer
	Meter  Meter
}

// The metric names (docs/config.md section 7.10).
const (
	MetricRequestDuration = "http.client.request.duration"
	MetricCallDuration    = "inorbit.client.call.duration"
	MetricRetries         = "inorbit.client.retries"
	MetricTokenExchanges  = "inorbit.client.token.exchanges"
)

var traceparentSyntax = regexp.MustCompile(`^00-([0-9a-f]{32})-([0-9a-f]{16})-([0-9a-f]{2})$`)

// ParseTraceparent reads a W3C traceparent: its trace id and span id, and whether it is
// one.
func ParseTraceparent(v string) (traceID, spanID string, ok bool) {
	m := traceparentSyntax.FindStringSubmatch(strings.TrimSpace(v))
	if m == nil || strings.Trim(m[1], "0") == "" || strings.Trim(m[2], "0") == "" {
		return "", "", false
	}
	return m[1], m[2], true
}

// redactedURL is u with every query value replaced by REDACTED, for url.full.
func redactedURL(u *url.URL) string {
	if u.RawQuery == "" {
		return u.String()
	}
	c := *u
	var names []string
	seen := map[string]bool{}
	for _, pair := range strings.Split(u.RawQuery, "&") {
		name, _, _ := strings.Cut(pair, "=")
		if n, err := url.QueryUnescape(name); err == nil && !seen[n] {
			seen[n] = true
			names = append(names, url.QueryEscape(n)+"=REDACTED")
		}
	}
	c.RawQuery = strings.Join(names, "&")
	return c.String()
}

// errorType is error.type for err: the API's code, or the error's kind.
func errorType(err error) string {
	if a, ok := asAPIError(err); ok {
		return string(a.Code)
	}
	return errorKind(err)
}
