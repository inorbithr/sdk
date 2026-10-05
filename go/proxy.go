package inorbit

import (
	"net/netip"
	"net/url"
	"strconv"
	"strings"
)

// The one no_proxy grammar every runtime implements itself (docs/config.md section 6.2),
// and the loopback rule. Names are never resolved to check a range.

type noProxyKind int

const (
	npAll noProxyKind = iota
	npCIDR
	npIP
	npName
)

type noProxyEntry struct {
	kind   noProxyKind
	prefix netip.Prefix
	ip     netip.Addr
	name   string
	port   int // 0: any port
}

// parseIP reads an IP literal, with or without brackets around an IPv6 one; zones are
// not addresses.
func parseIP(raw string) (netip.Addr, bool) {
	s := raw
	if strings.HasPrefix(s, "[") && strings.HasSuffix(s, "]") {
		s = s[1 : len(s)-1]
	}
	a, err := netip.ParseAddr(s)
	if err != nil || a.Zone() != "" {
		return netip.Addr{}, false
	}
	return a.Unmap(), true
}

func validLabel(l string) bool {
	if l == "" {
		return false
	}
	for _, c := range l {
		if (c < 'a' || c > 'z') && (c < '0' || c > '9') && c != '_' && c != '-' {
			return false
		}
	}
	return true
}

// parseNoProxyEntry reads one entry; false when it fits none of the forms.
func parseNoProxyEntry(raw string) (noProxyEntry, bool) {
	e := strings.ToLower(strings.TrimSpace(raw))
	if e == "*" {
		return noProxyEntry{kind: npAll}, true
	}
	if addr, bits, ok := strings.Cut(e, "/"); ok {
		ip, ok := parseIP(addr)
		if !ok || len(bits) == 0 || len(bits) > 3 {
			return noProxyEntry{}, false
		}
		n, err := strconv.Atoi(bits)
		if err != nil || n < 0 || n > ip.BitLen() {
			return noProxyEntry{}, false
		}
		p, err := ip.Prefix(n)
		if err != nil {
			return noProxyEntry{}, false
		}
		return noProxyEntry{kind: npCIDR, prefix: p}, true
	}
	if ip, ok := parseIP(e); ok {
		return noProxyEntry{kind: npIP, ip: ip}, true
	}
	host, port := e, 0
	if i := strings.LastIndexByte(e, ':'); i >= 0 {
		h := e[:i]
		if !strings.Contains(h, ":") || strings.HasSuffix(h, "]") {
			p := e[i+1:]
			n, err := strconv.Atoi(p)
			if len(p) == 0 || len(p) > 5 || err != nil || n < 0 || n > 65535 || strings.ContainsAny(p, "+-") {
				return noProxyEntry{}, false
			}
			host, port = h, n
		}
	}
	if ip, ok := parseIP(host); ok {
		return noProxyEntry{kind: npIP, ip: ip, port: port}, true
	}
	name := strings.TrimLeft(host, ".")
	if name == "" {
		return noProxyEntry{}, false
	}
	for _, l := range strings.Split(name, ".") {
		if !validLabel(l) {
			return noProxyEntry{}, false
		}
	}
	return noProxyEntry{kind: npName, name: name, port: port}, true
}

// noProxyList is a parsed no_proxy list.
type noProxyList []noProxyEntry

// parseNoProxy reads a list, skipping empty entries; the first entry that fits no form
// is returned as the error.
func parseNoProxy(entries []string) (noProxyList, string, bool) {
	var out noProxyList
	for _, raw := range entries {
		if strings.TrimSpace(raw) == "" {
			continue
		}
		e, ok := parseNoProxyEntry(raw)
		if !ok {
			return nil, raw, false
		}
		out = append(out, e)
	}
	return out, "", true
}

func defaultPort(u *url.URL) int {
	if p := u.Port(); p != "" {
		n, _ := strconv.Atoi(p)
		return n
	}
	if u.Scheme == "http" || u.Scheme == "ws" {
		return 80
	}
	return 443
}

// bypasses reports whether no_proxy leaves u out of the proxy.
func (l noProxyList) bypasses(u *url.URL) bool {
	host := strings.ToLower(u.Hostname())
	port := defaultPort(u)
	ip, isIP := parseIP(host)
	for _, e := range l {
		switch e.kind {
		case npAll:
			return true
		case npCIDR:
			if isIP && e.prefix.Contains(ip) {
				return true
			}
		case npIP:
			if isIP && ip == e.ip && (e.port == 0 || e.port == port) {
				return true
			}
		case npName:
			if (host == e.name || strings.HasSuffix(host, "."+e.name)) && (e.port == 0 || e.port == port) {
				return true
			}
		}
	}
	return false
}

// isLoopback reports whether host (a URL's hostname) is this machine.
func isLoopback(host string) bool {
	h := strings.ToLower(host)
	if h == "localhost" {
		return true
	}
	ip, ok := parseIP(h)
	return ok && ip.IsLoopback()
}

// proxyChoice is the proxy in effect and whether it was set on purpose (code,
// INORBIT_PROXY or the file), which makes it apply to loopback too.
type proxyChoice struct {
	url      string
	explicit bool
}

// proxyFor is the proxy u goes through, or "" for a direct connection.
func proxyFor(u *url.URL, p *proxyChoice, noProxy noProxyList) string {
	if p == nil || p.url == "off" {
		return ""
	}
	if !p.explicit && isLoopback(u.Hostname()) {
		return ""
	}
	if noProxy.bypasses(u) {
		return ""
	}
	return p.url
}
