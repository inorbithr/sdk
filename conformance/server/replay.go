package main

import (
	"bytes"
	"encoding/json"
	"fmt"
	"io"
	"maps"
	"net"
	"net/http"
	"net/url"
	"reflect"
	"slices"
	"strconv"
	"strings"
	"sync"
	"time"
)

const (
	// tokenPath is the token endpoint the SDKs call; it counts as a token exchange.
	tokenPath = "/oauth2/token"
	// maxBody bounds what the server reads from one request.
	maxBody = 1 << 20
)

// Server replays one case at a time: POST /_case loads it, every other request must be
// the case's next exchange, GET /_result reports the outcome.
type Server struct {
	casesDir string
	now      func() time.Time
	sleep    func(time.Duration)

	mu      sync.Mutex
	current *Case
	used    []bool
	next    int // the first exchange not yet used
	window  int // how many unused exchanges, from next on, a request may match
	last    time.Time
	fail    *Mismatch
	tokens  int
	calls   int
	session int // bumped by every load, so a socket left from an earlier case fails nothing
}

// NewServer replays cases read from casesDir.
func NewServer(casesDir string) *Server {
	return &Server{casesDir: casesDir, now: time.Now, sleep: time.Sleep}
}

// Mismatch is the first request that broke the case.
type Mismatch struct {
	Index    int      `json:"index"`
	Reason   string   `json:"reason"`
	Expected *Request `json:"expected,omitempty"`
	Actual   Seen     `json:"actual"`
}

// Seen is a request as it arrived.
type Seen struct {
	Method  string            `json:"method"`
	Path    string            `json:"path"`
	Query   map[string]string `json:"query,omitempty"`
	Headers map[string]string `json:"headers,omitempty"`
	Form    map[string]string `json:"form,omitempty"`
	JSON    any               `json:"json,omitempty"`
	Body    string            `json:"body,omitempty"`
}

// Result is what GET /_result answers.
type Result struct {
	Status         string    `json:"status"` // pass, fail, incomplete or no_case
	Case           string    `json:"case,omitempty"`
	Used           int       `json:"used"`
	Total          int       `json:"total"`
	TokenExchanges int       `json:"token_exchanges"`
	Attempts       int       `json:"attempts"`
	Mismatch       *Mismatch `json:"mismatch,omitempty"`
	Next           *Request  `json:"next,omitempty"`
}

func (s *Server) ServeHTTP(w http.ResponseWriter, r *http.Request) {
	switch r.URL.Path {
	case "/_case":
		s.loadHandler(w, r)
	case "/_result":
		s.resultHandler(w, r)
	case "/_cases":
		s.listHandler(w, r)
	default:
		if strings.HasPrefix(r.URL.Path, "/_") {
			writeJSON(w, http.StatusNotFound, problem("not_found", "no such control route: "+r.URL.Path))
			return
		}
		s.replay(w, r)
	}
}

// loadHandler takes {"name": "auth/token-is-cached"} or {"case": {...}} and starts a
// session; whatever was loaded before is dropped.
func (s *Server) loadHandler(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		writeJSON(w, http.StatusMethodNotAllowed, problem("method_not_allowed", "POST a case"))
		return
	}
	var body struct {
		Name string          `json:"name"`
		Case json.RawMessage `json:"case"`
	}
	if err := json.NewDecoder(io.LimitReader(r.Body, maxBody)).Decode(&body); err != nil {
		writeJSON(w, http.StatusBadRequest, problem("bad_request", "the body is {\"name\": ...} or {\"case\": {...}}"))
		return
	}
	var (
		c   *Case
		err error
	)
	if len(body.Case) > 0 {
		c, err = parseCase(body.Case)
	} else {
		c, err = loadCase(s.casesDir, body.Name)
	}
	if err != nil {
		writeJSON(w, http.StatusBadRequest, problem("bad_request", err.Error()))
		return
	}

	s.mu.Lock()
	defer s.mu.Unlock()
	s.current = c
	s.used = make([]bool, len(c.Exchanges))
	s.next = 0
	s.window = max(1, c.Action.Concurrent)
	s.last = s.now()
	s.fail = nil
	s.tokens, s.calls = 0, 0
	s.session++
	// The loaded case goes back as JSON, so a driver reads client, action and expect
	// from here instead of parsing YAML itself.
	writeJSON(w, http.StatusOK, map[string]any{"loaded": c.Name, "exchanges": len(c.Exchanges), "case": c})
}

