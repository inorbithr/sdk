package greet

import "testing"

func TestHello(t *testing.T) {
	if got := Hello("Ana"); got != "Hello, Ana!" {
		t.Fatalf("Hello = %q", got)
	}
}
