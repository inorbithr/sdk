package inorbit

import "testing"

func TestUnprocessableIsKnownAndAnUnknownCodeIsKept(t *testing.T) {
	if !CodeUnprocessable.Known() || codeStatus[CodeUnprocessable] != 422 {
		t.Fatalf("unprocessable: known=%v status=%d", CodeUnprocessable.Known(), codeStatus[CodeUnprocessable])
	}
	if got := CodeForStatus(422); got != CodeUnprocessable {
		t.Fatalf("CodeForStatus(422) = %q", got)
	}
	if fresh := Code("brand_new_code"); fresh.Known() {
		t.Fatal("an unknown code reads as known")
	}
}
