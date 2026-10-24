// Command stream-socket shows the same stream over the one multiplexed /v1/ws socket.
// Only the option differs from stream-sse: the call, the loop and the events are the same.
//
//	INORBIT_TOKEN=... go run ./stream-socket
package main

import (
	"context"
	"fmt"
	"os"

	inorbit "github.com/inorbithr/sdk/go"
	"github.com/inorbithr/sdk/go/public"
)

func main() {
	api, err := public.FromEnv(inorbit.WithStreams(inorbit.StreamsSocket))
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
