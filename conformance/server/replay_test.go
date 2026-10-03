package main

import (
	"bytes"
	"encoding/json"
	"errors"
	"io"
	"net/http"
	"net/http/httptest"
	"net/url"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
	"time"
)

const casesDir = "../cases"

// start runs a server on a free loopback port for one test.
func start(t *testing.T) *httptest.Server {
	t.Helper()
	ts := httptest.NewServer(NewServer(casesDir))
	t.Cleanup(ts.Close)
	return ts
}

// client never reuses a connection, so a reset is never retried by net/http itself.
var client = &http.Client{
	Timeout:   10 * time.Second,
	Transport: &http.Transport{DisableKeepAlives: true},
}

func load(t *testing.T, ts *httptest.Server, body string) (int, map[string]any) {
	t.Helper()
	resp, err := client.Post(ts.URL+"/_case", "application/json", strings.NewReader(body))
	if err != nil {
		t.Fatal(err)
	}
	defer func() { _ = resp.Body.Close() }()
	var out map[string]any
	_ = json.NewDecoder(resp.Body).Decode(&out)
	return resp.StatusCode, out
}

func result(t *testing.T, ts *httptest.Server) Result {
	t.Helper()
	resp, err := client.Get(ts.URL + "/_result")
	if err != nil {
		t.Fatal(err)
	}
	defer func() { _ = resp.Body.Close() }()
	var res Result
	if err := json.NewDecoder(resp.Body).Decode(&res); err != nil {
		t.Fatal(err)
	}
	return res
}

// send makes the request an exchange describes, as an SDK would.
func send(t *testing.T, ts *httptest.Server, req Request) (*http.Response, []byte, error) {
	t.Helper()
	target := ts.URL + req.Path
	if len(req.Query) > 0 {
		q := url.Values{}
		for k, v := range req.Query {
			q.Set(k, v)
		}
		target += "?" + q.Encode()
	}
	var body io.Reader
	contentType := ""
	switch {
	case req.Form != nil:
		f := url.Values{}
		for k, v := range req.Form {
			f.Set(k, v)
		}
		body, contentType = strings.NewReader(f.Encode()), "application/x-www-form-urlencoded"
	case req.JSON != nil:
		data, _ := json.Marshal(req.JSON)
		body, contentType = bytes.NewReader(data), "application/json"
	case req.Method == http.MethodPost && req.Path == tokenPath:
		body, contentType = strings.NewReader("grant_type=client_credentials"), "application/x-www-form-urlencoded"
	}
	r, err := http.NewRequest(req.Method, target, body)
	if err != nil {
		t.Fatal(err)
	}
	if contentType != "" {
		r.Header.Set("content-type", contentType)
	}
	for k, v := range req.Headers {
		r.Header.Set(k, v)
	}
	resp, err := client.Do(r)
	if err != nil {
		return nil, nil, err
	}
	defer func() { _ = resp.Body.Close() }()
	data, err := io.ReadAll(resp.Body)
	return resp, data, err
}

