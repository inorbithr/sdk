// Package ids makes request ids.
package ids

import "github.com/google/uuid"

// New returns a random request id.
func New() string {
	return uuid.NewString()
}
