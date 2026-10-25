// Command typed-error shows how a refused call comes back: an *inorbit.APIError with the
// status, the code, the problem text and the request id. Match on the code, never on the
// text. This example expects the 403 and exits 0 when it gets it.
//
//	INORBIT_TOKEN=... go run ./typed-error
package main

import (
	"context"
	"errors"
	"fmt"
	"os"

	inorbit "github.com/inorbithr/sdk/go"
	"github.com/inorbithr/sdk/go/public"
)

func main() {
	api, err := public.FromEnv()
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(2)
	}
	_, err = api.Me(context.Background())
	var apiErr *inorbit.APIError
	switch {
	case err == nil:
		fmt.Fprintln(os.Stderr, "expected the API to refuse the call")
		os.Exit(1)
	case errors.As(err, &apiErr) && apiErr.Code == inorbit.CodeForbidden:
		fmt.Printf("refused: status %d, code %s, %s\n", apiErr.Status, apiErr.Code, apiErr.Problem)
	default:
		fmt.Fprintln(os.Stderr, err) // a network error, a timeout or another code
		os.Exit(1)
	}
}
