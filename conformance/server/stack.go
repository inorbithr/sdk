package main

import (
	"context"
	"crypto/tls"
	"errors"
	"net"
	"net/http"
	"os"
	"path/filepath"
	"time"
)

// tempPrefix names the temporary directories Start makes.
const tempPrefix = "replay-pki-"

// sweepStale removes directories an earlier replay made and never cleaned up: named
// with tempPrefix, holding a ca.pem, untouched for longer than age.
func sweepStale(tmp string, age time.Duration) {
	matches, _ := filepath.Glob(filepath.Join(tmp, tempPrefix+"*"))
	for _, dir := range matches {
		info, err := os.Stat(filepath.Join(dir, "ca.pem"))
		if err == nil && time.Since(info.ModTime()) > age {
			_ = os.RemoveAll(dir)
		}
	}
}

// Endpoints are the listeners' locations, handed to drivers with every loaded case.
type Endpoints struct {
	HTTPURL        string `json:"http_url"`
	HTTPSURL       string `json:"https_url"`
	MTLSURL        string `json:"mtls_url"`
	ProxyURL       string `json:"proxy_url"`
	CAFile         string `json:"ca_file"`
	ClientCertFile string `json:"client_cert_file"`
	ClientKeyFile  string `json:"client_key_file"`

	httpAddr, httpsAddr, mtlsAddr string
}

// baseURL is the listener a case's client.transport selects: what `{replay}` stands for
// and where base_url and token_url point.
func (e Endpoints) baseURL(transport string) string {
	switch transport {
	case "https", "proxy":
		return e.HTTPSURL
	case "mtls":
		return e.MTLSURL
	default:
		return e.HTTPURL
	}
}

// Options say where the listeners go. Every address defaults to the plain one's host
// with a free port; Dir defaults to a new temporary directory, removed on Close.
type Options struct {
	Addr, HTTPSAddr, MTLSAddr, ProxyAddr string
	CasesDir                             string
	Dir                                  string
}

// Stack is the replay server on all four listeners: plain HTTP, TLS, mTLS and the
// CONNECT proxy. They share one Server, so one session and one case.
type Stack struct {
	Server    *Server
	PKI       *PKI
	Endpoints Endpoints

	servers []*http.Server
	tempDir string // removed on Close when Start made it
}

// Start opens every listener and serves them in the background.
func Start(opts Options) (_ *Stack, err error) {
	st := &Stack{Server: NewServer(opts.CasesDir)}
	dir := opts.Dir
	if dir == "" {
		// A driver that kills the server leaves its directory behind; the next start
		// removes those older than a day (their certificates are throwaway).
		sweepStale(os.TempDir(), 24*time.Hour)
		if dir, err = os.MkdirTemp("", tempPrefix); err != nil {
			return nil, err
		}
		st.tempDir = dir
	}
	defer func() {
		if err != nil {
			_ = st.Close()
		}
	}()
	if st.PKI, err = newPKI(dir); err != nil {
		return nil, err
	}
	plain, err := net.Listen("tcp", opts.Addr)
	if err != nil {
		return nil, err
	}
	host, _, _ := net.SplitHostPort(plain.Addr().String())
	free := func(addr string) string {
		if addr == "" {
			return net.JoinHostPort(host, "0")
		}
		return addr
	}
	secure, err := net.Listen("tcp", free(opts.HTTPSAddr))
	if err != nil {
		_ = plain.Close()
		return nil, err
	}
	mutual, err := net.Listen("tcp", free(opts.MTLSAddr))
	if err != nil {
		_ = plain.Close()
		_ = secure.Close()
		return nil, err
	}
	proxy, err := net.Listen("tcp", free(opts.ProxyAddr))
	if err != nil {
		_ = plain.Close()
		_ = secure.Close()
		_ = mutual.Close()
		return nil, err
	}

	e := Endpoints{
		HTTPURL:        "http://" + plain.Addr().String(),
		HTTPSURL:       "https://" + secure.Addr().String(),
		MTLSURL:        "https://" + mutual.Addr().String(),
		ProxyURL:       "http://" + proxy.Addr().String(),
		CAFile:         st.PKI.CAFile,
		ClientCertFile: st.PKI.ClientCertFile,
		ClientKeyFile:  st.PKI.ClientKeyFile,
		httpAddr:       plain.Addr().String(),
		httpsAddr:      secure.Addr().String(),
		mtlsAddr:       mutual.Addr().String(),
	}
	st.Endpoints = e
	st.Server.endpoints = e

	st.serve(plain, st.Server)
	st.serve(tls.NewListener(secure, st.PKI.serverTLS(false)), st.Server)
	st.serve(tls.NewListener(mutual, st.PKI.serverTLS(true)), st.Server)
	st.serve(proxy, st.Server.proxyHandler())
	return st, nil
}

func (st *Stack) serve(ln net.Listener, h http.Handler) {
	srv := &http.Server{
		Handler:           h,
		ReadHeaderTimeout: 10 * time.Second,
		IdleTimeout:       60 * time.Second,
		// HTTP/1.1 only on every listener (see serverTLS).
		TLSNextProto: map[string]func(*http.Server, *tls.Conn, http.Handler){},
	}
	st.servers = append(st.servers, srv)
	go func() { _ = srv.Serve(ln) }()
}

// Shutdown stops the listeners, waits for answers in flight up to ctx, and removes the
// temporary directory (with the client key) when Start made it.
func (st *Stack) Shutdown(ctx context.Context) error {
	var errs []error
	for _, srv := range st.servers {
		if err := srv.Shutdown(ctx); err != nil && !errors.Is(err, http.ErrServerClosed) {
			errs = append(errs, err)
		}
	}
	if st.tempDir != "" {
		errs = append(errs, os.RemoveAll(st.tempDir))
	}
	return errors.Join(errs...)
}

// Close stops everything at once.
func (st *Stack) Close() error {
	var errs []error
	for _, srv := range st.servers {
		errs = append(errs, srv.Close())
	}
	if st.tempDir != "" {
		errs = append(errs, os.RemoveAll(st.tempDir))
	}
	return errors.Join(errs...)
}
