package orders

import (
	"context"
	"net/http"
	"net/http/httptest"
	"testing"
)

type deadlineStore struct{ sawDeadline, sawCancel bool }

func (s *deadlineStore) Order(ctx context.Context, _ string) (map[string]any, error) {
	_, s.sawDeadline = ctx.Deadline()
	s.sawCancel = ctx.Done() != nil
	return map[string]any{}, nil
}

// The store call must carry a deadline that ends with the request.
func TestEvalStoreCallHasTheRequestDeadline(t *testing.T) {
	s := &deadlineStore{}
	req := httptest.NewRequest(http.MethodGet, "/orders/7", nil)
	ctx, cancel := context.WithCancel(req.Context())
	defer cancel()
	Handler{Store: s}.ServeHTTP(httptest.NewRecorder(), req.WithContext(ctx))
	if !s.sawDeadline || !s.sawCancel {
		t.Fatal("the store call has no deadline: context.Background() drops the request's")
	}
}
