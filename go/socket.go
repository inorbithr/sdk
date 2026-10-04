package inorbit

import (
	"context"
	"encoding/json"
	"errors"
	"io"
	"iter"
	"net/http"
	"strconv"
	"sync"
	"time"
)

// The socket (docs/design.md section 7): every stream of a client over one /v1/ws
// connection, opened with the first stream and closed with the last. A stream is one
// call; a caller that stops sends cancel. An id-less error frame concerns the socket:
// unauthenticated (the key was revoked) ends every stream; anything else, a close, a
// broken connection or silence past the idle timeout reconnects and issues again every
// call that had not ended, which is safe because streams are reads.

// wsPath is the multiplexed socket.
const wsPath = "/v1/ws"

// socketMgr is a client's one socket and the calls on it.
type socketMgr struct {
	c *Client

	mu    sync.Mutex
	conn  *wsConn
	gen   int // bumped for every new connection, so a stale reader acts on nothing
	calls map[string]*sockCall
	next  uint64
}

// sockCall is one stream on the socket.
type sockCall struct {
	id    string
	frame []byte
	items chan sockItem
	gone  chan struct{} // closed when the caller has left
	ended bool          // an end or error was handed over; guarded by socketMgr.mu
}

type sockItem struct {
	data []byte
	err  error
	end  bool
}

// frame is one server frame.
type frame struct {
	Type    string          `json:"type"`
	ID      *string         `json:"id"`
	Body    json.RawMessage `json:"body"`
	Code    string          `json:"code"`
	Error   string          `json:"error"`
	Details []Detail        `json:"details"`
}

func (m *socketMgr) stream(ctx context.Context, op Operation) iter.Seq2[[]byte, error] {
	return func(yield func([]byte, error) bool) {
		call, err := m.start(ctx, op)
		if err != nil {
			yield(nil, err)
			return
		}
		defer m.leave(call)
		for {
			select {
			case <-ctx.Done():
				yield(nil, ctx.Err())
				return
			case it := <-call.items:
				switch {
				case it.err != nil:
					yield(nil, it.err)
					return
				case it.end:
					return
				}
				if !yield(it.data, nil) {
					return
				}
			}
		}
	}
}

// start registers a call, opens the socket if there is none, and sends the call frame.
func (m *socketMgr) start(ctx context.Context, op Operation) (*sockCall, error) {
	fields := op.Fields
	if fields == nil {
		fields = map[string]any{}
	}
	m.mu.Lock()
	defer m.mu.Unlock()
	m.next++
	id := strconv.FormatUint(m.next, 10)
	frameJSON, err := json.Marshal(map[string]any{"type": "call", "id": id, "method": op.RPC, "body": fields})
	if err != nil {
		return nil, &ConfigError{Message: "the call could not be written as JSON: " + err.Error()}
	}
	call := &sockCall{id: id, frame: frameJSON, items: make(chan sockItem, streamQueue), gone: make(chan struct{})}
	if m.conn == nil {
		if err := m.connectLocked(ctx, op); err != nil {
			return nil, err
		}
	}
	m.calls[id] = call
	if err := m.conn.write(opText, frameJSON); err != nil {
		// The reader sees the broken connection and issues the call again.
		m.conn.drop()
	}
	return call, nil
}

// leave ends the caller's part: a call that has not ended is cancelled; the socket
// closes with its last call.
func (m *socketMgr) leave(call *sockCall) {
	m.mu.Lock()
	defer m.mu.Unlock()
	if !call.ended {
		call.ended = true
		delete(m.calls, call.id)
		if m.conn != nil {
			cancel, _ := json.Marshal(map[string]string{"type": "cancel", "id": call.id})
			_ = m.conn.write(opText, cancel)
		}
	}
	close(call.gone)
	if len(m.calls) == 0 && m.conn != nil {
		m.conn.close()
		m.conn = nil
		m.gen++
	}
}

// connectLocked opens the socket, with the retry rules of a GET, and starts its reader.
func (m *socketMgr) connectLocked(ctx context.Context, op Operation) error {
	c := m.c
	u, err := c.url(Operation{Path: wsPath})
	if err != nil {
		return err
	}
	key, accept := wsKey()
	h := http.Header{}
	h.Set("Connection", "Upgrade")
	h.Set("Upgrade", "websocket")
	h.Set("Sec-WebSocket-Version", "13")
	h.Set("Sec-WebSocket-Key", key)
	// The socket outlives the caller that opened it: only the caller's values travel.
	resp, cancel, err := c.open(context.WithoutCancel(ctx), Operation{Name: op.Name, Path: wsPath}, u, h,
		func(status int) bool { return status == http.StatusSwitchingProtocols })
	if err != nil {
		return err
	}
	rwc, ok := resp.Body.(io.ReadWriteCloser)
	if !ok || resp.Header.Get("Sec-WebSocket-Accept") != accept {
		_ = resp.Body.Close()
		cancel()
		return &ConnectionError{Host: c.base.Host, Err: errors.New("the socket upgrade was not a WebSocket")}
	}
	m.gen++
	m.conn = newWSConn(rwc, cancel)
	go m.read(m.conn, m.gen)
	// A new connection issues again every call that had not ended on the last one.
	for _, call := range m.calls {
		if werr := m.conn.write(opText, call.frame); werr != nil {
			m.conn.drop()
			m.conn = nil
			m.gen++
			return &ConnectionError{Host: c.base.Host, Err: errors.New("the socket broke while its calls were issued again")}
		}
	}
	return nil
}