// listHandler answers GET /_cases with every case name under the cases directory.
func (s *Server) listHandler(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodGet {
		writeJSON(w, http.StatusMethodNotAllowed, problem("method_not_allowed", "GET the case list"))
		return
	}
	names, err := listCases(s.casesDir)
	if err != nil {
		writeJSON(w, http.StatusInternalServerError, problem("internal", err.Error()))
		return
	}
	writeJSON(w, http.StatusOK, map[string]any{"cases": names})
}

func (s *Server) resultHandler(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodGet {
		writeJSON(w, http.StatusMethodNotAllowed, problem("method_not_allowed", "GET the result"))
		return
	}
	writeJSON(w, http.StatusOK, s.Result())
}

// Result reports the session so far.
func (s *Server) Result() Result {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.current == nil {
		return Result{Status: "no_case"}
	}
	res := Result{
		Case: s.current.Name, Total: len(s.used),
		TokenExchanges: s.tokens, Attempts: s.calls, Mismatch: s.fail,
	}
	for _, u := range s.used {
		if u {
			res.Used++
		}
	}
	switch {
	case s.fail != nil:
		res.Status = "fail"
	case res.Used < res.Total:
		res.Status = "incomplete"
		next := s.current.Exchanges[s.next].Request
		res.Next = &next
	default:
		res.Status = "pass"
	}
	return res
}

// replay matches one SDK request against the case and sends the exchange's response.
func (s *Server) replay(w http.ResponseWriter, r *http.Request) {
	seen, err := read(r)
	if err != nil {
		writeJSON(w, http.StatusBadRequest, problem("bad_request", err.Error()))
		return
	}
	resp, ok := s.match(seen)
	if !ok {
		// A status no SDK retries, so a mismatch ends the case at once.
		writeJSON(w, http.StatusBadRequest, problem("conformance_mismatch",
			"this request does not match the case; GET /_result for the expected one"))
		return
	}
	s.mu.Lock()
	session := s.session
	s.mu.Unlock()
	switch {
	case resp.Socket != nil:
		s.respondSocket(w, r, resp)
	case resp.SSE != nil:
		s.respondSSE(w, resp.Status, resp)
	default:
		s.respond(w, resp)
	}
	// A stream or a socket can outlive its case; only an answer of the case still
	// loaded moves the clock its next request's delay is measured from.
	s.mu.Lock()
	if s.session == session {
		s.last = s.now()
	}
	s.mu.Unlock()
}

// match finds the exchange a request is, marks it used and returns its response.
func (s *Server) match(seen Seen) (Response, bool) {
	s.mu.Lock()
	defer s.mu.Unlock()
	if seen.Path == tokenPath {
		s.tokens++
	} else {
		s.calls++
	}
	if s.current == nil {
		s.fail = &Mismatch{Index: -1, Reason: "no case is loaded (POST /_case first)", Actual: seen}
		return Response{}, false
	}
	if s.fail != nil {
		return Response{}, false
	}
	if s.next >= len(s.used) {
		s.fail = &Mismatch{Index: len(s.used), Reason: "the case has no exchanges left", Actual: seen}
		return Response{}, false
	}

	elapsed := s.now().Sub(s.last)
	firstReason := ""
	for i, tried := s.next, 0; i < len(s.used) && tried < s.window; i++ {
		if s.used[i] {
			continue
		}
		tried++
		want := s.current.Exchanges[i].Request
		reason := mismatch(want, seen)
		if reason == "" {
			reason = timing(want, elapsed)
		}
		if reason == "" {
			s.used[i] = true
			for s.next < len(s.used) && s.used[s.next] {
				s.next++
			}
			return s.current.Exchanges[i].Response, true
		}
		if firstReason == "" {
			firstReason = reason
		}
	}
	want := s.current.Exchanges[s.next].Request
	s.fail = &Mismatch{Index: s.next, Reason: firstReason, Expected: &want, Actual: seen}
	return Response{}, false
}

