package ids

import "testing"

func TestNewIsUnique(t *testing.T) {
	if New() == New() {
		t.Fatal("two ids are equal")
	}
}
