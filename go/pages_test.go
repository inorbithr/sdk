package inorbit_test

import (
	"io"
	"net/http"
	"slices"
	"strings"
	"testing"

	inorbit "github.com/inorbithr/sdk/go"
	"github.com/inorbithr/sdk/go/public"
	"github.com/inorbithr/sdk/go/public/models"
)

type deliveriesAPI struct {
	asked []string // "status|page_token" of each page asked for
}

func (d *deliveriesAPI) RoundTrip(r *http.Request) (*http.Response, error) {
	q := r.URL.Query()
	d.asked = append(d.asked, q.Get("status")+"|"+q.Get("page_token"))
	body := map[string]string{
		"":   `{"deliveries":[{"id":"d1"},{"id":"d2"}],"next_page_token":"p2"}`,
		"p2": `{"deliveries":[{"id":"d3"}],"next_page_token":""}`,
	}[q.Get("page_token")]
	return &http.Response{StatusCode: 200, Header: http.Header{}, Body: io.NopCloser(strings.NewReader(body))}, nil
}

func deliveries(t *testing.T) (*public.Client, *deliveriesAPI) {
	t.Helper()
	api := &deliveriesAPI{}
	c, err := inorbit.NewClient(inorbit.WithToken("tok"), inorbit.WithBaseURL("https://api.test"),
		inorbit.WithHTTPClient(&http.Client{Transport: api}))
	if err != nil {
		t.Fatal(err)
	}
	return public.New(c), api
}

func TestAllListDeliveriesFollowsThePagesAndKeepsTheFilters(t *testing.T) {
	c, api := deliveries(t)
	status := "failed"
	var ids []string
	for d, err := range c.Events().AllListDeliveries(t.Context(), "ep_1", &models.EventsListDeliveriesParams{Status: &status}) {
		if err != nil {
			t.Fatal(err)
		}
		ids = append(ids, d.ID)
	}
	if !slices.Equal(ids, []string{"d1", "d2", "d3"}) {
		t.Fatalf("items %v", ids)
	}
	if !slices.Equal(api.asked, []string{"failed|", "failed|p2"}) {
		t.Fatalf("pages asked %q, want the filter on both and the token on the second", api.asked)
	}
}

func TestAllListDeliveriesFetchesNoMoreAfterABreak(t *testing.T) {
	c, api := deliveries(t)
	for d, err := range c.Events().AllListDeliveries(t.Context(), "ep_1", nil) {
		if err != nil || d.ID == "d1" {
			break
		}
	}
	if len(api.asked) != 1 {
		t.Fatalf("asked %d pages after breaking on the first item, want 1", len(api.asked))
	}
}
