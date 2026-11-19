// Package ids makes request ids.
package ids

import (
	"crypto/rand"
	"encoding/hex"
)

// New returns a random request id.
func New() string {
	b := make([]byte, 16)
	_, _ = rand.Read(b)
	return hex.EncodeToString(b)
}
