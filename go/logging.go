package inorbit

import (
	"context"
	"errors"
	"log/slog"
	"net/http"
	"slices"
	"strings"
)

// Logging (docs/config.md section 7.9): off until the log setting is on; records go to a
// *slog.Logger (WithLogger, or slog.Default), hold metadata only, and headers only from
// the allowlist.

// LogLevel is how much is logged.
type LogLevel string

// The levels, from nothing to everything.
const (
	LogOff   LogLevel = "off"
	LogError LogLevel = "error"
	LogWarn  LogLevel = "warn"
	LogInfo  LogLevel = "info"
	LogDebug LogLevel = "debug"
)

var logLevels = []LogLevel{LogOff, LogError, LogWarn, LogInfo, LogDebug}

func (l LogLevel) slog() slog.Level {
	switch l {
	case LogError:
		return slog.LevelError
	case LogWarn:
		return slog.LevelWarn
	case LogInfo:
		return slog.LevelInfo
	}
	return slog.LevelDebug
}

// LogRecord is one record before it reaches the logger: its level, its event (request,
// response, call, retry, token_refresh_failed, rate_limit_wait, call_failed) and its
// fields.
type LogRecord struct {
	Level LogLevel
	Event string
	Attrs []slog.Attr
}

// Redact sees every record last (SR-14): it returns the record, changed or not, and
// true, or false to drop it.
type Redact func(LogRecord) (LogRecord, bool)

// neverLogged are the headers never logged, whatever the settings.
var neverLogged = []string{"authorization", "proxy-authorization", "cookie", "set-cookie"}

var requestAllowlist = []string{
	"accept", "content-type", "content-length", "user-agent", "x-request-id", "traceparent", "idempotency-key",
}

var responseAllowlist = []string{
	"content-type", "content-length", "date", "retry-after", "x-request-id", "idempotency-replayed",
	"x-ratelimit-limit", "x-ratelimit-remaining", "x-ratelimit-reset", "ratelimit", "ratelimit-policy",
}

// logSink is the log of one client.
type logSink struct {
	level    int
	logger   *slog.Logger
	redact   Redact
	profile  string
	headers  bool
	request  []string
	response []string
}

func newLogSink(level LogLevel, logger *slog.Logger, redact Redact, profile string, headers bool, allow []string) *logSink {
	s := &logSink{
		level: slices.Index(logLevels, level), logger: logger, redact: redact, profile: profile, headers: headers,
		request: slices.Clone(requestAllowlist), response: slices.Clone(responseAllowlist),
	}
	if s.level < 0 {
		s.level = 0
	}
	for _, h := range allow {
		h = strings.ToLower(h)
		if !slices.Contains(neverLogged, h) {
			s.request = append(s.request, h)
			s.response = append(s.response, h)
		}
	}
	return s
}

// on reports whether records at level are kept.
func (s *logSink) on(level LogLevel) bool {
	return s != nil && s.level >= slices.Index(logLevels, level)
}

// show is h as a record may show it: allowlisted values, every other name REDACTED.
func (s *logSink) show(h http.Header, response bool) slog.Attr {
	allow := s.request
	if response {
		allow = s.response
	}
	names := make([]string, 0, len(h))
	for k := range h {
		names = append(names, k)
	}
	slices.Sort(names)
	attrs := make([]any, 0, len(names))
	for _, k := range names {
		name := strings.ToLower(k)
		value := "REDACTED"
		if slices.Contains(allow, name) && !slices.Contains(neverLogged, name) {
			value = strings.Join(h[k], ", ")
		}
		attrs = append(attrs, slog.String(name, value))
	}
	return slog.Group("headers", attrs...)
}

// emit logs event at level with attrs, when the level is on.
func (s *logSink) emit(ctx context.Context, level LogLevel, event string, attrs ...slog.Attr) {
	if !s.on(level) {
		return
	}
	r := LogRecord{Level: level, Event: event, Attrs: attrs}
	if s.profile != "" {
		r.Attrs = append(r.Attrs, slog.String("profile", s.profile))
	}
	if s.redact != nil {
		var keep bool
		func() {
			defer func() {
				if recover() != nil {
					keep = false
				}
			}()
			r, keep = s.redact(r)
		}()
		if !keep {
			return
		}
	}
	logger := s.logger
	if logger == nil {
		logger = slog.Default()
	}
	all := append([]slog.Attr{slog.String("event", r.Event)}, r.Attrs...)
	logger.LogAttrs(context.WithoutCancel(ctx), r.Level.slog(), "inorbithr "+r.Event, all...)
}

// errorKind is an error's stable kind: api, connection, timeout, auth, config,
// too_large, decode, cancelled.
func errorKind(err error) string {
	var (
		a *APIError
		c *ConnectionError
		t *TimeoutError
		u *AuthError
		g *ConfigError
		l *TooLargeError
		d *DecodeError
	)
	switch {
	case errors.As(err, &a):
		return "api"
	case errors.As(err, &c):
		return "connection"
	case errors.As(err, &t):
		return "timeout"
	case errors.As(err, &u):
		return "auth"
	case errors.As(err, &g):
		return "config"
	case errors.As(err, &l):
		return "too_large"
	case errors.As(err, &d):
		return "decode"
	case errors.Is(err, context.Canceled), errors.Is(err, context.DeadlineExceeded):
		return "cancelled"
	}
	return "error"
}

func asAPIError(err error) (*APIError, bool) {
	var a *APIError
	ok := errors.As(err, &a)
	return a, ok
}
