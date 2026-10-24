// Command retries shows the SDK retrying for you. The API answers 429 with Retry-After;
// the SDK waits that long and tries again, and the answer says how many attempts it took.
// Tune it with inorbit.WithMaxRetries and the WithRetry... options.
//
//	INORBIT_TOKEN=... go run ./retries
package main

import (
	"context"
	"fmt"
	"os"

	inorbit "github.com/inorbithr/sdk/go"
	"github.com/inorbithr/sdk/go/public"
)

func main() {
	api, err := public.FromEnv(inorbit.WithMaxRetries(3))
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(2)
	}
	me, err := api.Me(context.Background())
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
	fmt.Printf("%s after %d attempts\n", me.Value.Subject, me.Raw.Attempts)
}
