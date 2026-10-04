// Command replay is the conformance replay server: it plays the API and the token
// endpoint for one case at a time, so every SDK is tested against the same exchanges.
//
//	replay [--addr 127.0.0.1:0] [--cases conformance/cases] [--dir DIR]
//	       [--https-addr A] [--mtls-addr A] [--proxy-addr A]
//	replay auth token --profile P --format json    (the fake iohr)
//
// The first line it prints is "replay: listening on http://HOST:PORT"; a driver reads
// it, points base_url and token_url there, POSTs /_case, runs the case's action and
// GETs /_result. The TLS, mTLS and proxy listeners follow on later lines and in every
// /_case answer. See conformance/README.md.
package main

import (
	"context"
	"flag"
	"fmt"
	"os"
	"os/signal"
	"syscall"
	"time"
)

func main() {
	if isIohr(os.Args[1:]) {
		os.Exit(fakeIohr(os.Args[1:], time.Now(), os.Stdout, os.Stderr))
	}
	if err := run(); err != nil {
		fmt.Fprintln(os.Stderr, "replay:", err)
		os.Exit(1)
	}
}

func run() error {
	var opts Options
	flag.StringVar(&opts.Addr, "addr", "127.0.0.1:0", "address of the plain HTTP listener; port 0 picks a free one")
	flag.StringVar(&opts.CasesDir, "cases", "conformance/cases", "directory holding <area>/<name>.yaml cases")
	flag.StringVar(&opts.HTTPSAddr, "https-addr", "", "address of the TLS listener (default: the plain listener's host, a free port)")
	flag.StringVar(&opts.MTLSAddr, "mtls-addr", "", "address of the mTLS listener (default: the plain listener's host, a free port)")
	flag.StringVar(&opts.ProxyAddr, "proxy-addr", "", "address of the CONNECT proxy (default: the plain listener's host, a free port)")
	flag.StringVar(&opts.Dir, "dir", "", "directory for ca.pem, client.pem and client-key.pem (default: a new temporary one, removed on exit)")
	flag.Parse()

	if info, err := os.Stat(opts.CasesDir); err != nil || !info.IsDir() {
		return fmt.Errorf("no cases directory at %s (use --cases)", opts.CasesDir)
	}
	st, err := Start(opts)
	if err != nil {
		return err
	}
	e := st.Endpoints
	// The first line is the contract drivers read; the rest is for people.
	fmt.Printf("replay: listening on %s\n", e.HTTPURL)
	fmt.Printf("replay: tls on %s, mtls on %s, proxy on %s\n", e.HTTPSURL, e.MTLSURL, e.ProxyURL)
	fmt.Printf("replay: ca %s, client certificate %s, key %s\n", e.CAFile, e.ClientCertFile, e.ClientKeyFile)

	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()
	<-ctx.Done()
	shutdown, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	return st.Shutdown(shutdown)
}
