// Command load builds a client with Load, the way an application should: the
// environment, the iohr config file and the iohr login, each setting from the first that
// sets it (docs/config.md). It adds a middleware that tags every attempt with a team
// header, switches the rate-limit middleware off, and prints where each setting came
// from before calling the API.
//
//	go run ./load            # after iohr login, or with INORBIT_TOKEN set
package main

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"os"
	"time"

	inorbit "github.com/inorbithr/sdk/go"
	"github.com/inorbithr/sdk/go/public"
)

// team adds x-team to every attempt; a middleware sees the finished request, the
// Authorization header included, so it must never log it.
var team = inorbit.Middleware{
	Name: "team",
	Wrap: func(next http.RoundTripper) http.RoundTripper {
		return inorbit.RoundTripperFunc(func(r *http.Request) (*http.Response, error) {
			r.Header.Set("X-Team", "payments")
			return next.RoundTrip(r)
		})
	},
}

func main() {
	ctx := context.Background()
	api, err := public.Load(ctx,
		inorbit.WithTimeout(10*time.Second), // code wins over INORBIT_TIMEOUT and the file
		inorbit.WithPipeline(func(p *inorbit.Pipeline) {
			p.AddPerRetry(team)
			p.Remove("rate_limit")
		}),
	)
	if err != nil {
		fmt.Fprintln(os.Stderr, err) // a ConfigError lists every problem, or every source tried
		os.Exit(2)
	}
	doc, _ := json.MarshalIndent(api.Runtime().Config().Describe(), "", "  ")
	fmt.Println(string(doc)) // what iohr sdk config prints, secrets redacted
	me, err := api.Me(ctx)
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
	fmt.Printf("%s, a %s\n", me.Value.Subject, me.Value.Kind)
}
