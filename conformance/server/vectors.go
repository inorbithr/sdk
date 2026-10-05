package main

import (
	"encoding/json"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"sort"

	"go.yaml.in/yaml/v3"
)

// isVectors tells whether the binary was started as `replay vectors DIR`.
func isVectors(args []string) bool {
	return len(args) >= 1 && args[0] == "vectors"
}

// dumpVectors prints every vector under DIR (conformance/vectors/<kind>/<name>.yaml) as
// one JSON document, {"vectors": [...]} sorted by path, so a driver without a YAML reader
// can run them (docs/config.md section 9.2). It returns the exit code.
func dumpVectors(args []string, stdout, stderr io.Writer) int {
	if len(args) != 2 {
		_, _ = fmt.Fprintln(stderr, "replay: usage: replay vectors DIR")
		return 2
	}
	paths, err := filepath.Glob(filepath.Join(args[1], "*", "*.yaml"))
	if err != nil || len(paths) == 0 {
		_, _ = fmt.Fprintf(stderr, "replay: no vectors under %s\n", args[1])
		return 1
	}
	sort.Strings(paths)
	out := make([]any, 0, len(paths))
	for _, p := range paths {
		data, err := os.ReadFile(p) //nolint:gosec // the repository's own vectors
		if err != nil {
			_, _ = fmt.Fprintln(stderr, "replay:", err)
			return 1
		}
		var v any
		if err := yaml.Unmarshal(data, &v); err != nil {
			_, _ = fmt.Fprintf(stderr, "replay: %s: %v\n", p, err)
			return 1
		}
		out = append(out, v)
	}
	if err := json.NewEncoder(stdout).Encode(map[string]any{"vectors": out}); err != nil {
		_, _ = fmt.Fprintln(stderr, "replay:", err)
		return 1
	}
	return 0
}
