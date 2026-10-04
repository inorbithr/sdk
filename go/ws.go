package inorbit

import (
	"bufio"
	"crypto/rand"
	"crypto/sha1" //nolint:gosec // RFC 6455 fixes SHA-1 for the handshake; nothing here is secret.
	"encoding/base64"
	"encoding/binary"
	"errors"
	"io"
	"sync"
)

// The WebSocket client the socket needs, RFC 6455, and nothing more: the upgrade goes
// through the client's own http.Client (its transport, proxy and TLS settings), and this
// file reads and writes frames over the connection it hands back. Client frames are
// masked; fragments are joined; a ping is answered; a close ends the reading.

const (
	wsGUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"

	opContinuation = 0x0
	opText         = 0x1
	opBinary       = 0x2
	opClose        = 0x8
	opPing         = 0x9
	opPong         = 0xA
)

// errWSClosed is a close frame from the server.
var errWSClosed = errors.New("the server closed the socket")

// wsKey is a fresh Sec-WebSocket-Key and the Sec-WebSocket-Accept the server must answer.
func wsKey() (key, accept string) {
	var b [16]byte
	_, _ = rand.Read(b[:])
	key = base64.StdEncoding.EncodeToString(b[:])
	sum := sha1.Sum([]byte(key + wsGUID)) //nolint:gosec // RFC 6455
	return key, base64.StdEncoding.EncodeToString(sum[:])
}

// wsConn is one upgraded connection.
type wsConn struct {
	rwc    io.ReadWriteCloser
	br     *bufio.Reader
	wmu    sync.Mutex
	once   sync.Once
	cancel func()
}

func newWSConn(rwc io.ReadWriteCloser, cancel func()) *wsConn {
	return &wsConn{rwc: rwc, br: bufio.NewReaderSize(rwc, 32<<10), cancel: cancel}
}

// write sends one final, masked frame.
func (w *wsConn) write(op byte, payload []byte) error {
	var mask [4]byte
	_, _ = rand.Read(mask[:])
	head := []byte{0x80 | op}
	switch n := len(payload); {
	case n < 126:
		head = append(head, 0x80|byte(n)) //nolint:gosec // n < 126
	case n <= 0xFFFF:
		head = append(head, 0x80|126)
		head = binary.BigEndian.AppendUint16(head, uint16(n)) //nolint:gosec // n fits: checked above
	default:
		head = append(head, 0x80|127)
		head = binary.BigEndian.AppendUint64(head, uint64(n))
	}
	head = append(head, mask[:]...)
	frame := make([]byte, 0, len(head)+len(payload))
	frame = append(frame, head...)
	for i, b := range payload {
		frame = append(frame, b^mask[i%4])
	}
	w.wmu.Lock()
	defer w.wmu.Unlock()
	_, err := w.rwc.Write(frame)
	return err
}

// read returns the next message: a text or binary message whole, or a pong (as op with
// no data, so the caller sees the socket alive). A ping is answered here and also
// reported as alive; a close frame is errWSClosed.
func (w *wsConn) read() (byte, []byte, error) {
	var msg []byte
	var msgOp byte
	for {
		fin, op, payload, err := w.frame()
		if err != nil {
			return 0, nil, err
		}
		switch op {
		case opClose:
			_ = w.write(opClose, payload)
			return 0, nil, errWSClosed
		case opPing:
			if err := w.write(opPong, payload); err != nil {
				return 0, nil, err
			}
			return opPing, nil, nil
		case opPong:
			return opPong, nil, nil
		case opText, opBinary:
			msgOp, msg = op, append([]byte(nil), payload...)
		case opContinuation:
			msg = append(msg, payload...)
		default:
			return 0, nil, errors.New("an unknown WebSocket frame")
		}
		if len(msg) > maxBody {
			return 0, nil, &TooLargeError{}
		}
		if fin {
			return msgOp, msg, nil
		}
	}
}

func (w *wsConn) frame() (fin bool, op byte, payload []byte, err error) {
	var head [2]byte
	if _, err = io.ReadFull(w.br, head[:]); err != nil {
		return false, 0, nil, err
	}
	fin, op = head[0]&0x80 != 0, head[0]&0x0F
	masked := head[1]&0x80 != 0
	n := uint64(head[1] & 0x7F)
	switch n {
	case 126:
		var ext [2]byte
		if _, err = io.ReadFull(w.br, ext[:]); err != nil {
			return false, 0, nil, err
		}
		n = uint64(binary.BigEndian.Uint16(ext[:]))
	case 127:
		var ext [8]byte
		if _, err = io.ReadFull(w.br, ext[:]); err != nil {
			return false, 0, nil, err
		}
		n = binary.BigEndian.Uint64(ext[:])
	}
	if n > maxBody {
		return false, 0, nil, &TooLargeError{}
	}
	var mask [4]byte
	if masked {
		if _, err = io.ReadFull(w.br, mask[:]); err != nil {
			return false, 0, nil, err
		}
	}
	payload = make([]byte, n)
	if _, err = io.ReadFull(w.br, payload); err != nil {
		return false, 0, nil, err
	}
	if masked {
		for i := range payload {
			payload[i] ^= mask[i%4]
		}
	}
	return fin, op, payload, nil
}

// close sends a normal close (1000) and drops the connection; safe to call twice.
func (w *wsConn) close() {
	w.once.Do(func() {
		_ = w.write(opClose, []byte{0x03, 0xE8})
		_ = w.rwc.Close()
		if w.cancel != nil {
			w.cancel()
		}
	})
}

// drop ends the connection without a close frame (it is already broken or silent).
func (w *wsConn) drop() {
	w.once.Do(func() {
		_ = w.rwc.Close()
		if w.cancel != nil {
			w.cancel()
		}
	})
}
