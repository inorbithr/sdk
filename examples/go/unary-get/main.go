// Command unary-get shows one unary call: GET /v1/me, with the key or token the
// environment holds. The SDK gets a token, sends it, and returns the typed answer.
//
//	INORBIT_TOKEN=... go run ./unary-get
package main

import (
	"context"
	"fmt"
	"os"

	"github.com/inorbithr/sdk/go/public"
)

func main() {
	api, err := public.FromEnv() // INORBIT_TOKEN, or INORBIT_KEY_ID, _KEY_SECRET and _SCOPES
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(2)
	}
	me, err := api.Me(context.Background())
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
	fmt.Printf("%s, a %s, with scopes %v\n", me.Value.Subject, me.Value.Kind, me.Value.Scopes)
}
