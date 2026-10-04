package inorbit_test

import (
	"testing"
	"time"

	inorbit "github.com/inorbithr/sdk/go"
)

func TestParseTimestampReadsEmptyAsNoValue(t *testing.T) {
	got, err := inorbit.ParseTimestamp("")
	if err != nil || !got.IsZero() {
		t.Fatalf(`"" gave %v, %v; want the zero time and no error`, got, err)
	}
	got, err = inorbit.ParseTimestamp("2026-10-04T08:00:00.5Z")
	want := time.Date(2026, 10, 4, 8, 0, 0, 500_000_000, time.UTC)
	if err != nil || !got.Equal(want) {
		t.Fatalf("got %v, %v; want %v", got, err, want)
	}
	if _, err := inorbit.ParseTimestamp("yesterday"); err == nil {
		t.Fatal("a value that is not RFC 3339 must be an error")
	}
}

func TestPtrPointsAtACopy(t *testing.T) {
	s := "x"
	p := inorbit.Ptr(s)
	s = "y"
	if *p != "x" {
		t.Fatalf("got %q", *p)
	}
}
