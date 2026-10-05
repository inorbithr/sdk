package main

import (
	"bufio"
	"bytes"
	"crypto/x509"
	"encoding/json"
	"encoding/pem"
	"fmt"
	"net"
	"net/http"
	"net/url"
	"os"
	"strings"
	"testing"
	"time"
)

const matcherCase = `{"case": {"name": "inline", "area": "middleware", "action": {"op": "me"}, "exchanges": [
	{"request": {"method": "GET", "path": "/a", "headers": {"x-any": "*", "x-id": "$rid", "x-re": "~v[0-9]{2}"}, "headers_absent": ["x-never"]},
	 "response": {"status": 200}},
	{"request": {"method": "GET", "path": "/a", "headers": {"x-id": "$rid"}}, "response": {"status": 200}}]}}`

func TestHeaderMatchers(t *testing.T) {
	ts := start(t)
	ok := map[string]string{"x-any": "whatever", "x-id": "r-1", "x-re": "v42"}
	for _, tc := range []struct {
		name          string
		first, second map[string]string
		reason        string
	}{
		{"satisfied", ok, map[string]string{"x-id": "r-1"}, ""},
		{"a capture must repeat", ok, map[string]string{"x-id": "r-2"}, `the value captured as $rid`},
		{"star needs the header", map[string]string{"x-id": "r-1", "x-re": "v42"}, nil, "header x-any: missing"},
		{"the regex matches the whole value", map[string]string{"x-any": "1", "x-id": "r-1", "x-re": "v421"}, nil, "does not match"},
		{"an absent header is not sent", map[string]string{"x-any": "1", "x-id": "r-1", "x-re": "v42", "x-never": "1"}, nil, "header x-never: sent"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			load(t, ts, matcherCase)
			_, _, _ = send(t, ts, Request{Method: "GET", Path: "/a", Headers: tc.first})
			if tc.second != nil {
				_, _, _ = send(t, ts, Request{Method: "GET", Path: "/a", Headers: tc.second})
			}
			res := result(t, ts)
			if tc.reason == "" {
				if res.Status != "pass" {
					t.Fatalf("%+v", res)
				}
				return
			}
			if res.Status != "fail" || !strings.Contains(res.Mismatch.Reason, tc.reason) {
				t.Fatalf("want a failure with %q, got %+v", tc.reason, res)
			}
		})
	}
}

func TestABadMatcherDoesNotLoad(t *testing.T) {
	ts := start(t)
	for _, req := range []string{
		`{"method": "GET", "path": "/a", "headers": {"x": "~("}}`,
		`{"method": "GET", "path": "/a", "headers": {"x": "$"}}`,
		`{"method": "GET", "path": "/a", "via": "sideways"}`,
	} {
		status, _ := load(t, ts, `{"case": {"name": "x", "area": "middleware", "action": {"op": "me"}, "exchanges": [{"request": `+req+`, "response": {}}]}}`)
		if status != http.StatusBadRequest {
			t.Fatalf("%s loaded: %d", req, status)
		}
	}
}

