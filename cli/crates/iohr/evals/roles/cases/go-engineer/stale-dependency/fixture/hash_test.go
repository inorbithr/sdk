package accounts

import "testing"

func TestHashMatches(t *testing.T) {
	h, err := Hash("correct horse")
	if err != nil {
		t.Fatal(err)
	}
	if !Matches(h, "correct horse") || Matches(h, "wrong") {
		t.Fatal("Matches disagrees with Hash")
	}
}
