package main

import (
	"bufio"
	"encoding/json"
	"fmt"
	"net"
	"net/http"
	"net/url"
	"reflect"
	"strings"
	"testing"
	"time"
)

// playSocket plays a socket answer's steps from the client's side: it sends what the
// server expects (with "c1" for a captured id) and reads what the server sends.
func playSocket(t *testing.T, ts *testServer, req Request, sock *Socket) {
	t.Helper()
	u, _ := url.Parse(ts.URL)
	conn, err := net.Dial("tcp", u.Host)
	if err != nil {
		t.Fatal(err)
	}
	defer func() { _ = conn.Close() }()
	_ = conn.SetDeadline(time.Now().Add(10 * time.Second))
	var hs strings.Builder
	fmt.Fprintf(&hs, "GET %s HTTP/1.1\r\nHost: %s\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n", req.Path, u.Host)
	hs.WriteString("Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n")
	for k, v := range req.Headers {
		fmt.Fprintf(&hs, "%s: %s\r\n", k, v)
	}
	hs.WriteString("\r\n")
	if _, err := conn.Write([]byte(hs.String())); err != nil {
		t.Fatal(err)
	}
	rw := bufio.NewReadWriter(bufio.NewReader(conn), bufio.NewWriter(conn))
	resp, err := http.ReadResponse(rw.Reader, nil)
	if err != nil || resp.StatusCode != http.StatusSwitchingProtocols {
		t.Fatalf("upgrade: %v %v", err, resp)
	}
	if got := resp.Header.Get("Sec-WebSocket-Accept"); got != "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=" {
		t.Fatalf("accept %q", got)
	}
	ids := map[string]string{}
	mask := []byte{1, 2, 3, 4}
	for i, step := range sock.Steps {
		switch {
		case step.Expect != nil:
			if step.As != "" {
				ids[step.As] = "c1"
			}
			frame := substitute(step.Expect, ids)
			if m, ok := frame.(map[string]any); ok && step.As != "" {
				m["id"] = "c1"
			}
			data, _ := json.Marshal(frame)
			if err := writeFrameMasked(rw.Writer, 0x1, data, mask); err != nil {
				t.Fatalf("step %d: %v", i, err)
			}
		case step.Send != nil:
			_, op, payload, err := readFrame(rw.Reader)
			if err != nil || op != 0x1 {
				t.Fatalf("step %d: %v op %d", i, err, op)
			}
			var got any
			_ = json.Unmarshal(payload, &got)
			if want := substitute(step.Send, ids); !reflect.DeepEqual(got, want) {
				t.Fatalf("step %d: got %v, want %v", i, got, want)
			}
		case step.Close != 0:
			_, op, _, err := readFrame(rw.Reader)
			if err != nil || op != 0x8 {
				t.Fatalf("step %d: want a close, got %v op %d", i, err, op)
			}
			return
		}
	}
	_ = writeFrameMasked(rw.Writer, 0x8, []byte{0x03, 0xE8}, mask)
	_, _, _, _ = readFrame(rw.Reader)
}

// checkSSE reads a whole SSE body and checks each data event is in it.
func checkSSE(t *testing.T, i int, want *SSE, resp *http.Response, body []byte) {
	t.Helper()
	if !strings.HasPrefix(resp.Header.Get("content-type"), "text/event-stream") {
		t.Fatalf("exchange %d: content-type %q", i, resp.Header.Get("content-type"))
	}
	for _, ev := range want.Events {
		if ev.Data == nil {
			continue
		}
		data, _ := json.Marshal(ev.Data)
		if !strings.Contains(string(body), "data: "+string(data)) {
			t.Fatalf("exchange %d: body lacks %s:\n%s", i, data, body)
		}
	}
}

func TestASocketStepThatDoesNotMatchFailsTheCase(t *testing.T) {
	ts := start(t)
	status, out := load(t, ts, `{"case": {"name": "x", "area": "socket", "action": {"op": "events.stream_events"},
		"exchanges": [{"request": {"method": "GET", "path": "/v1/ws"},
		  "response": {"socket": {"steps": [{"expect": {"type": "call", "method": "a/B"}, "as": "c"}]}}}]}}`)
	if status != http.StatusOK {
		t.Fatalf("%d %v", status, out)
	}
	playSocket(t, ts, Request{Method: "GET", Path: "/v1/ws"}, &Socket{Steps: []SocketStep{
		{Expect: map[string]any{"type": "call", "method": "other/C"}},
	}})
	deadline := time.Now().Add(5 * time.Second)
	for time.Now().Before(deadline) {
		if res := result(t, ts); res.Status == "fail" {
			if !strings.Contains(res.Mismatch.Reason, "socket step 0") {
				t.Fatalf("%+v", res.Mismatch)
			}
			return
		}
		time.Sleep(20 * time.Millisecond)
	}
	t.Fatal("the mismatch was never recorded")
}

func TestAnAnswerIsSSEOrASocketNotBoth(t *testing.T) {
	ts := start(t)
	status, _ := load(t, ts, `{"case": {"name": "x", "area": "sse", "action": {"op": "events.stream_events"},
		"exchanges": [{"request": {"method": "GET", "path": "/v1/ws"},
		  "response": {"sse": {"events": []}, "socket": {"steps": []}}}]}}`)
	if status != http.StatusBadRequest {
		t.Fatalf("%d", status)
	}
}
