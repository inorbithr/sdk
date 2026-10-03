package codegen

import (
	"context"
	"errors"
	"slices"
	"strconv"
	"testing"
)

// pager serves pages of ints by token: "" is the first page, then "2", "3", ...
type pager struct {
	pages  [][]int
	failAt int // the page (from 1) that fails; 0 for none
	repeat bool
	asked  []string
}

func (p *pager) page(_ context.Context, token string) ([]int, string, error) {
	p.asked = append(p.asked, token)
	n := len(p.asked)
	if n == p.failAt {
		return nil, "", errors.New("page failed")
	}
	next := ""
	if n < len(p.pages) {
		next = strconv.Itoa(n + 1)
	}
	if p.repeat {
		next = token
	}
	return p.pages[n-1], next, nil
}

func TestPagesFollowsTheTokenToTheLastPage(t *testing.T) {
	p := &pager{pages: [][]int{{1, 2}, {3}, {4, 5}}}
	var got []int
	for v, err := range Pages(t.Context(), p.page) {
		if err != nil {
			t.Fatal(err)
		}
		got = append(got, v)
	}
	if !slices.Equal(got, []int{1, 2, 3, 4, 5}) || !slices.Equal(p.asked, []string{"", "2", "3"}) {
		t.Fatalf("items %v, tokens %q", got, p.asked)
	}
}

func TestPagesStopsWhenTheLoopBreaks(t *testing.T) {
	p := &pager{pages: [][]int{{1, 2}, {3}}}
	for v := range Pages(t.Context(), p.page) {
		if v == 1 {
			break
		}
	}
	if len(p.asked) != 1 {
		t.Fatalf("fetched %d pages after a break on the first item, want 1", len(p.asked))
	}
}

func TestPagesYieldsAnErrorAndStops(t *testing.T) {
	p := &pager{pages: [][]int{{1}, {2}, {3}}, failAt: 2}
	var got []int
	var errs int
	for v, err := range Pages(t.Context(), p.page) {
		if err != nil {
			errs++
			continue
		}
		got = append(got, v)
	}
	if !slices.Equal(got, []int{1}) || errs != 1 || len(p.asked) != 2 {
		t.Fatalf("items %v, errors %d, pages %d", got, errs, len(p.asked))
	}
}

func TestPagesChecksTheContextBetweenPages(t *testing.T) {
	ctx, cancel := context.WithCancel(t.Context())
	defer cancel()
	p := &pager{pages: [][]int{{1}, {2}}}
	var last error
	for v, err := range Pages(ctx, p.page) {
		if v == 1 {
			cancel()
		}
		last = err
	}
	if !errors.Is(last, context.Canceled) || len(p.asked) != 1 {
		t.Fatalf("last %v after %d pages, want Canceled after 1", last, len(p.asked))
	}
}

func TestPagesStopsWhenAPageNamesItselfAsTheNext(t *testing.T) {
	p := &pager{pages: [][]int{{1}, {2}}, repeat: true}
	n := 0
	for range Pages(t.Context(), p.page) {
		n++
	}
	if n != 1 || len(p.asked) != 1 {
		t.Fatalf("%d items from %d pages, want the first page once", n, len(p.asked))
	}
}
