// Package conformance is the driver for conformance/cases: it starts the replay
// server, loads every case, runs its action through the generated public surface, and
// compares the result and the server's verdict with expect (conformance/README.md).
// Without the server binary (mise run conformance:server:build) it skips, unless
// IOHR_TEST_REQUIRE_REPLAY is set.
package conformance

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"reflect"
	"runtime"
	"strings"
	"sync"
	"testing"
	"time"

	inorbit "github.com/inorbithr/sdk/go"
	"github.com/inorbithr/sdk/go/public"
	"github.com/inorbithr/sdk/go/public/models"
)

type testCase struct {
	Name    string   `json:"name"`
	Area    string   `json:"area"`
	Pending []string `json:"pending"`
	Action  struct {
		Op         string         `json:"op"`
		Args       map[string]any `json:"args"`
		Repeat     int            `json:"repeat"`
		Concurrent int            `json:"concurrent"`
	} `json:"action"`
	Client struct {
		MaxRetries *int     `json:"max_retries"`
		TimeoutMS  int      `json:"timeout_ms"`
		KeyID      string   `json:"key_id"`
		KeySecret  string   `json:"key_secret"`
		Scopes     []string `json:"scopes"`
	} `json:"client"`
	Expect struct {
		OK    any `json:"ok"`
		Error *struct {
			Kind            string `json:"kind"`
			Code            string `json:"code"`
			Status          int    `json:"status"`
			MessageContains string `json:"message_contains"`
			MessageExcludes string `json:"message_excludes"`
		} `json:"error"`
		Attempts       *int `json:"attempts"`
		TokenExchanges *int `json:"token_exchanges"`
	} `json:"expect"`
}

type verdict struct {
	Status         string `json:"status"`
	TokenExchanges int    `json:"token_exchanges"`
	Attempts       int    `json:"attempts"`
	Mismatch       any    `json:"mismatch"`
	Next           any    `json:"next"`
}

func startReplay(t *testing.T) string {
	t.Helper()
	root, _ := filepath.Abs("../../..")
	bin := filepath.Join(root, "conformance/server/bin/replay")
	if runtime.GOOS == "windows" {
		bin += ".exe"
	}
	if _, err := os.Stat(bin); err != nil {
		note := "conformance: no replay server at conformance/server/bin/replay; run `mise run conformance:server:build`"
		if os.Getenv("IOHR_TEST_REQUIRE_REPLAY") != "" {
			t.Fatal(note)
		}
		t.Skip(note)
	}
	cmd := exec.Command(bin, "--addr", "127.0.0.1:0", "--cases", filepath.Join(root, "conformance/cases")) //nolint:gosec // the repository's own replay server
	cmd.Stderr = os.Stderr
	out, err := cmd.StdoutPipe()
	if err != nil {
		t.Fatal(err)
	}
	if err := cmd.Start(); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = cmd.Process.Kill(); _ = cmd.Wait() })
	lines := make(chan string, 1)
	go func() {
		s := bufio.NewScanner(out)
		if s.Scan() {
			lines <- s.Text()
		}
		for s.Scan() { //nolint:revive // drain the server's output
		}
	}()
	select {
	case first := <-lines:
		const prefix = "replay: listening on "
		if !strings.HasPrefix(first, prefix) {
			t.Fatalf("unexpected first line: %s", first)
		}
		return strings.TrimSpace(strings.TrimPrefix(first, prefix))
	case <-time.After(10 * time.Second):
		t.Fatal("the replay server did not announce itself within 10 s")
	}
	return ""
}

func getJSON(t *testing.T, method, url string, body any, into any) int {
	t.Helper()
	var reader *bytes.Reader
	if body != nil {
		b, _ := json.Marshal(body)
		reader = bytes.NewReader(b)
	} else {
		reader = bytes.NewReader(nil)
	}
	req, _ := http.NewRequestWithContext(context.Background(), method, url, reader)
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		t.Fatal(err)
	}
	defer func() { _ = resp.Body.Close() }()
	if into != nil && resp.StatusCode < 300 {
		if err := json.NewDecoder(resp.Body).Decode(into); err != nil {
			t.Fatal(err)
		}
	}
	return resp.StatusCode
}

func str(args map[string]any, k string) string {
	if v, ok := args[k]; ok && v != nil {
		return fmt.Sprint(v)
	}
	return ""
}

