// Package greet writes greetings.
package greet

import "strings"

// Hello greets name, without the spaces around it.
func Hello(name string) string {
	return "Hello, " + strings.TrimSpace(name) + "!"
}
