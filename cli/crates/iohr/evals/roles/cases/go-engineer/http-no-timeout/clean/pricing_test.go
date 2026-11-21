package pricing

import (
	"net/http"
	"net/http/httptest"
	"testing"
)

func TestFetch(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		_, _ = w.Write([]byte(`{"sku":"a1","cents":1250}`))
	}))
	defer srv.Close()
	p, err := Fetch(srv.URL, "a1")
	if err != nil || p.Cents != 1250 {
		t.Fatalf("Fetch = %+v, %v", p, err)
	}
}