// call runs an action through the generated public surface.
func call(ctx context.Context, api *public.Client, op string, args map[string]any) (*inorbit.RawResponse, error) {
	switch op {
	case "me":
		r, err := api.Me(ctx)
		return rawOf(r, err)
	case "accounts.get_me":
		r, err := api.Accounts().GetMe(ctx)
		return rawOf(r, err)
	case "accounts.get_usage":
		p := &models.AccountsGetUsageParams{}
		if v := str(args, "from"); v != "" {
			p.From = &v
		}
		if v := str(args, "to"); v != "" {
			p.To = &v
		}
		r, err := api.Accounts().GetUsage(ctx, str(args, "org_id"), p)
		return rawOf(r, err)
	case "radar.get_digest":
		r, err := api.Radar().GetDigest(ctx, str(args, "id"))
		return rawOf(r, err)
	case "events.create_endpoint":
		var body models.CreateEndpointRequest
		b, _ := json.Marshal(args)
		if err := json.Unmarshal(b, &body); err != nil {
			return nil, err
		}
		r, err := api.Events().CreateEndpoint(ctx, body)
		return rawOf(r, err)
	case "events.delete_endpoint":
		r, err := api.Events().DeleteEndpoint(ctx, str(args, "endpoint_id"))
		return rawOf(r, err)
	}
	return nil, fmt.Errorf("the conformance schema names an op this driver does not know: %s", op)
}

func rawOf[T any](r *inorbit.Response[T], err error) (*inorbit.RawResponse, error) {
	if err != nil {
		return nil, err
	}
	return r.Raw, nil
}

func kind(err error) string {
	var (
		a *inorbit.APIError
		c *inorbit.ConnectionError
		t *inorbit.TimeoutError
		u *inorbit.AuthError
		g *inorbit.ConfigError
	)
	switch {
	case errors.As(err, &a):
		return "api"
	case errors.As(err, &c):
		return "connection"
	case errors.As(err, &t):
		return "timeout"
	case errors.As(err, &u):
		return "auth"
	case errors.As(err, &g):
		return "config"
	}
	return "other"
}

// subset reports whether want is contained in got: objects by key, arrays element by
// element with equal length, scalars by value.
func subset(want, got any) bool {
	switch w := want.(type) {
	case map[string]any:
		g, ok := got.(map[string]any)
		if !ok {
			return false
		}
		for k, v := range w {
			if !subset(v, g[k]) {
				return false
			}
		}
		return true
	case []any:
		g, ok := got.([]any)
		if !ok || len(g) != len(w) {
			return false
		}
		for i := range w {
			if !subset(w[i], g[i]) {
				return false
			}
		}
		return true
	}
	return reflect.DeepEqual(want, got)
}

