// Command events prints the account's events as they happen, until interrupted. It
// needs a key or token with events:read; INORBIT_STREAMS=socket carries the stream over
// the /v1/ws socket instead of server-sent events.
//
//	INORBIT_TOKEN=... go run ./events
package main

import (
	"context"
	"fmt"
	"os"
	"os/signal"

	inorbit "github.com/inorbithr/sdk/go"
	"github.com/inorbithr/sdk/go/public"
)

func main() {
	streams := inorbit.StreamsSSE
	if os.Getenv("INORBIT_STREAMS") == "socket" {
		streams = inorbit.StreamsSocket
	}
	api, err := public.FromEnv(inorbit.WithStreams(streams))
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(2)
	}
	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt)
	defer stop()
	for ev, err := range api.Events().StreamEvents(ctx, nil) {
		if err != nil {
			if ctx.Err() != nil {
				return
			}
			fmt.Fprintln(os.Stderr, err)
			os.Exit(1)
		}
		fmt.Printf("%s %s %s\n", ev.OccurredAt, ev.Type, ev.ID)
	}
}