// TestEveryCaseReplays plays each case's exchanges as the SDK side and checks the
// server answers what the case says and reports a pass with the right counts.
func TestEveryCaseReplays(t *testing.T) {
	files, err := filepath.Glob(filepath.Join(casesDir, "*", "*.yaml"))
	if err != nil || len(files) == 0 {
		t.Fatalf("no cases found under %s: %v", casesDir, err)
	}
	for _, file := range files {
		name := strings.TrimSuffix(filepath.Base(file), ".yaml")
		area := filepath.Base(filepath.Dir(file))
		t.Run(area+"/"+name, func(t *testing.T) {
			t.Parallel()
			ts := start(t)
			data, err := os.ReadFile(file)
			if err != nil {
				t.Fatal(err)
			}
			c, err := parseCase(data)
			if err != nil {
				t.Fatal(err)
			}
			if status, out := load(t, ts, `{"name": "`+area+"/"+name+`"}`); status != http.StatusOK {
				t.Fatalf("load: %d %v", status, out)
			}
			wantTokens, wantCalls := 0, 0
			last := time.Now()
			for i, ex := range c.Exchanges {
				if ex.Request.MinDelayMS != nil {
					time.Sleep(time.Until(last.Add(time.Duration(*ex.Request.MinDelayMS)*time.Millisecond + 20*time.Millisecond)))
				}
				if ex.Request.Path == tokenPath {
					wantTokens++
				} else {
					wantCalls++
				}
				resp, body, err := send(t, ts, ex.Request)
				last = time.Now()
				if ex.Response.Fault == "reset" {
					if err == nil {
						t.Fatalf("exchange %d: want a reset, got %d", i, resp.StatusCode)
					}
					continue
				}
				if err != nil {
					t.Fatalf("exchange %d: %v", i, err)
				}
				checkResponse(t, i, ex.Response, resp, body)
			}
			res := result(t, ts)
			if res.Status != "pass" || res.TokenExchanges != wantTokens || res.Attempts != wantCalls {
				t.Fatalf("result: %+v, want pass with %d token exchanges and %d attempts", res, wantTokens, wantCalls)
			}
		})
	}
}

func checkResponse(t *testing.T, i int, want Response, resp *http.Response, body []byte) {
	t.Helper()
	status := want.Status
	if status == 0 {
		status = http.StatusOK
	}
	if resp.StatusCode != status {
		t.Fatalf("exchange %d: status %d, want %d (%s)", i, resp.StatusCode, status, body)
	}
	for k, v := range want.Headers {
		if got := resp.Header.Get(k); got != v {
			t.Fatalf("exchange %d: header %s = %q, want %q", i, k, got, v)
		}
	}
	switch {
	case want.Text != nil:
		if string(body) != *want.Text {
			t.Fatalf("exchange %d: body %q, want %q", i, body, *want.Text)
		}
	case want.JSON != nil:
		var got any
		if err := json.Unmarshal(body, &got); err != nil || !reflect.DeepEqual(got, want.JSON) {
			t.Fatalf("exchange %d: body %s, want %v", i, body, want.JSON)
		}
	}
}

func TestWrongRequestFailsWithExpectedAndActual(t *testing.T) {
	ts := start(t)
	load(t, ts, `{"name": "token-is-cached"}`)
	resp, body, err := send(t, ts, Request{Method: "GET", Path: "/v1/me"})
	if err != nil {
		t.Fatal(err)
	}
	if resp.StatusCode != http.StatusBadRequest || !strings.Contains(string(body), "conformance_mismatch") {
		t.Fatalf("got %d %s", resp.StatusCode, body)
	}
	res := result(t, ts)
	if res.Status != "fail" || res.Mismatch == nil || res.Mismatch.Index != 0 ||
		res.Mismatch.Expected.Path != tokenPath || res.Mismatch.Actual.Path != "/v1/me" {
		t.Fatalf("result: %+v", res)
	}
	// A failed case stays failed: the right request afterwards does not repair it.
	if resp, _, _ := send(t, ts, Request{Method: "POST", Path: tokenPath}); resp.StatusCode != http.StatusBadRequest {
		t.Fatalf("after a failure: %d", resp.StatusCode)
	}
}