func TestTheLoadAnswerNamesTheListenersAndFiles(t *testing.T) {
	ts := start(t)
	_, out := load(t, ts, `{"name": "transport/a-client-certificate-is-presented"}`)
	e := ts.Endpoints
	for k, want := range map[string]string{
		"base_url": e.MTLSURL, "http_url": e.HTTPURL, "https_url": e.HTTPSURL, "mtls_url": e.MTLSURL,
		"proxy_url": e.ProxyURL, "ca_file": e.CAFile, "client_cert_file": e.ClientCertFile, "client_key_file": e.ClientKeyFile,
	} {
		if out[k] != want || want == "" {
			t.Fatalf("%s = %v, want %q", k, out[k], want)
		}
	}
	// The leaf verifies against ca.pem for every loopback name.
	pool := x509.NewCertPool()
	data, err := os.ReadFile(e.CAFile)
	if err != nil || !pool.AppendCertsFromPEM(data) {
		t.Fatalf("ca.pem: %v", err)
	}
	for _, name := range []string{"127.0.0.1", "::1", "localhost"} {
		if _, err := ts.PKI.server.Leaf.Verify(x509.VerifyOptions{Roots: pool, DNSName: name}); err != nil {
			t.Fatalf("%s: %v", name, err)
		}
	}
	// The client key is a PKCS#8 PEM only its owner reads.
	info, err := os.Stat(e.ClientKeyFile)
	if err != nil || info.Mode().Perm() != 0o600 {
		t.Fatalf("client key: %v %v", info, err)
	}
	key, _ := os.ReadFile(e.ClientKeyFile)
	if block, _ := pem.Decode(key); block == nil || block.Type != "PRIVATE KEY" {
		t.Fatal("the client key is not a PKCS#8 PEM")
	}
}

func TestViaIsMatched(t *testing.T) {
	ts := start(t)
	load(t, ts, `{"name": "transport/an-explicit-proxy-carries-the-call"}`)
	_, _, _ = sendOver(t, ts, "https", Request{Method: "POST", Path: tokenPath})
	if res := result(t, ts); res.Status != "fail" || res.Mismatch.Reason != "via: want proxy, got direct" {
		t.Fatalf("%+v", res)
	}
	load(t, ts, `{"name": "transport/no-proxy-sends-the-call-direct"}`)
	_, _, _ = sendOver(t, ts, "https", Request{Method: "POST", Path: tokenPath, Via: "proxy"})
	if res := result(t, ts); res.Status != "fail" || res.Mismatch.Reason != "via: want direct, got proxy" {
		t.Fatalf("%+v", res)
	}
}

func TestTheProxyCarriesProxyAuthorizationAndPlainRequests(t *testing.T) {
	ts := start(t)
	inline := `{"case": {"name": "x", "area": "transport", "action": {"op": "me"}, "exchanges": [
		{"request": {"method": "GET", "path": "/a", "via": "proxy", "headers": {"proxy-authorization": "Basic dXNlcjpwYXNz"}}, "response": {"status": 204}}]}}`
	proxy, _ := url.Parse(ts.Endpoints.ProxyURL)
	proxy.User = url.UserPassword("user", "pass")
	cfg, _ := ts.PKI.clientTLS(false)
	for _, base := range []string{ts.Endpoints.HTTPSURL, ts.Endpoints.HTTPURL} { // CONNECT, then absolute-form
		load(t, ts, inline)
		c := &http.Client{Timeout: 5 * time.Second, Transport: &http.Transport{
			Proxy: http.ProxyURL(proxy), TLSClientConfig: cfg, DisableKeepAlives: true}}
		resp, err := c.Get(base + "/a")
		if err != nil {
			t.Fatal(err)
		}
		_ = resp.Body.Close()
		if res := result(t, ts); res.Status != "pass" {
			t.Fatalf("%s: %+v", base, res)
		}
	}
}

func TestTheProxyReachesOnlyTheReplay(t *testing.T) {
	ts := start(t)
	u, _ := url.Parse(ts.Endpoints.ProxyURL)
	for _, target := range []string{"example.com:443", "127.0.0.1:1"} {
		conn, err := net.Dial("tcp", u.Host)
		if err != nil {
			t.Fatal(err)
		}
		_, _ = fmt.Fprintf(conn, "CONNECT %s HTTP/1.1\r\nHost: %s\r\n\r\n", target, target)
		resp, err := http.ReadResponse(bufio.NewReader(conn), nil)
		_ = conn.Close()
		if err != nil || resp.StatusCode != http.StatusForbidden {
			t.Fatalf("CONNECT %s: %v %v", target, resp, err)
		}
	}
}

