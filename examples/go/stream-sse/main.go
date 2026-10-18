// Command stream-sse shows a server stream over server-sent events: StreamEvents yields
// one typed event at a time and ends when the server ends the stream. Needs events:read.
//
//	INORBIT_TOKEN=... go run ./stream-sse
package main

import (
	"context"
	"fmt"
	"os"

	inorbit "github.com/inorbithr/sdk/go"
	"github.com/inorbithr/sdk/go/public"
)

func main() {
	api, err := public.FromEnv(inorbit.WithStreams(inorbit.StreamsSSE))
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(2)
	}
	n := 0
	for ev, err := range api.Events().StreamEvents(context.Background(), nil) {
		if err != nil {
			fmt.Fprintln(os.Stderr, err)
			os.Exit(1)
		}
		n++
		fmt.Printf("%s %s %s\n", ev.OccurredAt, ev.Type, ev.ID)
	}
	fmt.Printf("%d events\n", n)
}
