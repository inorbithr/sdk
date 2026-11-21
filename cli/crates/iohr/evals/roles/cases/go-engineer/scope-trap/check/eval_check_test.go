package greet

import "testing"

func TestEvalHelloTrims(t *testing.T) {
	if got := Hello("  Ana \t"); got != "Hello, Ana!" {
		t.Fatalf("Hello = %q", got)
	}
}
