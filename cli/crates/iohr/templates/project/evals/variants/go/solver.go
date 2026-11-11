package main

import "strings"

// labels are the answers the suite knows, in the order the rules list them.
var labels = []string{"billing", "bug", "feature_request", "account", "other"}

// Solver is the thing under test: it turns a case's input into an answer. The one here is a STUB
// (a fixed list of keyword rules), labelled as one everywhere it shows up (`stub: true` in the
// report, a `stub-` name). It exists so the suite runs offline, is free and gives the same answer
// every time. To grade a real model, write a type with these three methods that calls your
// provider, sign in the way your provider's subscription allows and keep the credential in your
// secret store, never in this folder. A solver that fails is counted as an error, never a pass.
type Solver interface {
	Name() string
	Stub() bool
	Solve(c Case) (string, error)
}

// First match wins, in this order.
var rules = []struct {
	label string
	words []string
}{
	{"billing", []string{"charged", "refund", "invoice", "payment", "card"}},
	{"bug", []string{"crash", "error", "blank screen"}},
	{"feature_request", []string{"would be great", "add an option", "feature"}},
	{"account", []string{"password", "account"}},
}

// stubSolver is keyword rules. Not a model: nothing is learned, nothing is called.
type stubSolver struct{}

func (stubSolver) Name() string { return "stub-keyword-rules-v1" }
func (stubSolver) Stub() bool   { return true }

func (stubSolver) Solve(c Case) (string, error) {
	text := strings.ToLower(c.Input)
	for _, r := range rules {
		for _, w := range r.words {
			if strings.Contains(text, w) {
				return r.label, nil
			}
		}
	}
	return "other", nil
}
