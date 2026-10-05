package inorbit

import (
	"net/http"
	"regexp"
	"strconv"
	"strings"
	"time"
)

// RateLimitMode is what the rate_limit middleware does with the rate-limit headers
// (docs/config.md section 7.8).
type RateLimitMode string

const (
	// RateLimitObserve reads the headers into every result's snapshot and waits for
	// nothing: the default.
	RateLimitObserve RateLimitMode = "observe"

	// RateLimitWait also holds an attempt until the window resets when the latest
	// snapshot says nothing remains.
	RateLimitWait RateLimitMode = "wait"

	// RateLimitOff does not read the headers.
	RateLimitOff RateLimitMode = "off"
)

// RateLimitPolicy is the quota policy the IETF RateLimit-Policy field names.
type RateLimitPolicy struct {
	// Name is the policy's name.
	Name string

	// Quota is the requests the window allows; 0 when not sent.
	Quota int64

	// Window is the window's length; 0 when not sent.
	Window time.Duration
}

// RateLimit is what an answer said about the rate limit. A field the answer did not send
// is nil.
type RateLimit struct {
	// Limit is the requests the window allows.
	Limit *int64

	// Remaining is the requests left in the window.
	Remaining *int64

	// Reset is how long until the window resets.
	Reset *time.Duration

	// Policy is the policy the IETF fields named, if any.
	Policy *RateLimitPolicy
}

var sfToken = regexp.MustCompile(`^[A-Za-z*][A-Za-z0-9_\-.:%*/]*$`)

// sfItem is the first item of a structured-field list: "name";k=v;k=v.
func sfItem(field string) (string, map[string]string, bool) {
	first, _, _ := strings.Cut(field, ",")
	parts := strings.Split(strings.TrimSpace(first), ";")
	head := strings.TrimSpace(parts[0])
	var name string
	switch {
	case len(head) >= 2 && head[0] == '"' && head[len(head)-1] == '"' && !strings.ContainsAny(head[1:len(head)-1], `"\`):
		name = head[1 : len(head)-1]
	case sfToken.MatchString(head):
		name = head
	default:
		return "", nil, false
	}
	params := map[string]string{}
	for _, p := range parts[1:] {
		k, v, ok := strings.Cut(strings.TrimSpace(p), "=")
		if ok && strings.TrimSpace(k) != "" {
			params[strings.TrimSpace(k)] = strings.TrimSpace(v)
		}
	}
	return name, params, true
}

// digits parses a whole number of digits only.
func digits(v string) (int64, bool) {
	if v == "" {
		return 0, false
	}
	for _, c := range v {
		if c < '0' || c > '9' {
			return 0, false
		}
	}
	n, err := strconv.ParseInt(v, 10, 64)
	return n, err == nil
}


// ParseRateLimit reads the rate-limit snapshot h carries: the IETF draft 11 RateLimit
// and RateLimit-Policy fields, which win when both are sent, or Envoy's X-RateLimit-*
// headers (Reset in seconds). It returns nil when h carries none, or a malformed one.
func ParseRateLimit(h http.Header) *RateLimit {
	if field := h.Get("RateLimit"); field != "" {
		if name, params, ok := sfItem(field); ok {
			rl := &RateLimit{}
			if n, ok := digits(params["r"]); ok {
				rl.Remaining = Ptr(n)
			}
			if n, ok := digits(params["t"]); ok {
				rl.Reset = Ptr(time.Duration(n) * time.Second)
			}
			if pf := h.Get("RateLimit-Policy"); pf != "" {
				if pname, pparams, ok := sfItem(pf); ok && pname == name {
					p := &RateLimitPolicy{Name: pname}
					if q, ok := digits(pparams["q"]); ok {
						p.Quota = q
						rl.Limit = Ptr(q)
					}
					if w, ok := digits(pparams["w"]); ok {
						p.Window = time.Duration(w) * time.Second
					}
					rl.Policy = p
				}
			}
			return rl
		}
	}
	read := func(name string) (*int64, bool) {
		v, ok := h[http.CanonicalHeaderKey(name)]
		if !ok || len(v) == 0 {
			return nil, true
		}
		n, ok := digits(strings.TrimSpace(v[0]))
		if !ok {
			return nil, false
		}
		return Ptr(n), true
	}
	limit, ok1 := read("X-RateLimit-Limit")
	remaining, ok2 := read("X-RateLimit-Remaining")
	reset, ok3 := read("X-RateLimit-Reset")
	if !ok1 || !ok2 || !ok3 || (limit == nil && remaining == nil && reset == nil) {
		return nil
	}
	rl := &RateLimit{Limit: limit, Remaining: remaining}
	if reset != nil {
		rl.Reset = Ptr(time.Duration(*reset) * time.Second)
	}
	return rl
}
