// Command config-load shows Load, the way an application builds a client: code, then the
// environment, then the iohr config file, then the iohr login, each setting from the first
// that sets it (docs/config.md). Describe says where every setting came from, secrets
// redacted, which is what `iohr sdk config` prints.
//
//	go run ./config-load      # after iohr login, or with INORBIT_TOKEN set
package main

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"time"

	inorbit "github.com/inorbithr/sdk/go"
	"github.com/inorbithr/sdk/go/public"
)

func main() {
	ctx := context.Background()
	api, err := public.Load(ctx, inorbit.WithTimeout(10*time.Second)) // code wins over the rest
	if err != nil {
		fmt.Fprintln(os.Stderr, err) // a ConfigError lists every problem, or every source tried
		os.Exit(2)
	}
	doc, _ := json.MarshalIndent(api.Runtime().Config().Describe(), "", "  ")
	fmt.Println(string(doc))
	me, err := api.Me(ctx)
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
	fmt.Printf("%s, a %s\n", me.Value.Subject, me.Value.Kind)
}
