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
		h := http.Header{}
		h.Set("Accept", "text/event-stream")
		h.Set("Cache-Control", "no-cache")
		sctx, cancel := context.WithCancel(ctx)
		defer cancel()
		resp, err := c.open(sctx, op, h, func(status int) bool { return status >= 200 && status < 300 })
		if err != nil {
			yield(nil, err)
			return
		}
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

// open sends a GET for a stream through the pipeline, with the retry rules of any GET,
// and returns the answer whose status ok accepts with its body unread. Closing the body
// ends the request.
func (c *Client) open(ctx context.Context, op Operation, h http.Header, ok func(int) bool) (*http.Response, error) {
	started := time.Now()
	op.Method = http.MethodGet
	op.Body = nil
	resp, st, err := c.do(ctx, op, callKind{stream: true, header: h, ok: ok})
	if err != nil {
		return nil, err
	}
	if ok(resp.StatusCode) {
		return resp, nil
	}
	raw := c.rawOf(resp, st)
	apiErr := newAPIError(raw)
	c.failed(ctx, op, st, apiErr, started)
	return nil, apiErr
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
