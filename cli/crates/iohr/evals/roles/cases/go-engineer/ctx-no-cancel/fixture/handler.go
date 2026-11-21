// Package orders serves one order by id.
package orders

import (
	"context"
	"encoding/json"
	"net/http"
	"time"
)

// Store reads orders.
type Store interface {
	Order(ctx context.Context, id string) (map[string]any, error)
}

// Handler serves GET /orders/{id}.
type Handler struct {
	Store Store
}

func (h Handler) ServeHTTP(w http.ResponseWriter, r *http.Request) {
	order, err := h.Store.Order(context.Background(), r.PathValue("id"))
	if err != nil {
		http.Error(w, "store unavailable", http.StatusBadGateway)
		return
	}
	_ = json.NewEncoder(w).Encode(order)
}

// Warm loads the hot orders once at start.
func (h Handler) Warm(ids []string) error {
	ctx, _ := context.WithTimeout(context.Background(), 2*time.Second)
	for _, id := range ids {
		if _, err := h.Store.Order(ctx, id); err != nil {
			return err
		}
	}
	return nil
}
