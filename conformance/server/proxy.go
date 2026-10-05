package main

import (
	"context"
	"io"
	"net"
	"net/http"
	"strings"
	"time"
)

// tunnel is what the proxy knows about a connection it opened to one of the listeners:
// the Proxy-Authorization the client sent with its CONNECT, if any.
type tunnel struct {
	auth string
}

// viaProxy marks a request the proxy served itself (an absolute-form request to the
// plain listener), with the Proxy-Authorization it carried.
type viaProxyKey struct{}

// proxyHandler is the CONNECT proxy: plain HTTP, no authentication required. It reaches
// only the replay's own listeners, never another host, and records each tunnel by the
// local address of its upstream connection, so the listener at the other end can tell
// that a request came through it (`via: proxy`).
func (s *Server) proxyHandler() http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Method == http.MethodConnect {
			s.connect(w, r)
			return
		}
		// A client that proxies a plain http:// URL sends the absolute URL instead of
		// CONNECT; the plain listener's routes are served here directly.
		if target := s.proxyTarget(r.URL.Host); r.URL.IsAbs() && r.URL.Scheme == "http" && target != "" && target == s.endpoints.httpAddr {
			ctx := context.WithValue(r.Context(), viaProxyKey{}, tunnel{auth: r.Header.Get("Proxy-Authorization")})
			r.Header.Del("Proxy-Authorization")
			s.ServeHTTP(w, r.WithContext(ctx))
			return
		}
		writeJSON(w, http.StatusForbidden, problem("forbidden",
			"the replay proxy carries CONNECT (or plain http requests) to the replay's own listeners only"))
	})
}

func (s *Server) connect(w http.ResponseWriter, r *http.Request) {
	target := s.proxyTarget(r.Host)
	if target == "" {
		writeJSON(w, http.StatusForbidden, problem("forbidden", "the replay proxy reaches the replay's own listeners only, not "+r.Host))
		return
	}
	upstream, err := net.DialTimeout("tcp", target, 5*time.Second)
	if err != nil {
		writeJSON(w, http.StatusBadGateway, problem("bad_gateway", err.Error()))
		return
	}
	hj, ok := w.(http.Hijacker)
	if !ok {
		_ = upstream.Close()
		writeJSON(w, http.StatusInternalServerError, problem("internal", "cannot take over the connection"))
		return
	}
	client, buf, err := hj.Hijack()
	if err != nil {
		_ = upstream.Close()
		return
	}
	// Registered before a byte flows, so the listener knows the connection when the
	// tunnelled request arrives.
	key := upstream.LocalAddr().String()
	s.tunnels.Store(key, tunnel{auth: r.Header.Get("Proxy-Authorization")})
	defer s.tunnels.Delete(key)

	if _, err := client.Write([]byte("HTTP/1.1 200 Connection established\r\n\r\n")); err != nil {
		_ = client.Close()
		_ = upstream.Close()
		return
	}
	if n := buf.Reader.Buffered(); n > 0 {
		early, _ := buf.Peek(n)
		if _, err := upstream.Write(early); err != nil {
			_ = client.Close()
			_ = upstream.Close()
			return
		}
	}
	done := make(chan struct{}, 2)
	go func() { _, _ = io.Copy(upstream, client); done <- struct{}{} }()
	go func() { _, _ = io.Copy(client, upstream); done <- struct{}{} }()
	<-done
	_ = client.Close()
	_ = upstream.Close()
	<-done
}

// proxyTarget maps a CONNECT authority to the listener it names, or "" when it names
// none: the port must be one of the listeners' ports and the host a loopback name or
// the listener's own host.
func (s *Server) proxyTarget(authority string) string {
	host, port, err := net.SplitHostPort(authority)
	if err != nil {
		return ""
	}
	for _, addr := range []string{s.endpoints.httpAddr, s.endpoints.httpsAddr, s.endpoints.mtlsAddr} {
		lhost, lport, err := net.SplitHostPort(addr)
		if err != nil || lport != port {
			continue
		}
		if strings.EqualFold(host, "localhost") || host == lhost {
			return addr
		}
		if ip := net.ParseIP(host); ip != nil && ip.IsLoopback() {
			return addr
		}
	}
	return ""
}

// via tells whether a request came through the proxy, and with which tunnel.
func (s *Server) via(r *http.Request) (string, tunnel) {
	if t, ok := r.Context().Value(viaProxyKey{}).(tunnel); ok {
		return "proxy", t
	}
	if t, ok := s.tunnels.Load(r.RemoteAddr); ok {
		return "proxy", t.(tunnel)
	}
	return "direct", tunnel{}
}
