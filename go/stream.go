package inorbit

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"io"
	"iter"
	"net/http"
	"net/url"
	"sync/atomic"
	"time"
)

// Streams is how a client opens streams (docs/design.md section 7).
type Streams string

const (
	// StreamsSSE opens each stream as one server-sent events request: the default.
	StreamsSSE Streams = "sse"

	// StreamsSocket opens every stream of the client over one /v1/ws socket, opened with
	// the first stream and closed with the last.
	StreamsSocket Streams = "socket"
)

const (
	// defaultStreamIdle is how long a stream may be silent; the server sends a keep-alive
	// every 15 s.
	defaultStreamIdle = 45 * time.Second

	// maxEvent bounds one event's data, 1 MiB.
	maxEvent = 1 << 20

	// streamQueue is how many items wait for a slow reader of one stream on the socket.
	streamQueue = 64
)

// Stream opens op, a streaming operation, and yields its events decoded as T, in order.
// The opening follows the rules of any GET: one fresh token after a 401, retries after a
// connection failure, 429, 503 or 504. An error (opening, an error event, silence past
// the idle timeout, ctx done) is yielded once with a nil item and ends the stream; the
// end of the stream ends the iteration. Breaking out of the loop closes the stream.
func Stream[T any](ctx context.Context, c *Client, op Operation) iter.Seq2[*T, error] {
	return func(yield func(*T, error) bool) {
		for data, err := range c.streamRaw(ctx, op) {
			if err != nil {
				yield(nil, err)
				return
			}
			v := new(T)
			if err := json.Unmarshal(data, v); err != nil {
				yield(nil, &DecodeError{Reason: err.Error()})
				return
			}
			if !yield(v, nil) {
				return
			}
		}
	}
}

func (c *Client) streamRaw(ctx context.Context, op Operation) iter.Seq2[[]byte, error] {
	if c.streams == StreamsSocket {
		if op.RPC == "" {
			return func(yield func([]byte, error) bool) {
				yield(nil, &ConfigError{Message: "the operation names no RPC, so it cannot stream over the socket; use StreamsSSE"})
			}
		}
		return c.socket.stream(ctx, op)
	}
	return c.sseStream(ctx, op)
}

// sseStream yields each default event's data; an error event, a broken body or silence
// past the idle timeout ends it with an error, the end of the body without one.
func (c *Client) sseStream(ctx context.Context, op Operation) iter.Seq2[[]byte, error] {
	return func(yield func([]byte, error) bool) {
		u, err := c.url(op)
		if err != nil {
			yield(nil, err)
			return
		}
		h := http.Header{}
		h.Set("Accept", "text/event-stream")
		h.Set("Cache-Control", "no-cache")
		resp, cancel, err := c.open(ctx, op, u, h, func(status int) bool { return status >= 200 && status < 300 })
		if err != nil {
			yield(nil, err)
			return
		}
		defer cancel()
		defer func() { _ = resp.Body.Close() }()
		idle := newIdleReader(resp.Body, c.streamIdle, cancel)
		defer idle.stop()
		p := &sseParser{r: bufio.NewReaderSize(idle, 64<<10)}
		for {
			name, data, err := p.next()
			if err != nil {
				switch {
				case idle.fired.Load():
					yield(nil, &TimeoutError{Host: c.base.Host, Seconds: int(c.streamIdle / time.Second)})
				case ctx.Err() != nil:
					yield(nil, ctx.Err())
				case errors.Is(err, io.EOF):
				default:
					var tooLarge *TooLargeError
					if !errors.As(err, &tooLarge) {
						err = &ConnectionError{Host: c.base.Host, Err: errors.New(describe(err))}
					}
					yield(nil, err)
				}
				return
			}
			switch name {
			case "", "message":
				if !yield(data, nil) {
					return
				}
			case "error":
				yield(nil, streamProblem(data))
				return
			}
		}
	}
}

