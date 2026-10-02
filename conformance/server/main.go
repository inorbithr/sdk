// Command replay is the conformance replay server: it plays the API and the token
// endpoint for one case at a time, so every SDK is tested against the same exchanges.
//
//	replay [--addr 127.0.0.1:0] [--cases conformance/cases]
//
// The first line it prints is "replay: listening on http://HOST:PORT"; a driver reads
// it, points base_url and token_url there, POSTs /_case, runs the case's action and
// GETs /_result. See conformance/README.md.
package main

import (
	"context"
	"errors"
	"flag"
	"fmt"
	"net"
	"net/http"
	"os"
	"os/signal"
	"syscall"
	"time"
)

func main() {
	if err := run(); err != nil {
		fmt.Fprintln(os.Stderr, "replay:", err)
		os.Exit(1)
	}
}

func run() error {
	addr := flag.String("addr", "127.0.0.1:0", "address to listen on; port 0 picks a free one")
	cases := flag.String("cases", "conformance/cases", "directory holding <area>/<name>.yaml cases")
	flag.Parse()

	if info, err := os.Stat(*cases); err != nil || !info.IsDir() {
		return fmt.Errorf("no cases directory at %s (use --cases)", *cases)
	}
	ln, err := net.Listen("tcp", *addr)
	if err != nil {
		return err
	}
	srv := &http.Server{
		Handler:           NewServer(*cases),
		ReadHeaderTimeout: 10 * time.Second,
		IdleTimeout:       60 * time.Second,
	}
	fmt.Printf("replay: listening on http://%s\n", ln.Addr())

	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()
	go func() {
		<-ctx.Done()
		shutdown, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()
		_ = srv.Shutdown(shutdown)
	}()
	if err := srv.Serve(ln); !errors.Is(err, http.ErrServerClosed) {
		return err
	}
	return nil
}
