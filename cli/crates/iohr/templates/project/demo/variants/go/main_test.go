package main

// The self-test: `go test ./...` with the STATE and CACHE `iohr ext dev --test` gives (or the
// folders InOrbit Desktop's smoke test uses). One heartbeat, then what it wrote and where.

import (
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"testing"
)

func names(t *testing.T) []string {
	t.Helper()
	entries, err := os.ReadDir(".")
	if err != nil {
		t.Fatal(err)
	}
	out := make([]string, 0, len(entries))
	for _, e := range entries {
		out = append(out, e.Name())
	}
	return out
}

func TestOneHeartbeatInStateAndNothingElsewhere(t *testing.T) {
	t.Setenv("STATE", t.TempDir())
	t.Setenv("CACHE", t.TempDir())
	t.Setenv("IOHR_EXT_ONCE", "1")
	t.Setenv("IOHR_EXT_TOKEN_SOCKET", "")
	before := names(t)
	if code := run(); code != 0 {
		t.Fatalf("run exited with %d", code)
	}
	data, err := os.ReadFile(filepath.Join(os.Getenv("STATE"), "heartbeat.json"))
	if err != nil {
		t.Fatal(err)
	}
	var beat map[string]any
	if err := json.Unmarshal(data, &beat); err != nil {
		t.Fatal(err)
	}
	if beat["id"] != "@@ID@@" || beat["state"] != "ok" || beat["beats"] != float64(1) {
		t.Fatalf("unexpected heartbeat %v", beat)
	}
	// Granted when you are signed in and the channel is up, refused when not: either way it answered.
	if tok := beat["token"]; tok != "granted" && tok != "refused" {
		t.Fatalf("token is %v", tok)
	}
	if after := names(t); len(after) != len(before) {
		t.Fatalf("the project wrote into its own folder: %v -> %v", before, after)
	}
}

func TestAnUndeclaredScopeIsRefused(t *testing.T) {
	t.Setenv("IOHR_EXT_TOKEN_SOCKET", "")
	if _, err := token([]string{"tokens:write"}); !errors.Is(err, errRefused) {
		t.Fatalf("want a refusal, got %v", err)
	}
}