// respond writes a response as the case gives it.
func (s *Server) respond(w http.ResponseWriter, resp Response) {
	if resp.DelayMS > 0 {
		s.sleep(time.Duration(resp.DelayMS) * time.Millisecond)
	}
	if resp.Fault == "reset" {
		reset(w)
		return
	}
	body, contentType := encodeBody(resp)
	for k, v := range resp.Headers {
		w.Header().Set(k, v)
	}
	if w.Header().Get("content-type") == "" && contentType != "" {
		w.Header().Set("content-type", contentType)
	}
	status := resp.Status
	if status == 0 {
		status = http.StatusOK
	}
	if resp.Chunked == nil {
		w.Header().Set("content-length", strconv.Itoa(len(body)))
		w.WriteHeader(status)
		_, _ = w.Write(body)
		return
	}
	// No content-length: net/http sends the body with chunked transfer encoding.
	w.WriteHeader(status)
	flusher, _ := w.(http.Flusher)
	for start := 0; start < len(body); start += resp.Chunked.Bytes {
		end := min(start+resp.Chunked.Bytes, len(body))
		if _, err := w.Write(body[start:end]); err != nil {
			return
		}
		if flusher != nil {
			flusher.Flush()
		}
		if end < len(body) && resp.Chunked.DelayMS > 0 {
			s.sleep(time.Duration(resp.Chunked.DelayMS) * time.Millisecond)
		}
	}
}

func encodeBody(resp Response) ([]byte, string) {
	switch {
	case resp.Text != nil:
		return []byte(*resp.Text), "text/plain; charset=utf-8"
	case resp.JSON != nil:
		data, err := json.Marshal(resp.JSON)
		if err != nil {
			return nil, ""
		}
		return data, "application/json"
	default:
		return nil, ""
	}
}

// reset closes the connection with a TCP reset instead of answering.
func reset(w http.ResponseWriter) {
	hj, ok := w.(http.Hijacker)
	if !ok {
		panic(http.ErrAbortHandler)
	}
	conn, _, err := hj.Hijack()
	if err != nil {
		panic(http.ErrAbortHandler)
	}
	if tcp, ok := conn.(*net.TCPConn); ok {
		_ = tcp.SetLinger(0)
	}
	_ = conn.Close()
}

// read captures a request in the shape cases describe: lower-case header names, the
// first value of each query parameter, a form or JSON body.
func read(r *http.Request) (Seen, error) {
	body, err := io.ReadAll(io.LimitReader(r.Body, maxBody+1))
	if err != nil {
		return Seen{}, fmt.Errorf("reading the body: %w", err)
	}
	if len(body) > maxBody {
		return Seen{}, fmt.Errorf("the body is larger than %d bytes", maxBody)
	}
	seen := Seen{Method: r.Method, Path: r.URL.EscapedPath(), Query: first(r.URL.Query()), Headers: map[string]string{}}
	for k := range r.Header {
		seen.Headers[strings.ToLower(k)] = r.Header.Get(k)
	}
	if host := r.Host; host != "" {
		seen.Headers["host"] = host
	}
	if len(body) == 0 {
		return seen, nil
	}
	mediaType := strings.ToLower(strings.TrimSpace(strings.Split(r.Header.Get("content-type"), ";")[0]))
	switch mediaType {
	case "application/x-www-form-urlencoded":
		form, err := url.ParseQuery(string(body))
		if err != nil {
			return Seen{}, fmt.Errorf("the form body does not parse: %w", err)
		}
		seen.Form = first(form)
	case "application/json":
		if err := json.Unmarshal(body, &seen.JSON); err != nil {
			seen.Body = string(body)
		}
	default:
		seen.Body = string(body)
	}
	return seen, nil
}

func first(values map[string][]string) map[string]string {
	if len(values) == 0 {
		return nil
	}
	out := make(map[string]string, len(values))
	for k, v := range values {
		if len(v) > 0 {
			out[k] = v[0]
		}
	}
	return out
}

