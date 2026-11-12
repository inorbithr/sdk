package main

import (
	"encoding/json"
	"log/slog"
	"net/http"
	"sync/atomic"
	"time"
	"unicode/utf8"
)

// service holds what the handlers share. ready is false until the server listens and again as
// soon as shutdown starts, so a load balancer stops sending traffic before the process stops.
type service struct {
	log     *slog.Logger
	metrics *metrics
	ready   atomic.Bool
}

func newService(log *slog.Logger) *service {
	return &service{log: log, metrics: newMetrics()}
}

// statusWriter remembers the status a handler wrote, for the log, the span and the counters.
type statusWriter struct {
	http.ResponseWriter
	status int
}

func (w *statusWriter) WriteHeader(code int) {
	w.status = code
	w.ResponseWriter.WriteHeader(code)
}

func writeJSON(w http.ResponseWriter, code int, v any) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(code)
	_ = json.NewEncoder(w).Encode(v)
}

// handler routes by pattern and wraps every route in observe, the one place a request is
// traced, logged and counted.
func (s *service) handler() http.Handler {
	mux := http.NewServeMux()
	mux.Handle("GET /healthz", s.observe("/healthz", func(w http.ResponseWriter, _ *http.Request) {
		writeJSON(w, http.StatusOK, map[string]string{"status": "ok"})
	}))
	mux.Handle("GET /readyz", s.observe("/readyz", func(w http.ResponseWriter, _ *http.Request) {
		if !s.ready.Load() {
			writeJSON(w, http.StatusServiceUnavailable, map[string]string{"status": "not ready"})
			return
		}
		writeJSON(w, http.StatusOK, map[string]string{"status": "ready"})
	}))
	mux.Handle("GET /metrics", s.observe("/metrics", func(w http.ResponseWriter, _ *http.Request) {
		w.Header().Set("Content-Type", "text/plain; version=0.0.4")
		_, _ = w.Write([]byte(s.metrics.render()))
	}))
	// Your first route: replace it. The name comes from the query string, so it is bounded and
	// echoed back, never logged.
	mux.Handle("GET /v1/hello", s.observe("/v1/hello", func(w http.ResponseWriter, r *http.Request) {
		name := r.URL.Query().Get("name")
		if name == "" {
			name = "world"
		}
		if len(name) > 64 || !utf8.ValidString(name) {
			writeJSON(w, http.StatusBadRequest, map[string]string{"error": "name is at most 64 characters of text"})
			return
		}
		writeJSON(w, http.StatusOK, map[string]string{"message": "Hello, " + name})
	}))
	mux.Handle("/", s.observe("unmatched", func(w http.ResponseWriter, _ *http.Request) {
		writeJSON(w, http.StatusNotFound, map[string]string{"error": "not found"})
	}))
	return mux
}

// observe traces, logs and counts one request. route is the pattern, not the path.
func (s *service) observe(route string, next http.HandlerFunc) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		sp := startSpan(r)
		w.Header().Set("traceparent", sp.header())
		sw := &statusWriter{ResponseWriter: w, status: http.StatusOK}
		next(sw, r)
		ms := float64(time.Since(sp.start).Microseconds()) / 1000
		attrs := []any{
			slog.String("trace_id", sp.traceID), slog.String("span_id", sp.spanID),
			slog.String("name", r.Method+" "+route), slog.String("route", route),
			slog.Int("status", sw.status), slog.Float64("duration_ms", ms),
		}
		if sp.parentID != "" {
			attrs = append(attrs, slog.String("parent_span_id", sp.parentID))
		}
		s.log.Info("span", attrs...)
		s.log.Info("request",
			slog.String("trace_id", sp.traceID), slog.String("method", r.Method),
			slog.String("route", route), slog.Int("status", sw.status), slog.Float64("duration_ms", ms))
		s.metrics.inc(route, sw.status)
	})
}
