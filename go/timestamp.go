package inorbit

import (
	"errors"
	"time"
)

// ParseTimestamp reads a timestamp as the API sends it: an RFC 3339 string, or "" when
// the field is unset (spec/README.md, rule N5). "" gives the zero time.Time (check it
// with IsZero) and no error; anything else that is not RFC 3339 is an error.
func ParseTimestamp(value string) (time.Time, error) {
	if value == "" {
		return time.Time{}, nil
	}
	t, err := time.Parse(time.RFC3339Nano, value)
	if err != nil {
		return time.Time{}, errors.New("inorbit: not an RFC 3339 timestamp")
	}
	return t, nil
}

// Ptr returns a pointer to v. Every request field is optional (nothing in a request is
// required, and an unset field is left out of the body), so a generated request
// carries its scalars as pointers: inorbit.Ptr("https://example.com/hook").
func Ptr[T any](v T) *T {
	return &v
}