// mismatch says why a request is not the expected one, or "" when it is.
func mismatch(want Request, got Seen) string {
	if !strings.EqualFold(want.Method, got.Method) {
		return fmt.Sprintf("method: want %s, got %s", want.Method, got.Method)
	}
	if want.Path != got.Path {
		return fmt.Sprintf("path: want %s, got %s", want.Path, got.Path)
	}
	if reason := subsetMap("query", want.Query, got.Query); reason != "" {
		return reason
	}
	lower := make(map[string]string, len(want.Headers))
	for k, v := range want.Headers {
		lower[strings.ToLower(k)] = v
	}
	if reason := subsetMap("header", lower, got.Headers); reason != "" {
		return reason
	}
	if reason := subsetMap("form field", want.Form, got.Form); reason != "" {
		return reason
	}
	if want.JSON != nil {
		if got.JSON == nil {
			return "body: want JSON, got none or not JSON"
		}
		if path, ok := subset(want.JSON, got.JSON, "$"); !ok {
			return "body: " + path + " differs"
		}
	}
	if len(want.Absent) > 0 {
		body, ok := got.JSON.(map[string]any)
		if !ok {
			return "body: want a JSON object, got none or another value"
		}
		for _, k := range slices.Sorted(slices.Values(want.Absent)) {
			if _, sent := body[k]; sent {
				return "body: $." + k + " was sent; an unset field is left out"
			}
		}
	}
	return ""
}

// subsetMap checks the keys in sorted order, so the reason names the same missing or
// different key on every run (Go's map order is random).
func subsetMap(what string, want, got map[string]string) string {
	for _, k := range slices.Sorted(maps.Keys(want)) {
		v := want[k]
		actual, ok := got[k]
		if !ok {
			return fmt.Sprintf("%s %s: missing", what, k)
		}
		if actual != v {
			return fmt.Sprintf("%s %s: want %q, got %q", what, k, v, actual)
		}
	}
	return ""
}

// subset reports whether want is contained in got: objects by key, arrays element by
// element with equal length, scalars by value. On a difference it returns the path.
func subset(want, got any, path string) (string, bool) {
	switch w := want.(type) {
	case map[string]any:
		g, ok := got.(map[string]any)
		if !ok {
			return path, false
		}
		for _, k := range slices.Sorted(maps.Keys(w)) {
			v := w[k]
			gv, present := g[k]
			if !present {
				return path + "." + k, false
			}
			if p, ok := subset(v, gv, path+"."+k); !ok {
				return p, false
			}
		}
		return "", true
	case []any:
		g, ok := got.([]any)
		if !ok || len(g) != len(w) {
			return path, false
		}
		for i := range w {
			if p, ok := subset(w[i], g[i], fmt.Sprintf("%s[%d]", path, i)); !ok {
				return p, false
			}
		}
		return "", true
	default:
		if !reflect.DeepEqual(want, got) {
			return path, false
		}
		return "", true
	}
}

// timing checks a request's delay since the previous answer against the case's bounds.
func timing(want Request, elapsed time.Duration) string {
	ms := int(elapsed / time.Millisecond)
	if want.MinDelayMS != nil && ms < *want.MinDelayMS {
		return fmt.Sprintf("too early: %d ms after the previous answer, want at least %d", ms, *want.MinDelayMS)
	}
	if want.MaxDelayMS != nil && ms > *want.MaxDelayMS {
		return fmt.Sprintf("too late: %d ms after the previous answer, want at most %d", ms, *want.MaxDelayMS)
	}
	return ""
}

func problem(code, message string) map[string]any {
	return map[string]any{"code": code, "error": message, "details": []any{}}
}

func writeJSON(w http.ResponseWriter, status int, v any) {
	var buf bytes.Buffer
	enc := json.NewEncoder(&buf)
	enc.SetIndent("", "  ")
	_ = enc.Encode(v)
	w.Header().Set("content-type", "application/json")
	w.Header().Set("content-length", strconv.Itoa(buf.Len()))
	w.WriteHeader(status)
	_, _ = w.Write(buf.Bytes())
}
