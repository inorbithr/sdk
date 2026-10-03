// Package codegen is what a generated surface imports from the runtime, and nothing
// else does. It is a contract with iohr sdk generate: a change that breaks generated
// code bumps the version, and a surface generated for another version fails to build.
package codegen

import (
	"net/url"
	"strings"
)

// Version is the surface contract this runtime implements.
const Version = V1

// V1 is what a surface generated for contract 1 refers to; a runtime with another
// contract drops it, so such a surface fails to build with a message that names it.
const V1 = 1

// PathSegment percent-encodes value as one path segment: every byte but the RFC 3986
// unreserved characters, so a slash or a space in an id never changes the route.
func PathSegment(value string) string {
	var b strings.Builder
	for i := range len(value) {
		c := value[i]
		if 'a' <= c && c <= 'z' || 'A' <= c && c <= 'Z' || '0' <= c && c <= '9' ||
			c == '-' || c == '.' || c == '_' || c == '~' {
			b.WriteByte(c)
			continue
		}
		const hex = "0123456789ABCDEF"
		b.WriteByte('%')
		b.WriteByte(hex[c>>4])
		b.WriteByte(hex[c&15])
	}
	return b.String()
}

// Query starts the query parameters of one call.
func Query() url.Values { return url.Values{} }