// read handles one connection's frames until it ends.
func (m *socketMgr) read(conn *wsConn, gen int) {
	var silent bool
	var smu sync.Mutex
	idle := time.AfterFunc(m.c.streamIdle, func() { smu.Lock(); silent = true; smu.Unlock(); conn.drop() })
	defer idle.Stop()
	for {
		op, data, err := conn.read()
		if err != nil {
			smu.Lock()
			s := silent
			smu.Unlock()
			if s {
				err = &TimeoutError{Host: m.c.base.Host, Seconds: int(m.c.streamIdle / time.Second)}
			}
			m.lost(gen, err)
			return
		}
		idle.Reset(m.c.streamIdle)
		if op != opText && op != opBinary {
			continue
		}
		var f frame
		if json.Unmarshal(data, &f) != nil {
			continue
		}
		switch f.Type {
		case "data":
			if f.ID != nil {
				m.deliver(gen, *f.ID, sockItem{data: f.Body}, false)
			}
		case "end":
			if f.ID != nil {
				m.deliver(gen, *f.ID, sockItem{end: true}, true)
			}
		case "error":
			e := problemOf(f.Code, f.Error, f.Details)
			if f.ID != nil {
				m.deliver(gen, *f.ID, sockItem{err: e}, true)
				continue
			}
			if e.Code == CodeUnauthenticated {
				m.failAll(gen, e)
				conn.close()
				return
			}
			m.lost(gen, e)
			return
		}
	}
}

// deliver hands an item to its call, waiting while the call's queue is full: a slow
// reader holds back the socket's reading, never memory. last ends the call.
func (m *socketMgr) deliver(gen int, id string, it sockItem, last bool) {
	m.mu.Lock()
	call := m.calls[id]
	if gen != m.gen || call == nil {
		m.mu.Unlock()
		return
	}
	if last {
		call.ended = true
		delete(m.calls, id)
	}
	m.mu.Unlock()
	select {
	case call.items <- it:
	case <-call.gone:
	}
}

// failAll ends every call with err; nothing reconnects.
func (m *socketMgr) failAll(gen int, err error) {
	m.mu.Lock()
	if gen != m.gen {
		m.mu.Unlock()
		return
	}
	calls := m.takeCallsLocked()
	if m.conn != nil {
		m.conn.drop()
		m.conn = nil
	}
	m.gen++
	m.mu.Unlock()
	for _, call := range calls {
		select {
		case call.items <- sockItem{err: err}:
		case <-call.gone:
		}
	}
}

func (m *socketMgr) takeCallsLocked() []*sockCall {
	calls := make([]*sockCall, 0, len(m.calls))
	for id, call := range m.calls {
		call.ended = true
		calls = append(calls, call)
		delete(m.calls, id)
	}
	return calls
}

// lost reconnects after the connection of gen ended; the new connection issues every
// open call again. When none can be had within the retry budget, every call ends with
// the error.
func (m *socketMgr) lost(gen int, cause error) {
	m.mu.Lock()
	if gen != m.gen {
		m.mu.Unlock()
		return
	}
	if m.conn != nil {
		m.conn.drop()
		m.conn = nil
	}
	m.gen++
	err := cause
	for attempt := 0; attempt <= m.c.maxRetries; attempt++ {
		if len(m.calls) == 0 || m.conn != nil {
			m.mu.Unlock()
			return
		}
		m.mu.Unlock()
		time.Sleep(backoff(attempt))
		m.mu.Lock()
		// A stream that started meanwhile may have connected already.
		if len(m.calls) == 0 || m.conn != nil {
			m.mu.Unlock()
			return
		}
		if err = m.connectLocked(context.Background(), Operation{Name: "socket"}); err == nil {
			m.mu.Unlock()
			return
		}
	}
	calls := m.takeCallsLocked()
	m.gen++
	m.mu.Unlock()
	if err == nil {
		err = &ConnectionError{Host: m.c.base.Host, Err: errors.New("the socket closed")}
	}
	for _, call := range calls {
		select {
		case call.items <- sockItem{err: err}:
		case <-call.gone:
		}
	}
}
