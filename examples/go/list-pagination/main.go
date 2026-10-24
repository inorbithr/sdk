// Command list-pagination shows a paged list. AllListMonitors yields every item and
// follows next_page_token for you; the second half reads one page by hand with
// page_size and page_token.
//
//	INORBIT_ORG_ID=org_... INORBIT_TOKEN=... go run ./list-pagination
package main

import (
	"context"
	"fmt"
	"os"

	"github.com/inorbithr/sdk/go/public"
	"github.com/inorbithr/sdk/go/public/models"
)

func main() {
	ctx := context.Background()
	api, err := public.FromEnv()
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(2)
	}
	org := os.Getenv("INORBIT_ORG_ID")
	pageSize := int32(2)

	// Every item, page after page. The loop ends at the last page or at the first error.
	n := 0
	for m, err := range api.Connections().AllListMonitors(ctx, org, &models.ConnectionsListMonitorsParams{PageSize: &pageSize}) {
		if err != nil {
			fmt.Fprintln(os.Stderr, err)
			os.Exit(1)
		}
		n++
		fmt.Printf("%s %s\n", m.MonitorID, m.Name)
	}
	fmt.Printf("%d monitors\n", n)

	// One page by hand: the next page token comes back with the page.
	one := int32(1)
	page, err := api.Connections().ListMonitors(ctx, org, &models.ConnectionsListMonitorsParams{PageSize: &one})
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
	fmt.Printf("one page: %d monitor, more after %q\n", len(page.Value.Monitors), page.Value.NextPageToken)
}
