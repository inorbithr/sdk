package main

import (
	"slices"
	"strings"
)

// A scorer takes the case and the solver's answer and returns 0 or 1. It is a pure function:
// the same case and answer give the same score, with no clock, no randomness and no network, so
// a change in the score is a change in the solver. (A model that grades another model's answer
// is a solver of its own: label it a stub in tests and give it a bar of its own.)
type scorer struct {
	name string
	fn   func(c Case, answer string) int
}

var scorers = []scorer{{"exact", exact}, {"format", formatOK}}

func b2i(b bool) int {
	if b {
		return 1
	}
	return 0
}

// exact: the answer is the expected label, ignoring case and surrounding space.
func exact(c Case, answer string) int {
	return b2i(strings.ToLower(strings.TrimSpace(answer)) == strings.ToLower(strings.TrimSpace(c.Expected)))
}

// formatOK: the answer is one bare label from the list and nothing else (no sentence, no full
// stop). A model that is right but chatty breaks the program that reads its answer, so it fails.
func formatOK(_ Case, answer string) int {
	return b2i(slices.Contains(labels, answer))
}

func scorerNames() []string {
	names := make([]string, len(scorers))
	for i, s := range scorers {
		names[i] = s.name
	}
	return names
}
