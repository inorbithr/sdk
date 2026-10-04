package main

import (
	"bufio"
	"crypto/sha1" //nolint:gosec // RFC 6455 fixes SHA-1 for the handshake; nothing here is secret.
	"encoding/base64"
	"encoding/binary"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"strings"
	"time"
)

// SSE is a server-sent events answer: the events in order, then the body ends, or stays
// open and silent for HoldMS first (what a client's idle timeout is tested against).
type SSE struct {
	Events []SSEEvent `yaml:"events" json:"events"`
	HoldMS int        `yaml:"hold_ms" json:"hold_ms,omitempty"`
}

// SSEEvent is one item of the stream: a comment line, an event with JSON data (named by
// Event, default "message"), raw text written as is, or a pause.
type SSEEvent struct {
	Comment *string `yaml:"comment" json:"comment,omitempty"`
	Event   string  `yaml:"event" json:"event,omitempty"`
	Data    any     `yaml:"data" json:"data,omitempty"`
	Raw     *string `yaml:"raw" json:"raw,omitempty"`
	DelayMS int     `yaml:"delay_ms" json:"delay_ms,omitempty"`
}

// Socket is a WebSocket answer: after the upgrade, the steps run in order. A step
// expects one client frame (a subset of its JSON; `as` names the call id it carries),
// sends one frame (a string "$name" becomes the id named before), pauses, or closes.
// When the steps are done the server waits for the client to close, at most 5 s.
type Socket struct {
	Steps []SocketStep `yaml:"steps" json:"steps"`
}

// SocketStep is one step of a Socket answer.
type SocketStep struct {
	Expect  any    `yaml:"expect" json:"expect,omitempty"`
	As      string `yaml:"as" json:"as,omitempty"`
	Send    any    `yaml:"send" json:"send,omitempty"`
	DelayMS int    `yaml:"delay_ms" json:"delay_ms,omitempty"`
	Close   int    `yaml:"close" json:"close,omitempty"`
}

// wsGUID is the constant RFC 6455 section 1.3 appends to the client's key.
const wsGUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"

// maxFrame bounds one frame the server reads, as the platform does (256 KiB).
const maxFrame = 256 << 10

// respondSSE writes an SSE answer, flushing after each event.
func (s *Server) respondSSE(w http.ResponseWriter, status int, resp Response) {
	for k, v := range resp.Headers {
		w.Header().Set(k, v)
	}
	w.Header().Set("content-type", "text/event-stream")
	w.Header().Set("cache-control", "no-cache")
	if status == 0 {
		status = http.StatusOK
	}
	w.WriteHeader(status)
	flusher, _ := w.(http.Flusher)
	flush := func() {
		if flusher != nil {
			flusher.Flush()
		}
	}
	flush()
	for _, ev := range resp.SSE.Events {
		if ev.DelayMS > 0 {
			s.sleep(time.Duration(ev.DelayMS) * time.Millisecond)
		}
		var b strings.Builder
		switch {
		case ev.Comment != nil:
			b.WriteString(": " + *ev.Comment + "\n\n")
		case ev.Raw != nil:
			b.WriteString(*ev.Raw)
		case ev.Data != nil:
			data, err := json.Marshal(ev.Data)
			if err != nil {
				return
			}
			if ev.Event != "" {
				b.WriteString("event: " + ev.Event + "\n")
			}
			b.WriteString("data: " + string(data) + "\n\n")
		default:
			continue
		}
		if _, err := io.WriteString(w, b.String()); err != nil {
			return
		}
		flush()
	}
	if resp.SSE.HoldMS > 0 {
		s.sleep(time.Duration(resp.SSE.HoldMS) * time.Millisecond)
	}
}

// respondSocket upgrades the connection and runs the steps. A client frame that does not
// match its step fails the case, like a request that does not match its exchange.
func (s *Server) respondSocket(w http.ResponseWriter, r *http.Request, resp Response) {
	conn, rw, err := upgrade(w, r)
	if err != nil {
		writeJSON(w, http.StatusBadRequest, problem("bad_request", err.Error()))
		return
	}
	defer func() { _ = conn.Close() }()
	s.mu.Lock()
	session := s.session
	s.mu.Unlock()
	ids := map[string]string{}
	for i, step := range resp.Socket.Steps {
		if step.DelayMS > 0 {
			s.sleep(time.Duration(step.DelayMS) * time.Millisecond)
		}
		switch {
		case step.Expect != nil:
			_ = conn.SetReadDeadline(time.Now().Add(10 * time.Second))
			frame, err := readText(rw, conn)
			if err != nil {
				s.failSocket(session, i, "the client sent no frame: "+err.Error(), step.Expect, nil)
				return
			}
			var got any
			if err := json.Unmarshal(frame, &got); err != nil {
				s.failSocket(session, i, "the frame is not JSON", step.Expect, string(frame))
				return
			}
			want := substitute(step.Expect, ids)
			if path, ok := subset(want, got, "$"); !ok {
				s.failSocket(session, i, "frame: "+path+" differs", want, got)
				return
			}
			if step.As != "" {
				if m, ok := got.(map[string]any); ok {
					id, _ := m["id"].(string)
					ids[step.As] = id
				}
			}
		case step.Send != nil:
			data, err := json.Marshal(substitute(step.Send, ids))
			if err != nil {
				return
			}
			if err := writeFrame(rw.Writer, 0x1, data); err != nil {
				return
			}
		case step.Close != 0:
			payload := make([]byte, 2)
			binary.BigEndian.PutUint16(payload, uint16(step.Close)) //nolint:gosec // a close code fits
			_ = writeFrame(rw.Writer, 0x8, payload)
			return
		}
	}
	// The steps are done: wait for the client's close (or its silence), answering pings.
	_ = conn.SetReadDeadline(time.Now().Add(5 * time.Second))
	for {
		if _, err := readText(rw, conn); err != nil {
			return
		}
	}
}

