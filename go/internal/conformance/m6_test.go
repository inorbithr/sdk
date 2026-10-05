package conformance

import (
	"context"
	crand "crypto/rand"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"log/slog"
	"net/http"
	"os"
	"path/filepath"
	"regexp"
	"runtime"
	"strings"
	"sync"
	"testing"
	"time"

	inorbit "github.com/inorbithr/sdk/go"
)

// What the M6 cases add (docs/config.md section 9.1): clients built with Load from an
// injected environment and config file, probes in the pipeline, captured log records,
// spans in an in-memory tracer, and the rate limit, idempotency key and configuration a
// result carries.

func replayBin() string {
	root, _ := filepath.Abs("../../..")
	bin := filepath.Join(root, "conformance", "server", "bin", "replay")
	if runtime.GOOS == "windows" {
		bin += ".exe"
	}
	return bin
}

// built is a case's client and what the driver watches it with.
type built struct {
	client *inorbit.Client
	dir    string
	logs   *records
	probes map[string]*[]http.Header
	tracer *memTracer
}

func substituteText(text, replay, dir string) string {
	return strings.ReplaceAll(strings.ReplaceAll(text, "{replay}", replay), "{dir}", dir)
}

// build makes the client a case describes.
func build(t *testing.T, l loaded) (*built, error) {
	c := l.Case.Client
	dir := t.TempDir()
	b := &built{dir: dir, logs: &records{}, probes: map[string]*[]http.Header{}, tracer: &memTracer{}}
	for name, content := range c.Files {
		p := filepath.Join(dir, filepath.FromSlash(name))
		if err := os.MkdirAll(filepath.Dir(p), 0o750); err != nil {
			return nil, err
		}
		if err := os.WriteFile(p, []byte(substituteText(content, l.BaseURL, dir)), 0o600); err != nil {
			return nil, err
		}
	}
	var opts []inorbit.Option
	switch {
	case c.MaxRetries != nil:
		opts = append(opts, inorbit.WithMaxRetries(*c.MaxRetries))
	case !c.Load:
		opts = append(opts, inorbit.WithMaxRetries(2))
	}
	if c.TimeoutMS > 0 {
		opts = append(opts, inorbit.WithTimeout(time.Duration(c.TimeoutMS)*time.Millisecond))
	}
	if c.Streams != "" {
		opts = append(opts, inorbit.WithStreams(inorbit.Streams(c.Streams)))
	}
	if c.IdleMS > 0 {
		opts = append(opts, inorbit.WithStreamIdleTimeout(time.Duration(c.IdleMS)*time.Millisecond))
	}
	if c.Log != "" {
		opts = append(opts, inorbit.WithLog(inorbit.LogLevel(c.Log)), inorbit.WithLogger(slog.New(b.logs)))
	}
	if c.LogHeaders != nil {
		opts = append(opts, inorbit.WithLogHeaders(*c.LogHeaders))
	}
	if len(c.LogAllowHeaders) > 0 {
		opts = append(opts, inorbit.WithLogAllowHeaders(c.LogAllowHeaders...))
	}
	if c.RateLimit != "" {
		opts = append(opts, inorbit.WithRateLimit(inorbit.RateLimitMode(c.RateLimit)))
	}
	if c.TotalTimeoutMS > 0 {
		opts = append(opts, inorbit.WithTotalTimeout(time.Duration(c.TotalTimeoutMS)*time.Millisecond))
	}
	if c.RetryBudgetCapacity > 0 {
		opts = append(opts, inorbit.WithRetryBudgetCapacity(c.RetryBudgetCapacity))
	}
	if c.Tracing != nil {
		opts = append(opts, inorbit.WithTracing(*c.Tracing))
		if *c.Tracing {
			opts = append(opts, inorbit.WithTelemetry(inorbit.Telemetry{Tracer: b.tracer}))
		}
	}
	switch c.Transport {
	case "https", "proxy", "mtls":
		opts = append(opts, inorbit.WithCABundle(l.CAFile))
	}
	if c.Transport == "mtls" {
		opts = append(opts, inorbit.WithClientCert(l.ClientCertFile, l.ClientKeyFile))
	}
	if c.Transport == "proxy" {
		opts = append(opts, inorbit.WithProxy(l.ProxyURL))
	}
	if c.NoProxy != "" {
		opts = append(opts, inorbit.WithNoProxy(strings.Split(c.NoProxy, ",")...))
	}
	if len(c.CredentialSources) > 0 {
		opts = append(opts, inorbit.WithCredentialSources(c.CredentialSources...))
	}
	if c.CLI {
		opts = append(opts, inorbit.WithCLIPath(replayBin()))
	}
	if c.Pipeline != nil {
		opts = append(opts, inorbit.WithPipeline(func(p *inorbit.Pipeline) {
			for _, a := range c.Pipeline.Add {
				seen := &[]http.Header{}
				b.probes[a.Name] = seen
				probe := inorbit.Middleware{Name: a.Name, Wrap: func(next http.RoundTripper) http.RoundTripper {
					return inorbit.RoundTripperFunc(func(r *http.Request) (*http.Response, error) {
						*seen = append(*seen, r.Header.Clone())
						return next.RoundTrip(r)
					})
				}}
				switch {
				case a.Before != "":
					p.InsertBefore(a.Before, probe)
				case a.After != "":
					p.InsertAfter(a.After, probe)
				case a.Stage == "per_call":
					p.AddPerCall(probe)
				default:
					p.AddPerRetry(probe)
				}
			}
			for _, name := range c.Pipeline.Remove {
				p.Remove(name)
			}
		}))
	}
	var err error
	if c.Load {
		env := map[string]string{}
		for k, v := range c.Env {
			env[k] = substituteText(v, l.BaseURL, dir)
		}
		if c.ConfigFile != nil {
			file := filepath.Join(dir, "config.toml")
			if err := os.WriteFile(file, []byte(substituteText(*c.ConfigFile, l.BaseURL, dir)), 0o600); err != nil {
				return nil, err
			}
			opts = append(opts, inorbit.WithConfigFile(file))
		}
		if c.Profile != "" {
			opts = append(opts, inorbit.WithProfile(c.Profile))
		}
		opts = append(opts, inorbit.WithLoadOptions(inorbit.LoadOptions{Env: env, NoHome: true, Cwd: dir}))
		b.client, err = inorbit.Load(t.Context(), opts...)
	} else {
		opts = append(opts,
			inorbit.WithBaseURL(l.BaseURL), inorbit.WithTokenURL(l.BaseURL+"/oauth2/token"),
			inorbit.WithKey(or(c.KeyID, "ak_test"), or(c.KeySecret, "s3cr3t")),
			inorbit.WithScopes(orSlice(c.Scopes, []string{"identity:read"})...),
		)
		b.client, err = inorbit.NewClient(opts...)
	}
	if err != nil {
		return nil, err
	}
	return b, nil
}