func TestMTLSRequiresAndReportsTheClientCertificate(t *testing.T) {
	ts := start(t)
	// Without a certificate the handshake fails; the case sees nothing.
	cfg, _ := ts.PKI.clientTLS(false)
	c := &http.Client{Timeout: 5 * time.Second, Transport: &http.Transport{TLSClientConfig: cfg, DisableKeepAlives: true}}
	load(t, ts, `{"name": "transport/a-client-certificate-is-presented"}`)
	if resp, err := c.Post(ts.Endpoints.MTLSURL+tokenPath, "application/x-www-form-urlencoded", strings.NewReader("")); err == nil {
		_ = resp.Body.Close()
		t.Fatalf("mTLS without a certificate answered %d", resp.StatusCode)
	}
	if res := result(t, ts); res.Status != "incomplete" {
		t.Fatalf("%+v", res)
	}
	// Another subject is a mismatch; the bare common name matches too.
	load(t, ts, `{"case": {"name": "x", "area": "transport", "action": {"op": "me"}, "exchanges": [
		{"request": {"method": "GET", "path": "/a", "client_cert": "CN=someone-else"}, "response": {}}]}}`)
	_, _, _ = sendOver(t, ts, "mtls", Request{Method: "GET", Path: "/a"})
	if res := result(t, ts); res.Status != "fail" || res.Mismatch.Reason != "client_cert: want CN=someone-else, got CN=conformance-client" {
		t.Fatalf("%+v", res)
	}
	load(t, ts, `{"case": {"name": "x", "area": "transport", "action": {"op": "me"}, "exchanges": [
		{"request": {"method": "GET", "path": "/a", "client_cert": "conformance-client"}, "response": {}}]}}`)
	_, _, _ = sendOver(t, ts, "mtls", Request{Method: "GET", Path: "/a"})
	if res := result(t, ts); res.Status != "pass" {
		t.Fatalf("%+v", res)
	}
	// Over plain HTTP no certificate is presented.
	load(t, ts, `{"name": "transport/a-client-certificate-is-presented"}`)
	_, _, _ = send(t, ts, Request{Method: "POST", Path: tokenPath})
	if res := result(t, ts); res.Status != "fail" || !strings.Contains(res.Mismatch.Reason, "got no client certificate") {
		t.Fatalf("%+v", res)
	}
}

func TestTheFakeIohr(t *testing.T) {
	now := time.Date(2026, 10, 4, 10, 0, 0, 500, time.FixedZone("CEST", 2*3600))
	var out, errOut bytes.Buffer
	args := []string{"auth", "token", "--profile", "dev", "--format", "json"}
	if !isIohr(args) || isIohr([]string{"--addr", "x"}) {
		t.Fatal("isIohr")
	}
	if code := fakeIohr(args, now, &out, &errOut); code != 0 || errOut.Len() != 0 {
		t.Fatalf("exit %d: %s", code, errOut.String())
	}
	var got map[string]string
	if err := json.Unmarshal(out.Bytes(), &got); err != nil {
		t.Fatal(err)
	}
	want := map[string]string{"access_token": "cli-dev", "expires_at": "2026-10-04T08:15:00Z", "profile": "dev"}
	if fmt.Sprint(got) != fmt.Sprint(want) {
		t.Fatalf("got %v, want %v", got, want)
	}

	out.Reset()
	errOut.Reset()
	if code := fakeIohr([]string{"auth", "token", "--profile", "missing", "--format", "json"}, now, &out, &errOut); code != 1 ||
		out.Len() != 0 || strings.Count(errOut.String(), "\n") != 1 {
		t.Fatalf("missing: exit %d, stdout %q, stderr %q", code, out.String(), errOut.String())
	}
	for _, bad := range [][]string{{"auth", "token"}, {"auth", "token", "--profile", "dev"}, {"auth", "token", "--profile", "dev", "--format", "text"}} {
		if code := fakeIohr(bad, now, &out, &errOut); code != 2 {
			t.Fatalf("%v: exit %d", bad, code)
		}
	}
}