func TestSubsetMatchingOfFormHeadersAndJSON(t *testing.T) {
	ts := start(t)
	load(t, ts, `{"case": {"name": "inline", "area": "operations", "action": {"op": "me"}, "exchanges": [
		{"request": {"method": "POST", "path": "/v1/things", "headers": {"X-Trace": "abc"}, "json": {"a": {"b": [1, {"c": true}]}}},
		 "response": {"status": 201, "json": {"ok": true}}}]}}`)
	// Extra fields and headers are fine; the listed ones must match.
	resp, _, err := send(t, ts, Request{Method: "POST", Path: "/v1/things",
		Headers: map[string]string{"x-trace": "abc", "x-other": "1"},
		JSON:    map[string]any{"a": map[string]any{"b": []any{1, map[string]any{"c": true, "d": 2}}, "e": 3}}})
	if err != nil || resp.StatusCode != http.StatusCreated {
		t.Fatalf("%v %v", resp, err)
	}
	if res := result(t, ts); res.Status != "pass" {
		t.Fatalf("%+v", res)
	}

	load(t, ts, `{"name": "token-is-cached"}`)
	_, _, _ = send(t, ts, Request{Method: "POST", Path: tokenPath,
		Headers: map[string]string{"authorization": "Basic YWtfdGVzdDpzM2NyM3Q="},
		Form:    map[string]string{"grant_type": "client_credentials"}})
	res := result(t, ts)
	if res.Status != "fail" || !strings.Contains(res.Mismatch.Reason, "form field audience: missing") {
		t.Fatalf("%+v", res)
	}
}

func TestTooEarlyRequestFails(t *testing.T) {
	ts := start(t)
	load(t, ts, `{"name": "rate-limited-honours-retry-after"}`)
	c, _ := loadCase(casesDir, "rate-limited-honours-retry-after")
	for _, ex := range c.Exchanges { // the retry arrives at once, not after Retry-After
		_, _, _ = send(t, ts, ex.Request)
	}
	res := result(t, ts)
	if res.Status != "fail" || !strings.HasPrefix(res.Mismatch.Reason, "too early") {
		t.Fatalf("%+v", res)
	}
}

func TestIncompleteAndExtraRequests(t *testing.T) {
	ts := start(t)
	load(t, ts, `{"name": "internal-is-not-retried"}`)
	_, _, _ = send(t, ts, Request{Method: "POST", Path: tokenPath})
	if res := result(t, ts); res.Status != "incomplete" || res.Next == nil || res.Next.Path != "/v1/me" {
		t.Fatalf("%+v", res)
	}
	_, _, _ = send(t, ts, Request{Method: "GET", Path: "/v1/me"})
	_, _, _ = send(t, ts, Request{Method: "GET", Path: "/v1/me"}) // a retry the case forbids
	if res := result(t, ts); res.Status != "fail" || res.Mismatch.Reason != "the case has no exchanges left" || res.Attempts != 2 {
		t.Fatalf("%+v", res)
	}
}

func TestConcurrentWindowAcceptsAnyOrder(t *testing.T) {
	ts := start(t)
	load(t, ts, `{"case": {"name": "inline", "area": "auth", "action": {"op": "me", "concurrent": 2}, "exchanges": [
		{"request": {"method": "GET", "path": "/a"}, "response": {"status": 200}},
		{"request": {"method": "GET", "path": "/b"}, "response": {"status": 200}}]}}`)
	_, _, _ = send(t, ts, Request{Method: "GET", Path: "/b"})
	_, _, _ = send(t, ts, Request{Method: "GET", Path: "/a"})
	if res := result(t, ts); res.Status != "pass" {
		t.Fatalf("%+v", res)
	}
	// Without concurrency the order is strict.
	load(t, ts, `{"case": {"name": "inline", "area": "auth", "action": {"op": "me"}, "exchanges": [
		{"request": {"method": "GET", "path": "/a"}, "response": {"status": 200}},
		{"request": {"method": "GET", "path": "/b"}, "response": {"status": 200}}]}}`)
	_, _, _ = send(t, ts, Request{Method: "GET", Path: "/b"})
	if res := result(t, ts); res.Status != "fail" {
		t.Fatalf("%+v", res)
	}
}

