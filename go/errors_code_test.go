package inorbit

import "testing"

func TestUnprocessableIsKnownAndAnUnknownCodeIsKept(t *testing.T) {
	if !CodeUnprocessable.Known() || codeStatus[CodeUnprocessable] != 422 {
		t.Fatalf("unprocessable: known=%v status=%d", CodeUnprocessable.Known(), codeStatus[CodeUnprocessable])
	}
	if got := CodeForStatus(422); got != CodeUnprocessable {
		t.Fatalf("CodeForStatus(422) = %q", got)
	}
	if !CodeUpgradeRequired.Known() || codeStatus[CodeUpgradeRequired] != 426 || CodeForStatus(426) != CodeUpgradeRequired {
		t.Fatalf("upgrade_required: known=%v status=%d", CodeUpgradeRequired.Known(), codeStatus[CodeUpgradeRequired])
	}
	if fresh := Code("brand_new_code"); fresh.Known() {
		t.Fatal("an unknown code reads as known")
	}
}
