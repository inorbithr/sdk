package inorbit

import (
	"bufio"
	"context"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"
)

func parseAll(t *testing.T, body string) ([]string, error) {
	t.Helper()
	p := &sseParser{r: bufio.NewReader(strings.NewReader(body))}
	var out []string
	for {
		name, data, err := p.next()
		if errors.Is(err, io.EOF) {
			return out, nil
		}
		if err != nil {
			return out, err
		}
		out = append(out, name+"|"+string(data))
	}
}

func TestTheParserFollowsTheEventStreamRules(t *testing.T) {
	got, err := parseAll(t, ": open\n\ndata: a\ndata:b\n\nevent: error\ndata: {}\r\n\r\nid: 7\rretry: 1\rdata: c\r\rfoo\n\ndata: unterminated")
	if err != nil {
		t.Fatal(err)
	}
	want := []string{"|a\nb", "error|{}", "|c"}
	if fmt.Sprint(got) != fmt.Sprint(want) {
		t.Fatalf("got %q, want %q", got, want)
	}
}

func TestAnEventOverOneMiBIsRefused(t *testing.T) {
	_, err := parseAll(t, "data: "+strings.Repeat("x", maxEvent+1)+"\n\n")
	var tooLarge *TooLargeError
	if !errors.As(err, &tooLarge) || !tooLarge.Event {
		t.Fatalf("want an event too large, got %v", err)
	}
}

func sseServer(t *testing.T, handler func(w http.ResponseWriter, f http.Flusher)) *Client {
	t.Helper()
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.Header().Set("Content-Type", "text/event-stream")
		f, _ := w.(http.Flusher)
		handler(w, f)
	}))
	t.Cleanup(srv.Close)
	c, err := NewClient(WithToken("t"), WithBaseURL(srv.URL), WithStreamIdleTimeout(200*time.Millisecond), WithMaxRetries(0))
	if err != nil {
		t.Fatal(err)
	}
	return c
}

type event struct {
	ID string `json:"id"`
}

func TestAStreamYieldsEventsAndSilenceIsATimeout(t *testing.T) {
	c := sseServer(t, func(w http.ResponseWriter, f http.Flusher) {
		_, _ = io.WriteString(w, "data: {\"id\":\"e1\"}\n\n")
		f.Flush()
		time.Sleep(time.Second)
	})
	var ids []string
	var last error
	for ev, err := range Stream[event](context.Background(), c, Operation{Method: "GET", Path: "/v1/events/events"}) {
		if err != nil {
			last = err
			break
		}
		ids = append(ids, ev.ID)
	}
	var timeout *TimeoutError
	if fmt.Sprint(ids) != "[e1]" || !errors.As(last, &timeout) {
		t.Fatalf("ids %v, err %v", ids, last)
	}
}

func TestAStreamEndsWhenCtxEnds(t *testing.T) {
	c := sseServer(t, func(w http.ResponseWriter, f http.Flusher) {
		for range 20 {
			_, _ = io.WriteString(w, ": keep-alive\n\n")
			f.Flush()
			time.Sleep(50 * time.Millisecond)
		}
	})
	ctx, cancel := context.WithTimeout(context.Background(), 150*time.Millisecond)
	defer cancel()
	var last error
	for _, err := range Stream[event](ctx, c, Operation{Method: "GET", Path: "/v1/events/events"}) {
		last = err
	}
	if !errors.Is(last, context.DeadlineExceeded) {
		t.Fatalf("want the context's error, got %v", last)
	}
}

func TestStreamsOverTheSocketNeedAnRPC(t *testing.T) {
	c, err := NewClient(WithToken("t"), WithStreams(StreamsSocket))
	if err != nil {
		t.Fatal(err)
	}
	for _, err := range Stream[event](context.Background(), c, Operation{Method: "GET", Path: "/v1/x/events"}) {
		var cfg *ConfigError
		if !errors.As(err, &cfg) {
			t.Fatalf("want a config error, got %v", err)
		}
	}
	if _, err := NewClient(WithToken("t"), WithStreams("carrier-pigeon")); err == nil {
		t.Fatal("an unknown streams value was accepted")
	}
}

func TestAnErrorEventCarriesTheCodesStatus(t *testing.T) {
	e := streamProblem([]byte(`{"code":"unauthenticated","error":"revoked","details":[]}`))
	if e.Status != 401 || e.Code != CodeUnauthenticated || e.Problem != "revoked" {
		t.Fatalf("%+v", e)
	}
	if e := streamProblem([]byte(`{"code":"brand_new","error":"x"}`)); e.Status != 0 || e.Code != "brand_new" {
		t.Fatalf("an unknown code: %+v", e)
	}
}

// A masked frame over 64 KiB (the 8-byte length) and the next frame read back whole.
func TestFramesRoundTripMaskedAndFragmented(t *testing.T) {
	a, b := pipeConns()
	ca, cb := newWSConn(a, nil), newWSConn(b, nil)
	big := strings.Repeat("y", 70000)
	go func() {
		_ = ca.write(opText, []byte(big))
		_ = ca.write(opText, []byte(`{"type":"end","id":"1"}`))
	}()
	for _, want := range []string{big, `{"type":"end","id":"1"}`} {
		op, data, err := cb.read()
		if err != nil || op != opText || string(data) != want {
			t.Fatalf("op %d err %v len %d", op, err, len(data))
		}
	}
}

type pipeConn struct {
	io.Reader
	io.Writer
}

func (pipeConn) Close() error { return nil }

func pipeConns() (io.ReadWriteCloser, io.ReadWriteCloser) {
	ar, bw := io.Pipe()
	br, aw := io.Pipe()
	return pipeConn{ar, aw}, pipeConn{br, bw}
}
