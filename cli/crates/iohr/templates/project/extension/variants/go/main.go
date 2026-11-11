// @@ID@@: the demo project's private macOS service.
//
// Every minute it writes $STATE/heartbeat.json (the health InOrbit reads) and asks iohr's
// token channel for a token with the one scope its manifest declares. It also serves a tiny
// panel on 127.0.0.1 that InOrbit Desktop can show as a pane: the page reads /api/heartbeat.
//
// Run it with `go run .` under `iohr ext dev`'s STATE and CACHE, or once with IOHR_EXT_ONCE=1
// as the self-test does. Standard library only.
package main

import (
	_ "embed"
	"encoding/json"
	"errors"
	"fmt"
	"net"
	"net/http"
	"os"
	"os/signal"
	"path/filepath"
	"strconv"
	"sync"
	"syscall"
	"time"
)

const (
	id      = "@@ID@@"
	version = "0.1.0"
)

var scopes = []string{"identity:read"}

//go:embed panel/index.html
var panel []byte

func writeJSON(state, name string, value any) error {
	data, err := json.Marshal(value)
	if err != nil {
		return err
	}
	tmp := filepath.Join(state, "."+name+".tmp")
	if err := os.WriteFile(tmp, data, 0o600); err != nil {
		return err
	}
	return os.Rename(tmp, filepath.Join(state, name))
}

func heartbeat(state string, beats int) (map[string]any, error) {
	tok := "granted"
	if _, err := token(scopes); err != nil { // kept in memory only, never printed
		if !errors.Is(err, errRefused) {
			return nil, err
		}
		fmt.Fprintf(os.Stderr, "token refused: %v\n", err)
		tok = "refused"
	}
	now := time.Now().UTC().Format("2006-01-02T15:04:05Z")
	beat := map[string]any{
		"id": id, "version": version, "state": "ok", "at": now,
		"beats": beats, "token": tok, "dev": os.Getenv("IOHR_EXT_DEV") == "1",
	}
	if err := writeJSON(state, "heartbeat.json", beat); err != nil {
		return nil, err
	}
	fmt.Printf("heartbeat %d at %s (token %s)\n", beats, now, tok)
	return beat, nil
}

// serve listens on 127.0.0.1 only; the port is PANEL_PORT or one the system picks, and it is
// written to $STATE/panel.json so the app knows where the panel is.
func serve(state string, latest func() map[string]any) error {
	port := 0
	if p := os.Getenv("PANEL_PORT"); p != "" {
		n, err := strconv.Atoi(p)
		if err != nil {
			return fmt.Errorf("PANEL_PORT is not a number: %w", err)
		}
		port = n
	}
	ln, err := net.Listen("tcp", fmt.Sprintf("127.0.0.1:%d", port))
	if err != nil {
		return err
	}
	mux := http.NewServeMux()
	mux.HandleFunc("/api/heartbeat", func(w http.ResponseWriter, _ *http.Request) {
		w.Header().Set("content-type", "application/json")
		_ = json.NewEncoder(w).Encode(latest())
	})
	mux.HandleFunc("/{$}", func(w http.ResponseWriter, _ *http.Request) {
		w.Header().Set("content-type", "text/html; charset=utf-8")
		_, _ = w.Write(panel)
	})
	url := fmt.Sprintf("http://%s/", ln.Addr())
	if err := writeJSON(state, "panel.json", map[string]string{"id": id, "url": url}); err != nil {
		return err
	}
	fmt.Println("panel at " + url)
	go func() { _ = http.Serve(ln, mux) }()
	return nil
}

func run() int {
	state := os.Getenv("STATE")
	if state == "" || os.Getenv("CACHE") == "" {
		fmt.Fprintln(os.Stderr, "STATE and CACHE are not set: run this with iohr ext dev")
		return 2
	}
	every := 60
	if v, err := strconv.Atoi(os.Getenv("HEARTBEAT_EVERY")); err == nil && v > 0 {
		every = v
	}
	once := os.Getenv("IOHR_EXT_ONCE") != ""
	fmt.Printf("%s %s starting; state in %s\n", id, version, state)

	var mu sync.Mutex
	var latest map[string]any
	if !once {
		err := serve(state, func() map[string]any {
			mu.Lock()
			defer mu.Unlock()
			return latest
		})
		if err != nil {
			fmt.Fprintln(os.Stderr, err)
			return 1
		}
	}
	stop := make(chan os.Signal, 1)
	signal.Notify(stop, syscall.SIGTERM, os.Interrupt)
	for beats := 1; ; beats++ {
		beat, err := heartbeat(state, beats)
		if err != nil {
			fmt.Fprintln(os.Stderr, err)
			return 1
		}
		mu.Lock()
		latest = beat
		mu.Unlock()
		if once {
			return 0
		}
		select {
		case <-stop:
			fmt.Println("stopping")
			return 0
		case <-time.After(time.Duration(every) * time.Second):
		}
	}
}

func main() { os.Exit(run()) }