// matches is a header matcher: a literal, * (present), $name (captured, then equal) or
// ~regex (the whole value).
func matches(want, got string, present bool, captures map[string]string) bool {
	if !present {
		return false
	}
	switch {
	case want == "*":
		return true
	case strings.HasPrefix(want, "$"):
		if seen, ok := captures[want]; ok {
			return seen == got
		}
		captures[want] = got
		return true
	case strings.HasPrefix(want, "~"):
		re, err := regexp.Compile("^(?:" + want[1:] + ")$")
		return err == nil && re.MatchString(got)
	}
	return want == got
}

// checkM6 compares what the M6 expectations add.
func checkM6(c testCase, b *built, last *inorbit.RawResponse) []string {
	var problems []string
	want := c.Expect
	for name, p := range want.Probes {
		var seen []http.Header
		if s := b.probes[name]; s != nil {
			seen = *s
		}
		if p.Count != nil && len(seen) != *p.Count {
			problems = append(problems, fmt.Sprintf("probe %s: ran %d times, want %d", name, len(seen), *p.Count))
		}
		captures := map[string]string{}
		for i, headers := range p.Seen {
			for h, v := range headers {
				var got string
				present := false
				if i < len(seen) {
					vals, ok := seen[i][http.CanonicalHeaderKey(h)]
					present = ok && len(vals) > 0
					if present {
						got = vals[0]
					}
				}
				if !matches(v, got, present, captures) {
					problems = append(problems, fmt.Sprintf("probe %s request %d: %s is %q, want %s", name, i+1, h, got, v))
				}
			}
		}
	}
	if want.Logs != nil {
		recs := b.logs.all()
		for _, w := range want.Logs.Contains {
			found := false
			for _, r := range recs {
				if subset(jsonValue(w), r) {
					found = true
					break
				}
			}
			if !found {
				text, _ := json.Marshal(recs)
				problems = append(problems, fmt.Sprintf("logs: no record holds %v: %s", w, text))
			}
		}
		text, _ := json.Marshal(recs)
		for _, x := range want.Logs.Excludes {
			if strings.Contains(string(text), x) {
				problems = append(problems, fmt.Sprintf("logs: %q appears in %s", x, text))
			}
		}
	}
	if want.Spans != nil {
		spans := b.tracer.all()
		kinds := map[string]inorbit.SpanKind{"internal": inorbit.SpanKindInternal, "client": inorbit.SpanKindClient}
		if len(spans) != len(want.Spans) {
			problems = append(problems, fmt.Sprintf("spans: want %d, got %+v", len(want.Spans), spans))
		} else {
			for i, w := range want.Spans {
				s := spans[i]
				if s.Name != w.Name || (w.Kind != "" && s.Kind != kinds[w.Kind]) || !subset(jsonValue(w.Attributes), jsonValue(s.Attributes)) {
					problems = append(problems, fmt.Sprintf("span %d: want %+v, got %+v", i, w, s))
				}
			}
		}
	}
	if len(want.RateLimit) > 0 {
		var w any
		_ = json.Unmarshal(want.RateLimit, &w)
		var got any
		if last != nil && last.RateLimit != nil {
			r := last.RateLimit
			m := map[string]any{}
			if r.Limit != nil {
				m["limit"] = *r.Limit
			}
			if r.Remaining != nil {
				m["remaining"] = *r.Remaining
			}
			if r.Reset != nil {
				m["reset_ms"] = r.Reset.Milliseconds()
			}
			got = jsonValue(m)
		}
		if w == nil && got != nil || w != nil && !subset(w, got) {
			problems = append(problems, fmt.Sprintf("rate_limit: want %v, got %v", w, got))
		}
	}
	if want.IdempotencyKey != "" {
		k := ""
		if last != nil {
			k = last.IdempotencyKey
		}
		if want.IdempotencyKey == "*" && k == "" || want.IdempotencyKey != "*" && k != want.IdempotencyKey {
			problems = append(problems, fmt.Sprintf("idempotency_key: want %s, got %q", want.IdempotencyKey, k))
		}
	}
	if want.Config != nil {
		d := jsonValue(b.client.Config().Describe())
		if !subset(jsonValue(want.Config), d) {
			problems = append(problems, fmt.Sprintf("config: want %v in %v", want.Config, d))
		}
	}
	return problems
}

