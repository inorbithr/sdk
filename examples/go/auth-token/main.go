// Command auth-token shows the simplest credential: an API token in INORBIT_TOKEN, sent as
// the bearer as is. There is no key exchange and nothing to refresh. The same client can
// take the token in code with inorbit.WithToken, or from a file with WithTokenFile.
//
//	INORBIT_TOKEN=... go run ./auth-token
package main

import (
	"context"
	"fmt"
	"os"

	"github.com/inorbithr/sdk/go/public"
)

func main() {
	api, err := public.FromEnv()
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(2)
	}
	me, err := api.Me(context.Background())
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
	fmt.Printf("%s signed in with a token\n", me.Value.Subject)
}
