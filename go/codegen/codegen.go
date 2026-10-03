// Package codegen is what a generated surface imports from the runtime, and nothing
// else does. It is a contract with iohr sdk generate: a change that breaks generated
// code bumps the version, and a surface generated for another version fails to build.
package codegen

import (
	"context"
	"iter"
	"net/url"
	"strings"
)

// Version is the surface contract this runtime implements.
const Version = V2

// V1 is what a surface generated for contract 1 refers to; a runtime with another
// contract drops it, so such a surface fails to build with a message that names it.
const V1 = 1

// V2 is contract 1 plus [Pages], for the iterators over paged lists.
const V2 = 2

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

// Pages iterates over every item of a paged list. page fetches the page a token names
// (the first page for ""), and returns its items and the token of the next page, ""
// after the last. The iteration stops at the first error, which it yields with the zero
// item, when the consumer stops ranging, when ctx is done, or when a page names itself
// as the next, so a server that repeats a token cannot loop forever.
func Pages[T any](ctx context.Context, page func(ctx context.Context, token string) ([]T, string, error)) iter.Seq2[T, error] {
	return func(yield func(T, error) bool) {
		var zero T
		token := ""
		for {
			if err := ctx.Err(); err != nil {
				yield(zero, err)
				return
			}
			items, next, err := page(ctx, token)
			if err != nil {
				yield(zero, err)
				return
			}
			for _, item := range items {
				if !yield(item, nil) {
					return
				}
			}
			if next == "" || next == token {
				return
			}
			token = next
		}
	}
}
