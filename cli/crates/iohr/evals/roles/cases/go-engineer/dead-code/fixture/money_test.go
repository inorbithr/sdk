package money

import "testing"

func TestRound(t *testing.T) {
	if got := Round(12.345); got != 1235 {
		t.Fatalf("Round(12.345) = %d", got)
	}
}