// jsonValue is v as encoding/json reads it back: maps, slices and float64s.
func jsonValue(v any) any {
	b, _ := json.Marshal(v)
	var out any
	_ = json.Unmarshal(b, &out)
	return out
}

// records is a slog.Handler that keeps every record as a map, groups nested.
type records struct {
	mu   sync.Mutex
	list []map[string]any
}

func (*records) Enabled(context.Context, slog.Level) bool { return true }

func attrValue(v slog.Value) any {
	v = v.Resolve()
	if v.Kind() == slog.KindGroup {
		m := map[string]any{}
		for _, a := range v.Group() {
			m[a.Key] = attrValue(a.Value)
		}
		return m
	}
	return v.Any()
}

func (r *records) Handle(_ context.Context, rec slog.Record) error {
	m := map[string]any{}
	rec.Attrs(func(a slog.Attr) bool {
		m[a.Key] = attrValue(a.Value)
		return true
	})
	r.mu.Lock()
	r.list = append(r.list, jsonValue(m).(map[string]any))
	r.mu.Unlock()
	return nil
}

func (r *records) WithAttrs([]slog.Attr) slog.Handler { return r }
func (r *records) WithGroup(string) slog.Handler      { return r }

func (r *records) all() []any {
	r.mu.Lock()
	defer r.mu.Unlock()
	out := make([]any, len(r.list))
	for i, m := range r.list {
		out[i] = m
	}
	return out
}

// memTracer is an in-memory tracer: spans in start order, ids made up, the trace id
// taken from the parent in the context or the parent traceparent.
type memTracer struct {
	mu    sync.Mutex
	spans []*memSpan
}

type memSpan struct {
	Name       string
	Kind       inorbit.SpanKind
	Attributes map[string]any
	traceID    string
	spanID     string
	mu         *sync.Mutex
}

type spanKey struct{}

func randomHex(n int) string {
	b := make([]byte, n)
	_, _ = crand.Read(b)
	return hex.EncodeToString(b)
}

func (m *memTracer) Start(ctx context.Context, s inorbit.SpanStart) (context.Context, inorbit.Span) {
	traceID := randomHex(16)
	if parent, ok := ctx.Value(spanKey{}).(*memSpan); ok {
		traceID = parent.traceID
	} else if tid, _, ok := inorbit.ParseTraceparent(s.Parent); ok {
		traceID = tid
	}
	span := &memSpan{Name: s.Name, Kind: s.Kind, Attributes: map[string]any{}, traceID: traceID, spanID: randomHex(8), mu: &m.mu}
	for _, a := range s.Attributes {
		span.Attributes[a.Key] = a.Value
	}
	m.mu.Lock()
	m.spans = append(m.spans, span)
	m.mu.Unlock()
	return context.WithValue(ctx, spanKey{}, span), span
}

func (m *memTracer) all() []memSpan {
	m.mu.Lock()
	defer m.mu.Unlock()
	out := make([]memSpan, len(m.spans))
	for i, s := range m.spans {
		out[i] = memSpan{Name: s.Name, Kind: s.Kind, Attributes: s.Attributes}
	}
	return out
}

func (s *memSpan) SetAttributes(attrs ...inorbit.Attribute) {
	s.mu.Lock()
	defer s.mu.Unlock()
	for _, a := range attrs {
		s.Attributes[a.Key] = a.Value
	}
}

func (s *memSpan) SetError(errorType string) {
	s.SetAttributes(inorbit.Attribute{Key: "error.type", Value: errorType})
}

func (*memSpan) End() {}

func (s *memSpan) Traceparent() (string, string) {
	return "00-" + s.traceID + "-" + s.spanID + "-01", ""
}
