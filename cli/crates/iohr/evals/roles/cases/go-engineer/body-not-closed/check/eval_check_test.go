package feed

import (
	"io"
	"net/http"
	"strings"
	"testing"
)

type tracked struct {
	io.Reader
	closed *int
}

func (t tracked) Close() error { *t.closed++; return nil }

type stub struct {
	status int
	closed *int
}

func (s stub) RoundTrip(*http.Request) (*http.Response, error) {
	return &http.Response{StatusCode: s.status, Body: tracked{strings.NewReader("entry"), s.closed}}, nil
}

// Every body must be closed, on the error path too, or the connection leaks.
func TestEvalEveryBodyIsClosed(t *testing.T) {
	for _, status := range []int{http.StatusOK, http.StatusServiceUnavailable} {
		closed := 0
		c := &http.Client{Transport: stub{status, &closed}}
		_, _ = Latest(c, "http://upstream.invalid/feed")
		if closed == 0 {
			t.Fatalf("status %d: resp.Body was never closed", status)
		}
	}
}