// open sends a GET for a stream, with the retry rules of any GET, and returns the
// answer whose status ok accepts with its body unread. cancel ends the request; call it
// when done with the stream.
func (c *Client) open(ctx context.Context, op Operation, u *url.URL, h http.Header, ok func(int) bool) (*http.Response, context.CancelFunc, error) {
	id := requestID()
	retries, refreshed := 0, false
	name := op.Name
	if name == "" {
		name = op.Path
	}
	for number := 1; ; number++ {
		a := Attempt{Operation: name, Method: http.MethodGet, Path: op.Path, Number: number, RequestID: id}
		resp, cancel, raw, wait, kind, err := c.openOnce(ctx, u, h, ok, a)
		if kind == done && err == nil && resp != nil {
			return resp, cancel, nil
		}
		if kind == unauthorized && !refreshed {
			if inv, ok := c.provider.(Invalidator); ok {
				inv.Invalidate()
			}
			refreshed = true
			continue
		}
		if kind == retry && retries < c.maxRetries {
			if wait < 0 {
				wait = backoff(retries)
			}
			if serr := sleep(ctx, wait); serr != nil {
				c.failed(a, serr)
				return nil, nil, serr
			}
			retries++
			continue
		}
		if err == nil {
			err = newAPIError(raw)
		}
		c.failed(a, err)
		return nil, nil, err
	}
}

// openOnce sends one attempt. The answer's headers must come within the client's
// timeout; the body is then the stream's and has no deadline.
func (c *Client) openOnce(ctx context.Context, u *url.URL, h http.Header, ok func(int) bool, a Attempt) (*http.Response, context.CancelFunc, *RawResponse, time.Duration, outcome, error) {
	tok, err := c.provider.Token(ctx)
	if err != nil {
		var aerr *AuthError
		if !errors.As(err, &aerr) {
			err = &AuthError{Message: "the token provider failed: " + err.Error(), Err: err}
		}
		return nil, nil, nil, 0, done, err
	}
	sctx, cancel := context.WithCancel(ctx)
	var late atomic.Bool
	timer := time.AfterFunc(c.timeout, func() { late.Store(true); cancel() })
	req, err := http.NewRequestWithContext(sctx, http.MethodGet, u.String(), nil)
	if err != nil {
		timer.Stop()
		cancel()
		return nil, nil, nil, 0, done, &ConfigError{Message: "the request could not be built: " + err.Error()}
	}
	for k, v := range h {
		req.Header[k] = v
	}
	req.Header.Set("Authorization", "Bearer "+tok.Access)
	req.Header.Set("User-Agent", c.userAgent)
	req.Header.Set("X-Request-Id", a.RequestID)
	for _, hk := range c.hooks {
		hk.OnRequest(a)
	}
	resp, err := c.http.Do(req)
	timer.Stop()
	if err != nil {
		cancel()
		if ctx.Err() != nil {
			return nil, nil, nil, 0, done, ctx.Err()
		}
		if late.Load() || isTimeout(err) {
			return nil, nil, nil, -1, retry, &TimeoutError{Host: c.base.Host, Seconds: int(c.timeout.Round(time.Second) / time.Second)}
		}
		return nil, nil, nil, -1, retry, &ConnectionError{Host: c.base.Host, Err: errors.New(describe(err))}
	}
	raw := &RawResponse{
		Status: resp.StatusCode, Header: resp.Header, RequestID: a.RequestID,
		ServerRequestID: resp.Header.Get("X-Request-Id"), Attempts: a.Number,
	}
	if ok(resp.StatusCode) {
		for _, hk := range c.hooks {
			hk.OnResponse(a, raw)
		}
		return resp, cancel, raw, 0, done, nil
	}
	data, rerr := io.ReadAll(io.LimitReader(resp.Body, maxBody))
	_ = resp.Body.Close()
	cancel()
	if rerr != nil && ctx.Err() == nil {
		return nil, nil, nil, -1, retry, &ConnectionError{Host: c.base.Host, Err: errors.New(describe(rerr))}
	}
	raw.Body = data
	for _, hk := range c.hooks {
		hk.OnResponse(a, raw)
	}
	switch {
	case raw.Status == http.StatusUnauthorized:
		return nil, nil, raw, 0, unauthorized, nil
	case retryableStatus(raw.Status):
		wait, has := retryAfter(raw.Header)
		if !has {
			wait = -1
		}
		return nil, nil, raw, wait, retry, nil
	}
	return nil, nil, raw, 0, done, newAPIError(raw)
}

