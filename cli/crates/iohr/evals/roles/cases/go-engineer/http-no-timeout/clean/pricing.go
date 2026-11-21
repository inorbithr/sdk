// Package pricing reads prices from the pricing service.
package pricing

import (
	"encoding/json"
	"fmt"
	"net/http"
	"time"
)

// client bounds every call: http.DefaultClient has no timeout, so a silent service would
// hold the caller forever.
var client = &http.Client{
	Timeout:   5 * time.Second,
	Transport: http.DefaultTransport.(*http.Transport).Clone(),
}

// Price is one product's price in cents.
type Price struct {
	SKU   string `json:"sku"`
	Cents int64  `json:"cents"`
}

// Fetch returns the price of sku from the service at base.
func Fetch(base, sku string) (Price, error) {
	resp, err := client.Get(base + "/prices/" + sku)
	if err != nil {
		return Price{}, fmt.Errorf("pricing: %w", err)
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		return Price{}, fmt.Errorf("pricing: status %d", resp.StatusCode)
	}
	var p Price
	if err := json.NewDecoder(resp.Body).Decode(&p); err != nil {
		return Price{}, fmt.Errorf("pricing: %w", err)
	}
	return p, nil
}