// failSocket records a socket step that did not go as the case says.
func (s *Server) failSocket(session, step int, reason string, want, got any) {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.fail == nil && s.session == session {
		s.fail = &Mismatch{
			Index:  max(s.next-1, 0),
			Reason: fmt.Sprintf("socket step %d: %s", step, reason),
			Actual: Seen{Method: "WS", Path: "frame", JSON: map[string]any{"want": want, "got": got}},
		}
	}
}

// substitute replaces "$name" strings with the id captured under that name.
func substitute(v any, ids map[string]string) any {
	switch t := v.(type) {
	case string:
		if strings.HasPrefix(t, "$") {
			if id, ok := ids[t[1:]]; ok {
				return id
			}
		}
		return t
	case map[string]any:
		out := make(map[string]any, len(t))
		for k, x := range t {
			out[k] = substitute(x, ids)
		}
		return out
	case []any:
		out := make([]any, len(t))
		for i, x := range t {
			out[i] = substitute(x, ids)
		}
		return out
	default:
		return v
	}
}

// upgrade answers a WebSocket handshake (RFC 6455 section 4.2) and hands back the
// connection.
func upgrade(w http.ResponseWriter, r *http.Request) (net.Conn, *bufio.ReadWriter, error) {
	key := r.Header.Get("Sec-WebSocket-Key")
	if key == "" || !strings.EqualFold(r.Header.Get("Upgrade"), "websocket") {
		return nil, nil, errors.New("a WebSocket upgrade was expected")
	}
	hj, ok := w.(http.Hijacker)
	if !ok {
		return nil, nil, errors.New("the connection cannot be taken over")
	}
	conn, rw, err := hj.Hijack()
	if err != nil {
		return nil, nil, err
	}
	sum := sha1.Sum([]byte(key + wsGUID)) //nolint:gosec // RFC 6455
	accept := base64.StdEncoding.EncodeToString(sum[:])
	_, _ = rw.WriteString("HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n" +
		"Sec-WebSocket-Accept: " + accept + "\r\n\r\n")
	if err := rw.Flush(); err != nil {
		_ = conn.Close()
		return nil, nil, err
	}
	return conn, rw, nil
}

// readText reads frames until a whole text or binary message, answering pings; a close
// frame or any error ends it.
func readText(rw *bufio.ReadWriter, _ net.Conn) ([]byte, error) {
	var message []byte
	for {
		fin, op, payload, err := readFrame(rw.Reader)
		if err != nil {
			return nil, err
		}
		switch op {
		case 0x8:
			_ = writeFrame(rw.Writer, 0x8, payload)
			return nil, io.EOF
		case 0x9:
			if err := writeFrame(rw.Writer, 0xA, payload); err != nil {
				return nil, err
			}
			continue
		case 0xA:
			continue
		}
		message = append(message, payload...)
		if len(message) > maxFrame {
			return nil, errors.New("a message over 256 KiB")
		}
		if fin {
			return message, nil
		}
	}
}

// readFrame reads one frame, unmasking a client's payload.
func readFrame(r *bufio.Reader) (fin bool, op byte, payload []byte, err error) {
	var head [2]byte
	if _, err = io.ReadFull(r, head[:]); err != nil {
		return false, 0, nil, err
	}
	fin = head[0]&0x80 != 0
	op = head[0] & 0x0F
	masked := head[1]&0x80 != 0
	n := uint64(head[1] & 0x7F)
	switch n {
	case 126:
		var ext [2]byte
		if _, err = io.ReadFull(r, ext[:]); err != nil {
			return false, 0, nil, err
		}
		n = uint64(binary.BigEndian.Uint16(ext[:]))
	case 127:
		var ext [8]byte
		if _, err = io.ReadFull(r, ext[:]); err != nil {
			return false, 0, nil, err
		}
		n = binary.BigEndian.Uint64(ext[:])
	}
	if n > maxFrame {
		return false, 0, nil, errors.New("a frame over 256 KiB")
	}
	var mask [4]byte
	if masked {
		if _, err = io.ReadFull(r, mask[:]); err != nil {
			return false, 0, nil, err
		}
	}
	payload = make([]byte, n)
	if _, err = io.ReadFull(r, payload); err != nil {
		return false, 0, nil, err
	}
	if masked {
		for i := range payload {
			payload[i] ^= mask[i%4]
		}
	}
	return fin, op, payload, nil
}

// writeFrame writes one unmasked, final frame (a server's frames are never masked).
func writeFrame(w *bufio.Writer, op byte, payload []byte) error {
	return writeFrameMasked(w, op, payload, nil)
}

// writeFrameMasked writes one final frame, masked with mask when it is given (a client's).
func writeFrameMasked(w *bufio.Writer, op byte, payload []byte, mask []byte) error {
	head := []byte{0x80 | op}
	maskBit := byte(0)
	if mask != nil {
		maskBit = 0x80
	}
	switch n := len(payload); {
	case n < 126:
		head = append(head, maskBit|byte(n))
	case n <= 0xFFFF:
		head = append(head, maskBit|126, byte(n>>8), byte(n))
	default:
		head = append(head, maskBit|127)
		head = binary.BigEndian.AppendUint64(head, uint64(n))
	}
	body := payload
	if mask != nil {
		head = append(head, mask...)
		body = make([]byte, len(payload))
		for i := range payload {
			body[i] = payload[i] ^ mask[i%4]
		}
	}
	if _, err := w.Write(head); err != nil {
		return err
	}
	if _, err := w.Write(body); err != nil {
		return err
	}
	return w.Flush()
}
