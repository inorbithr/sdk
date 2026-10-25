// Command auth-custom-token shows a token provider of your own: anything with a Token
// method, here a function that would read a secret store. The SDK asks it before every
// attempt. Never log the token.
//
//	INORBIT_BASE_URL=... go run ./auth-custom-token
package main

import (
	"context"
	"fmt"
	"os"

	inorbit "github.com/inorbithr/sdk/go"
	"github.com/inorbithr/sdk/go/public"
)

// vault stands in for wherever your tokens live.
type vault struct{}

func (vault) Token(context.Context) (inorbit.Token, error) {
	return inorbit.Token{Access: "custom-token-1"}, nil // set ExpiresAt when you know it
}

func main() {
	opts := []inorbit.Option{inorbit.WithTokenProvider(vault{})}
	if base := os.Getenv("INORBIT_BASE_URL"); base != "" { // empty means the public API
		opts = append(opts, inorbit.WithBaseURL(base))
	}
	client, err := inorbit.NewClient(opts...)
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(2)
	}
	api := public.New(client)
	me, err := api.Me(context.Background())
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
	fmt.Printf("%s signed in with a custom provider\n", me.Value.Subject)
}
