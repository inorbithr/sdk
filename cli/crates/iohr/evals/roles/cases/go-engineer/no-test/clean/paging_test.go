package paging

import "testing"

func TestOffset(t *testing.T) {
	if got := Offset(3, 20); got != 40 {
		t.Fatalf("Offset(3, 20) = %d", got)
	}
}

func TestPagesCountsAPartialLastPage(t *testing.T) {
	cases := []struct{ total, per, want int }{{0, 20, 0}, {20, 20, 1}, {21, 20, 2}, {45, 20, 3}}
	for _, c := range cases {
		if got := Pages(c.total, c.per); got != c.want {
			t.Errorf("Pages(%d, %d) = %d, want %d", c.total, c.per, got, c.want)
		}
	}
}
