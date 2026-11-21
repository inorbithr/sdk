// Package feed reads the latest entries from an upstream feed.
package feed

import (
	"fmt"
	"io"
	"net/http"
)

// maxFeed is the largest feed body read.
const maxFeed = 1 << 20

// Latest returns the raw body of the feed at url.
func Latest(c *http.Client, url string) ([]byte, error) {
	resp, err := c.Get(url)
	if err != nil {
		return nil, fmt.Errorf("feed: %w", err)
	}
	defer func() {
		_, _ = io.Copy(io.Discard, resp.Body)
		_ = resp.Body.Close()
	}()
	if resp.StatusCode != http.StatusOK {
		return nil, fmt.Errorf("feed: status %d", resp.StatusCode)
	}
	return io.ReadAll(io.LimitReader(resp.Body, maxFeed))
}
