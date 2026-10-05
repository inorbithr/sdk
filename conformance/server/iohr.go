package main

import (
	"encoding/json"
	"flag"
	"fmt"
	"io"
	"time"
)

// missingProfile is the profile the fake iohr has no login for.
const missingProfile = "missing"

// isIohr tells whether the binary was started as the fake `iohr auth token`.
func isIohr(args []string) bool {
	return len(args) >= 2 && args[0] == "auth" && args[1] == "token"
}

// fakeIohr plays `iohr auth token --profile P --format json` (docs/config.md section
// 5.4) for drivers whose case sets `client.cli: true`: it prints
// {"access_token":"cli-<P>","expires_at":"<now + 15 min>","profile":"<P>"} and exits 0,
// or, for the profile `missing`, prints one line on standard error and exits 1. It
// carries no `account`: the fake has no login and knows no account, and an SDK reads only
// the token and its expiry. It returns the exit code.
func fakeIohr(args []string, now time.Time, stdout, stderr io.Writer) int {
	fs := flag.NewFlagSet("iohr auth token", flag.ContinueOnError)
	fs.SetOutput(io.Discard)
	profile := fs.String("profile", "", "profile whose login supplies the token")
	format := fs.String("format", "", "output format; only json")
	if err := fs.Parse(args[2:]); err != nil || fs.NArg() > 0 {
		_, _ = fmt.Fprintln(stderr, "iohr: usage: iohr auth token --profile <name> --format json")
		return 2
	}
	if *profile == "" || *format != "json" {
		_, _ = fmt.Fprintln(stderr, "iohr: usage: iohr auth token --profile <name> --format json")
		return 2
	}
	if *profile == missingProfile {
		_, _ = fmt.Fprintf(stderr, "iohr: no login for profile %q; run iohr login --profile %s\n", *profile, *profile)
		return 1
	}
	out, err := json.Marshal(map[string]string{
		"access_token": "cli-" + *profile,
		"expires_at":   now.UTC().Add(15 * time.Minute).Truncate(time.Second).Format(time.RFC3339),
		"profile":      *profile,
	})
	if err != nil {
		_, _ = fmt.Fprintln(stderr, "iohr:", err)
		return 1
	}
	_, _ = fmt.Fprintln(stdout, string(out))
	return 0
}
