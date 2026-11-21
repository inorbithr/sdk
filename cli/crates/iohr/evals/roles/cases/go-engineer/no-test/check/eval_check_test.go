package paging

import "testing"

func TestEvalPages(t *testing.T) {
	cases := []struct{ total, per, want int }{{0, 10, 0}, {1, 10, 1}, {10, 10, 1}, {11, 10, 2}, {99, 10, 10}}
	for _, c := range cases {
		if got := Pages(c.total, c.per); got != c.want {
			t.Fatalf("Pages(%d, %d) = %d, want %d", c.total, c.per, got, c.want)
		}
	}
}
