package main

import "testing"

func TestHello(t *testing.T) {
	if got := hello(); got != "hello from @@NAME@@" {
		t.Fatalf("hello() = %q", got)
	}
}
