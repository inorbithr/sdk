// Command concurrent-calls shows one client shared by several goroutines. The client is
// safe for concurrent use, and callers that need a token at the same moment share one
// exchange.
//
//	INORBIT_TOKEN=... go run ./concurrent-calls
package main

import (
	"context"
	"fmt"
	"os"
	"sync"

	"github.com/inorbithr/sdk/go/public"
)

func main() {
	api, err := public.FromEnv()
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(2)
	}
	const calls = 3
	subjects := make([]string, calls)
	errs := make([]error, calls)
	var wg sync.WaitGroup
	for i := range calls {
		wg.Add(1)
		go func() {
			defer wg.Done()
			me, err := api.Me(context.Background())
			if err != nil {
				errs[i] = err
				return
			}
			subjects[i] = me.Value.Subject
		}()
	}
	wg.Wait()
	for _, err := range errs {
		if err != nil {
			fmt.Fprintln(os.Stderr, err)
			os.Exit(1)
		}
	}
	fmt.Printf("%d calls at once: %v\n", calls, subjects)
}
