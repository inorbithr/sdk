// @@NAME@@: a command line with text and JSON output, exit codes that mean something, and a
// version that says which commit it was built from. Standard library only.
package main

import (
	"encoding/json"
	"fmt"
	"io"
	"os"
	"runtime/debug"
	"strings"
)

const version = "0.1.0"

// commit is set by tests; a real build reads it from the version control stamp Go writes
// into every binary built inside a git checkout (`go build`, not `go run`).
var commit = ""

// Exit codes: the contract scripts rely on.
const (
	exitOK      = 0
	exitFailure = 1
	exitUsage   = 2
)

const usage = `Usage: @@NAME@@ [--json] <command>

Commands:
  greet [NAME]   Say hello to NAME (default: world)
  version        Print the version and the commit it was built from

Options:
      --json     Machine-readable output
  -h, --help     Show this help

Exit codes: 0 success, 1 failure, 2 usage error.
`

func buildCommit() string {
	if commit != "" {
		return commit
	}
	if info, ok := debug.ReadBuildInfo(); ok {
		for _, s := range info.Settings {
			if s.Key == "vcs.revision" && len(s.Value) >= 12 {
				return s.Value[:12]
			}
		}
	}
	return "unknown"
}

func main() {
	os.Exit(run(os.Args[1:], os.Stdout, os.Stderr))
}

// run is the whole program, with its streams as arguments so a test can read them.
func run(args []string, stdout, stderr io.Writer) int {
	asJSON := false
	var rest []string
	for _, a := range args {
		switch a {
		case "--json":
			asJSON = true
		case "-h", "--help":
			fmt.Fprint(stdout, usage)
			return exitOK
		default:
			rest = append(rest, a)
		}
	}
	if len(rest) == 0 {
		fmt.Fprint(stderr, usage)
		return exitUsage
	}
	switch rest[0] {
	case "greet":
		return greet(rest[1:], asJSON, stdout, stderr)
	case "version", "--version":
		if len(rest) > 1 {
			return usageError(stderr, "version takes no arguments")
		}
		return printVersion(asJSON, stdout)
	default:
		return usageError(stderr, "unknown command "+quote(rest[0]))
	}
}

func usageError(stderr io.Writer, msg string) int {
	fmt.Fprintf(stderr, "@@NAME@@: %s\nrun `@@NAME@@ --help` for the commands\n", msg)
	return exitUsage
}

// quote keeps a hostile argument from shaping the error line: it is cut and escaped.
func quote(s string) string {
	if len([]rune(s)) > 40 {
		s = string([]rune(s)[:40]) + "..."
	}
	return fmt.Sprintf("%q", s)
}

func emit(w io.Writer, v map[string]string) {
	b, _ := json.Marshal(v) // map keys come out sorted: stable output for scripts
	fmt.Fprintln(w, string(b))
}

func greet(args []string, asJSON bool, stdout, stderr io.Writer) int {
	if len(args) > 1 {
		return usageError(stderr, "greet takes at most one name")
	}
	name := "world"
	if len(args) == 1 {
		name = strings.TrimSpace(args[0])
	}
	if name == "" || len([]rune(name)) > 64 {
		fmt.Fprintln(stderr, "@@NAME@@: the name is 1 to 64 characters of text")
		return exitFailure
	}
	msg := "Hello, " + name
	if asJSON {
		emit(stdout, map[string]string{"message": msg})
	} else {
		fmt.Fprintln(stdout, msg)
	}
	return exitOK
}

func printVersion(asJSON bool, stdout io.Writer) int {
	c := buildCommit()
	if asJSON {
		emit(stdout, map[string]string{"name": "@@NAME@@", "version": version, "commit": c})
	} else {
		fmt.Fprintf(stdout, "@@NAME@@ %s (commit %s)\n", version, c)
	}
	return exitOK
}
