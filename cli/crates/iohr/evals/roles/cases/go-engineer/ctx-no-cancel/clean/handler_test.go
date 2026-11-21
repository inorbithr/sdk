package orders

import (
	"context"
	"net/http"
	"net/http/httptest"
	"testing"
)

type memStore map[string]map[string]any

func (m memStore) Order(_ context.Context, id string) (map[string]any, error) {
	return m[id], nil
}

func TestServe(t *testing.T) {
	mux := http.NewServeMux()
	mux.Handle("GET /orders/{id}", Handler{Store: memStore{"7": {"id": "7"}}})
	rec := httptest.NewRecorder()
	mux.ServeHTTP(rec, httptest.NewRequest(http.MethodGet, "/orders/7", nil))
	if rec.Code != http.StatusOK {
		t.Fatalf("status %d", rec.Code)
	}
}
