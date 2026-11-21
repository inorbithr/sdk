package pricing

import (
	"net/http"
	"net/http/httptest"
	"testing"
	"time"
)

// The service takes the call and never answers: Fetch must give up on its own.
func TestEvalFetchGivesUpOnASilentService(t *testing.T) {
	stop := make(chan struct{})
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		select {
		case <-stop:
		case <-r.Context().Done():
		}
	}))
	defer srv.Close()
	defer close(stop)
	done := make(chan error, 1)
	go func() { _, err := Fetch(srv.URL, "a1"); done <- err }()
	select {
	case err := <-done:
		if err == nil {
			t.Fatal("Fetch returned no error from a silent service")
		}
	case <-time.After(15 * time.Second):
		t.Fatal("Fetch still waits after 15s on a silent service: the client has no timeout")
	}
}
