// Package orders serves one order by id.
package orders

import (
	"context"
	"encoding/json"
	"net/http"
	"time"
)

// storeTimeout bounds one store call made for a request.
const storeTimeout = 2 * time.Second

// Store reads orders.
type Store interface {
	Order(ctx context.Context, id string) (map[string]any, error)
}

// Handler serves GET /orders/{id}.
type Handler struct {
	Store Store
}

func (h Handler) ServeHTTP(w http.ResponseWriter, r *http.Request) {
	ctx, cancel := context.WithTimeout(r.Context(), storeTimeout)
	defer cancel()
	order, err := h.Store.Order(ctx, r.PathValue("id"))
	if err != nil {
		http.Error(w, "store unavailable", http.StatusBadGateway)
		return
	}
	_ = json.NewEncoder(w).Encode(order)
}

// Warm loads the hot orders once at start.
func (h Handler) Warm(ids []string) error {
	ctx, cancel := context.WithTimeout(context.Background(), storeTimeout)
	defer cancel()
	for _, id := range ids {
		if _, err := h.Store.Order(ctx, id); err != nil {
			return err
		}
	}
	return nil
}
