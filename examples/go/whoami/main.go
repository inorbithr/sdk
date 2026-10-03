// Command whoami prints who the API says the caller is, using the public surface:
// the operations an API token or key may call.
//
//	INORBIT_TOKEN=... go run ./whoami
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
	api, err := public.FromEnv() // INORBIT_TOKEN, or INORBIT_KEY_ID, _KEY_SECRET and _SCOPES
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(2)
	}
	me, err := api.Me(context.Background())
	var apiErr *inorbit.APIError
	switch {
	case errors.As(err, &apiErr):
		fmt.Fprintf(os.Stderr, "the API refused: %s (%s)\n", apiErr.Problem, apiErr.Code)
		os.Exit(1)
	case err != nil:
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
	fmt.Printf("%s, a %s, with scopes %v\n", me.Value.Subject, me.Value.Kind, me.Value.Scopes)
}