func TestEveryCasePasses(t *testing.T) {
	url := startReplay(t)
	var list struct {
		Cases []string `json:"cases"`
	}
	getJSON(t, "GET", url+"/_cases", nil, &list)
	if len(list.Cases) == 0 {
		t.Fatal("no cases listed")
	}
	var failed []string
	for _, name := range list.Cases {
		var loaded struct {
			Case testCase `json:"case"`
		}
		status := getJSON(t, "POST", url+"/_case", map[string]string{"name": name}, &loaded)
		if status == http.StatusNotImplemented {
			t.Logf("skip %s: not implemented by the replay server yet", name)
			continue
		}
		if status >= 300 {
			t.Fatalf("%s: loading answered %d", name, status)
		}
		c := loaded.Case
		if contains(c.Pending, "go") || c.Area == "sse" || c.Area == "socket" {
			t.Logf("skip %s: pending for go", name)
			continue
		}
		opts := []inorbit.Option{
			inorbit.WithBaseURL(url), inorbit.WithTokenURL(url + "/oauth2/token"),
			inorbit.WithKey(or(c.Client.KeyID, "ak_test"), or(c.Client.KeySecret, "s3cr3t")),
			inorbit.WithScopes(orSlice(c.Client.Scopes, []string{"identity:read"})...),
		}
		if c.Client.MaxRetries != nil {
			opts = append(opts, inorbit.WithMaxRetries(*c.Client.MaxRetries))
		}
		if c.Client.TimeoutMS > 0 {
			opts = append(opts, inorbit.WithTimeout(time.Duration(c.Client.TimeoutMS)*time.Millisecond))
		}
		client, err := inorbit.NewClient(opts...)
		if err != nil {
			t.Fatalf("%s: %v", name, err)
		}
		api := public.New(client)
		type result struct {
			raw *inorbit.RawResponse
			err error
		}
		var results []result
		run := func() result {
			raw, err := call(context.Background(), api, c.Action.Op, c.Action.Args)
			return result{raw, err}
		}
		if c.Action.Concurrent > 0 {
			results = make([]result, c.Action.Concurrent)
			var wg sync.WaitGroup
			for i := range results {
				wg.Go(func() { results[i] = run() })
			}
			wg.Wait()
		} else {
			for range max(c.Action.Repeat, 1) {
				results = append(results, run())
			}
		}
		var v verdict
		getJSON(t, "GET", url+"/_result", nil, &v)
		var problems []string
		if v.Status != "pass" {
			problems = append(problems, fmt.Sprintf("server: %s mismatch=%v next=%v", v.Status, v.Mismatch, v.Next))
		}
		if c.Expect.Attempts != nil && v.Attempts != *c.Expect.Attempts {
			problems = append(problems, fmt.Sprintf("attempts: want %d, got %d", *c.Expect.Attempts, v.Attempts))
		}
		if c.Expect.TokenExchanges != nil && v.TokenExchanges != *c.Expect.TokenExchanges {
			problems = append(problems, fmt.Sprintf("token_exchanges: want %d, got %d", *c.Expect.TokenExchanges, v.TokenExchanges))
		}
		for _, r := range results {
			switch {
			case r.err == nil && c.Expect.OK != nil:
				var got any
				_ = json.Unmarshal(r.raw.Body, &got)
				if !subset(c.Expect.OK, got) {
					problems = append(problems, fmt.Sprintf("ok: want a superset of %v, got %v", c.Expect.OK, got))
				}
			case r.err == nil && c.Expect.Error != nil:
				problems = append(problems, fmt.Sprintf("want an error, got HTTP %d", r.raw.Status))
			case r.err != nil && c.Expect.Error != nil:
				problems = append(problems, checkError(r.err, c.Expect.Error.Kind, c.Expect.Error.Code, c.Expect.Error.Status, c.Expect.Error.MessageContains, c.Expect.Error.MessageExcludes)...)
			case r.err != nil && c.Expect.OK != nil:
				problems = append(problems, "want ok, got "+r.err.Error())
			}
		}
		if len(problems) == 0 {
			t.Logf("pass %s", name)
		} else {
			failed = append(failed, name+":\n  "+strings.Join(problems, "\n  "))
		}
	}
	if len(failed) > 0 {
		t.Fatal(strings.Join(failed, "\n"))
	}
}

func checkError(err error, wantKind, wantCode string, wantStatus int, contains, excludes string) []string {
	var problems []string
	k := kind(err)
	if wantKind != "" && k != wantKind {
		problems = append(problems, fmt.Sprintf("error kind: want %s, got %s (%v)", wantKind, k, err))
	}
	var a *inorbit.APIError
	if errors.As(err, &a) {
		if wantCode != "" && string(a.Code) != wantCode {
			problems = append(problems, fmt.Sprintf("error code: want %s, got %s", wantCode, a.Code))
		}
		if wantStatus != 0 && a.Status != wantStatus {
			problems = append(problems, fmt.Sprintf("error status: want %d, got %d", wantStatus, a.Status))
		}
	} else if wantCode != "" || wantStatus != 0 {
		problems = append(problems, "error: want an API error, got "+err.Error())
	}
	if contains != "" && !strings.Contains(err.Error(), contains) {
		problems = append(problems, fmt.Sprintf("message: want it to contain %q, got %q", contains, err.Error()))
	}
	if excludes != "" && strings.Contains(err.Error(), excludes) {
		problems = append(problems, fmt.Sprintf("message: must not contain %q, got %q", excludes, err.Error()))
	}
	return problems
}

func contains(list []string, s string) bool {
	for _, v := range list {
		if v == s {
			return true
		}
	}
	return false
}

func or(s, d string) string {
	if s == "" {
		return d
	}
	return s
}

func orSlice(s, d []string) []string {
	if len(s) == 0 {
		return d
	}
	return s
}
