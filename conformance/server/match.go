package main

import (
	"fmt"
	"maps"
	"regexp"
	"slices"
	"strings"
)

// checkMatchers rejects a request whose matchers cannot work: a regex that does not
// compile, an empty capture name, an unknown `via`.
func checkMatchers(req Request) error {
	for _, k := range slices.Sorted(maps.Keys(req.Headers)) {
		v := req.Headers[k]
		switch {
		case v == "$":
			return fmt.Errorf("header %s: a capture needs a name ($name)", k)
		case strings.HasPrefix(v, "~"):
			if _, err := wholeMatch(v[1:]); err != nil {
				return fmt.Errorf("header %s: %w", k, err)
			}
		}
	}
	if req.Via != "" && req.Via != "proxy" && req.Via != "direct" {
		return fmt.Errorf("via is proxy or direct, not %q", req.Via)
	}
	return nil
}

// wholeMatch compiles a `~regex` matcher so it must match the whole value.
func wholeMatch(expr string) (*regexp.Regexp, error) {
	return regexp.Compile(`^(?:` + expr + `)$`)
}

// matchHeaders checks the case's headers against the request's in sorted order. Names
// `$name` captures already holds must carry the captured value; names seen for the first
// time are returned as fresh captures, kept only if the whole exchange matches.
func matchHeaders(want, got, captures map[string]string) (string, map[string]string) {
	lower := make(map[string]string, len(want))
	for k, v := range want {
		lower[strings.ToLower(k)] = v
	}
	var fresh map[string]string
	for _, k := range slices.Sorted(maps.Keys(lower)) {
		pattern := lower[k]
		actual, ok := got[k]
		if !ok {
			return fmt.Sprintf("header %s: missing", k), nil
		}
		switch {
		case pattern == "*":
		case strings.HasPrefix(pattern, "$"):
			name := pattern[1:]
			prev, seen := captures[name]
			if !seen {
				prev, seen = fresh[name]
			}
			if seen && actual != prev {
				return fmt.Sprintf("header %s: want %q (the value captured as $%s), got %q", k, prev, name, actual), nil
			}
			if !seen {
				if fresh == nil {
					fresh = map[string]string{}
				}
				fresh[name] = actual
			}
		case strings.HasPrefix(pattern, "~"):
			re, err := wholeMatch(pattern[1:])
			if err != nil {
				return fmt.Sprintf("header %s: %v", k, err), nil
			}
			if !re.MatchString(actual) {
				return fmt.Sprintf("header %s: %q does not match %s", k, actual, pattern[1:]), nil
			}
		default:
			if actual != pattern {
				return fmt.Sprintf("header %s: want %q, got %q", k, pattern, actual), nil
			}
		}
	}
	return "", fresh
}

// matchAbsent fails a request that carries a header the case says must be left out.
func matchAbsent(names []string, got map[string]string) string {
	for _, k := range slices.Sorted(slices.Values(names)) {
		if v, sent := got[strings.ToLower(k)]; sent {
			return fmt.Sprintf("header %s: sent (%q); the case says it is left out", strings.ToLower(k), v)
		}
	}
	return ""
}

// matchTransport checks `via` and `client_cert`.
func matchTransport(want Request, got Seen) string {
	if want.Via != "" && want.Via != got.Via {
		return fmt.Sprintf("via: want %s, got %s", want.Via, got.Via)
	}
	if want.ClientCert == "" {
		return ""
	}
	if got.ClientCert == "" {
		return fmt.Sprintf("client_cert: want %s, got no client certificate", want.ClientCert)
	}
	if want.ClientCert != got.ClientCert && "CN="+want.ClientCert != got.ClientCert {
		return fmt.Sprintf("client_cert: want %s, got %s", want.ClientCert, got.ClientCert)
	}
	return ""
}