func TestDelayAndChunkedBody(t *testing.T) {
	ts := start(t)
	load(t, ts, `{"case": {"name": "inline", "area": "operations", "action": {"op": "me"}, "exchanges": [
		{"request": {"method": "GET", "path": "/slow"},
		 "response": {"delay_ms": 150, "text": "0123456789", "chunked": {"bytes": 3, "delay_ms": 30}}}]}}`)
	began := time.Now()
	resp, body, err := send(t, ts, Request{Method: "GET", Path: "/slow"})
	took := time.Since(began)
	if err != nil {
		t.Fatal(err)
	}
	if string(body) != "0123456789" {
		t.Fatalf("body %q", body)
	}
	if len(resp.TransferEncoding) == 0 || resp.TransferEncoding[0] != "chunked" {
		t.Fatalf("transfer encoding %v, want chunked", resp.TransferEncoding)
	}
	if took < 150*time.Millisecond+3*30*time.Millisecond {
		t.Fatalf("answered in %v, want at least 240ms", took)
	}
}

func TestResetIsAConnectionError(t *testing.T) {
	ts := start(t)
	load(t, ts, `{"name": "reset-then-success"}`)
	_, _, _ = send(t, ts, Request{Method: "POST", Path: tokenPath})
	_, _, err := send(t, ts, Request{Method: "GET", Path: "/v1/me"})
	var urlErr *url.Error
	if err == nil || !errors.As(err, &urlErr) {
		t.Fatalf("want a connection error, got %v", err)
	}
}

func TestStreamingAreasAreNotImplemented(t *testing.T) {
	ts := start(t)
	status, out := load(t, ts, `{"case": {"name": "x", "area": "sse", "action": {"op": "me"},
		"exchanges": [{"request": {"method": "GET", "path": "/v1/ws"}}]}}`)
	if status != http.StatusNotImplemented || !strings.Contains(out["error"].(string), "not implemented") {
		t.Fatalf("%d %v", status, out)
	}
}

func TestBadLoads(t *testing.T) {
	ts := start(t)
	for _, body := range []string{`{"name": "../../etc/passwd"}`, `{"name": "no-such-case"}`, `not json`} {
		if status, _ := load(t, ts, body); status != http.StatusBadRequest {
			t.Fatalf("%s: %d", body, status)
		}
	}
	if res := result(t, ts); res.Status != "no_case" {
		t.Fatalf("%+v", res)
	}
	resp, _, _ := send(t, ts, Request{Method: "GET", Path: "/v1/me"})
	if resp.StatusCode != http.StatusBadRequest {
		t.Fatalf("request with no case: %d", resp.StatusCode)
	}
}

func TestTheCaseListAndTheLoadedCaseServeTheDrivers(t *testing.T) {
	ts := start(t)
	resp, err := client.Get(ts.URL + "/_cases")
	if err != nil {
		t.Fatal(err)
	}
	defer func() { _ = resp.Body.Close() }()
	var listed struct {
		Cases []string `json:"cases"`
	}
	if err := json.NewDecoder(resp.Body).Decode(&listed); err != nil {
		t.Fatal(err)
	}
	if len(listed.Cases) < 10 || listed.Cases[0] != "auth/concurrent-calls-share-one-exchange" {
		t.Fatalf("cases: %v", listed.Cases)
	}
	// Every listed case loads, and the answer carries what a driver reads.
	for _, name := range listed.Cases {
		status, out := load(t, ts, `{"name": "`+name+`"}`)
		if status != http.StatusOK {
			t.Fatalf("%s: %d %v", name, status, out)
		}
		c, ok := out["case"].(map[string]any)
		if !ok || c["expect"] == nil || c["action"] == nil {
			t.Fatalf("%s: the loaded case is not handed back: %v", name, out)
		}
	}
	_, out := load(t, ts, `{"name": "secret-never-in-message"}`)
	c := out["case"].(map[string]any)
	if c["client"].(map[string]any)["key_secret"] != "s3cr3t-do-not-print" {
		t.Fatalf("client options are not handed back: %v", c["client"])
	}
}
