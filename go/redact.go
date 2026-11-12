package inorbit

import (
	"fmt"
	"io"
	"log/slog"
)

// redacted is what a secret shows as wherever it could print: String, GoString, every
// fmt verb, a log/slog record and JSON (SR-10). Every type that holds a key secret, a
// token or a proxy password shows itself through the helpers below.
const redacted = "<redacted>"

// formatRedacted writes v's String for every verb, so %d, %x or %+v never reach the
// fields of a type that holds a secret.
func formatRedacted(st fmt.State, v fmt.Stringer) { _, _ = io.WriteString(st, v.String()) }

// logRedacted is v's String as a log/slog value, so a JSON or text handler never walks
// its fields.
func logRedacted(v fmt.Stringer) slog.Value { return slog.StringValue(v.String()) }
