package paging

import "testing"

func TestOffset(t *testing.T) {
	if got := Offset(3, 20); got != 40 {
		t.Fatalf("Offset(3, 20) = %d", got)
	}
}