// streamProblem is the error an error event or an error frame carries: the envelope,
// its status from the code (spec/problem.json, x-http-status).
func streamProblem(data []byte) *APIError {
	var wire struct {
		Code    string   `json:"code"`
		Error   string   `json:"error"`
		Details []Detail `json:"details"`
	}
	_ = json.Unmarshal(data, &wire)
	return problemOf(wire.Code, wire.Error, wire.Details)
}

func problemOf(code, message string, details []Detail) *APIError {
	e := &APIError{Code: Code(code), Details: details}
	if e.Code == "" {
		e.Code = CodeInternal
	}
	e.Status = codeStatus[e.Code]
	e.Problem = truncate(message, 300)
	if e.Problem == "" {
		e.Problem = gatewayMessage(e.Status)
	}
	return e
}

// idleReader ends the request when nothing arrives for d: the stream's idle timeout.
type idleReader struct {
	r     io.Reader
	d     time.Duration
	timer *time.Timer
	fired atomic.Bool
}

func newIdleReader(r io.Reader, d time.Duration, cancel context.CancelFunc) *idleReader {
	ir := &idleReader{r: r, d: d}
	ir.timer = time.AfterFunc(d, func() { ir.fired.Store(true); cancel() })
	return ir
}

func (ir *idleReader) Read(p []byte) (int, error) {
	n, err := ir.r.Read(p)
	if n > 0 && !ir.fired.Load() {
		ir.timer.Reset(ir.d)
	}
	return n, err
}

func (ir *idleReader) stop() { ir.timer.Stop() }

// sseParser reads events by the WHATWG event-stream rules: lines end with LF, CRLF or
// CR; a line starting with ':' is a comment; data lines join with a newline; a blank
// line dispatches; id, retry and unknown fields are ignored.
type sseParser struct {
	r *bufio.Reader
	// skipLF drops an LF that follows a CR read at the end of the buffer.
	skipLF bool
}

// line reads one line without its end, at most maxEvent bytes.
func (p *sseParser) line() ([]byte, error) {
	var out []byte
	for {
		b, err := p.r.ReadByte()
		if err != nil {
			return nil, err
		}
		if p.skipLF {
			p.skipLF = false
			if b == '\n' {
				continue
			}
		}
		switch b {
		case '\n':
			return out, nil
		case '\r':
			if p.r.Buffered() > 0 {
				if next, _ := p.r.Peek(1); next[0] == '\n' {
					_, _ = p.r.ReadByte()
				}
			} else {
				p.skipLF = true
			}
			return out, nil
		}
		out = append(out, b)
		if len(out) > maxEvent {
			return nil, &TooLargeError{Event: true}
		}
	}
}

// next reads up to the next event with data, its name ("" for the default) and data.
func (p *sseParser) next() (string, []byte, error) {
	var data bytes.Buffer
	has, name := false, ""
	for {
		l, err := p.line()
		if err != nil {
			return "", nil, err
		}
		if len(l) == 0 {
			if has {
				return name, data.Bytes(), nil
			}
			name = ""
			continue
		}
		if l[0] == ':' {
			continue
		}
		field, value := l, []byte(nil)
		if i := bytes.IndexByte(l, ':'); i >= 0 {
			field, value = l[:i], l[i+1:]
			if len(value) > 0 && value[0] == ' ' {
				value = value[1:]
			}
		}
		switch string(field) {
		case "data":
			if has {
				data.WriteByte('\n')
			}
			data.Write(value)
			has = true
			if data.Len() > maxEvent {
				return "", nil, &TooLargeError{Event: true}
			}
		case "event":
			name = string(value)
		}
	}
}
