package main

import (
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

const (
	defaultDataset = "datasets/tickets.jsonl"
	defaultBar     = "thresholds.toml"
)

// suiteError means the dataset or the bar is wrong. The run stops and exits 2: nothing was graded.
type suiteError struct{ msg string }

func (e *suiteError) Error() string { return e.msg }

func suiteErrorf(format string, args ...any) error {
	return &suiteError{fmt.Sprintf(format, args...)}
}

// Case is one line of the dataset: an id, an input and the expected answer.
type Case struct {
	ID       string
	Input    string
	Expected string
}

// loadCases reads the dataset, one JSON object per line. Loading is strict, so a broken dataset
// fails loudly instead of quietly scoring fewer cases.
func loadCases(path string) ([]Case, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		var pe *os.PathError
		reason := err.Error()
		if errors.As(err, &pe) {
			reason = pe.Err.Error()
		}
		return nil, suiteErrorf("cannot read the dataset %s: %s", path, reason)
	}
	name := filepath.Base(path)
	var cases []Case
	seen := map[string]bool{}
	for i, line := range strings.Split(string(data), "\n") {
		number := i + 1
		if strings.TrimSpace(line) == "" {
			continue
		}
		var raw any
		if err := json.Unmarshal([]byte(line), &raw); err != nil {
			return nil, suiteErrorf("%s line %d: not JSON (%v)", name, number, err)
		}
		row, ok := raw.(map[string]any)
		if !ok {
			return nil, suiteErrorf("%s line %d: expected an object", name, number)
		}
		field := map[string]string{}
		for _, key := range []string{"id", "input", "expected"} {
			s, ok := row[key].(string)
			if !ok || strings.TrimSpace(s) == "" {
				return nil, suiteErrorf("%s line %d: `%s` must be a non-empty string", name, number, key)
			}
			field[key] = s
		}
		if seen[field["id"]] {
			return nil, suiteErrorf("%s line %d: the id `%s` is used twice", name, number, field["id"])
		}
		seen[field["id"]] = true
		cases = append(cases, Case{field["id"], field["input"], field["expected"]})
	}
	if len(cases) == 0 {
		return nil, suiteErrorf("%s has no cases: an empty suite passes nothing", name)
	}
	return cases, nil
}
