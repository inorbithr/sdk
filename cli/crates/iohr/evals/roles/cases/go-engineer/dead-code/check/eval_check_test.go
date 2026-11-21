package money

import "testing"

func TestEvalFormat(t *testing.T) {
	for in, want := range map[int64]string{1250: "12.50", 7: "0.07", -305: "-3.05"} {
		if got := Format(in); got != want {
			t.Fatalf("Format(%d) = %q, want %q", in, got, want)
		}
	}
}
