package feed

import (
	"net/http"
	"net/http/httptest"
	"testing"
)

func TestLatest(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		_, _ = w.Write([]byte("entry"))
	}))
	defer srv.Close()
	b, err := Latest(srv.Client(), srv.URL)
	if err != nil || string(b) != "entry" {
		t.Fatalf("Latest = %q, %v", b, err)
	}
}
