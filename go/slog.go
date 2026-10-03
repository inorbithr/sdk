package inorbit

import (
	"context"
	"log/slog"
	"strings"
	"sync"
	"time"
)

// SlogHook returns a Hook that logs every call to logger: one Debug record per answer
// (operation, method, status, attempt, duration, request ids) and one Warn record when
// a call fails for good (operation, method, attempt, error).
//
// It never logs what could carry a secret or someone's data: no bound path (an id is
// data), no query value, no header, no body, no token. A raw call made with Send, which
// has no operation name, is logged as "raw".
func SlogHook(logger *slog.Logger) Hook {
	return &slogHook{logger: logger, started: map[attemptKey]time.Time{}}
}

type attemptKey struct {
	requestID string
	number    int
}

type slogHook struct {
	logger *slog.Logger

	mu      sync.Mutex
	started map[attemptKey]time.Time
}

// operation is the name to log: the operation's, or "raw" when the runtime only knows
// the bound path.
func operation(a Attempt) string {
	if a.Operation == "" || strings.HasPrefix(a.Operation, "/") {
		return "raw"
	}
	return a.Operation
}

func (h *slogHook) OnRequest(a Attempt) {
	h.mu.Lock()
	h.started[attemptKey{a.RequestID, a.Number}] = time.Now()
	h.mu.Unlock()
}

func (h *slogHook) took(a Attempt) time.Duration {
	h.mu.Lock()
	defer h.mu.Unlock()
	k := attemptKey{a.RequestID, a.Number}
	start, ok := h.started[k]
	delete(h.started, k)
	if !ok {
		return 0
	}
	return time.Since(start)
}

func (h *slogHook) OnResponse(a Attempt, r *RawResponse) {
	took := h.took(a)
	ctx := context.Background()
	if !h.logger.Enabled(ctx, slog.LevelDebug) {
		return
	}
	h.logger.LogAttrs(ctx, slog.LevelDebug, "inorbit: answer",
		slog.String("operation", operation(a)),
		slog.String("method", a.Method),
		slog.Int("status", r.Status),
		slog.Int("attempt", a.Number),
		slog.Duration("duration", took),
		slog.String("request_id", a.RequestID),
		slog.String("server_request_id", r.ServerRequestID),
	)
}

func (h *slogHook) OnError(a Attempt, err error) {
	h.took(a)
	h.logger.LogAttrs(context.Background(), slog.LevelWarn, "inorbit: call failed",
		slog.String("operation", operation(a)),
		slog.String("method", a.Method),
		slog.Int("attempt", a.Number),
		slog.String("request_id", a.RequestID),
		slog.String("error", err.Error()),
	)
}
