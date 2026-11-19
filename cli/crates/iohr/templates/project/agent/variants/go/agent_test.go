// The passes-on-init test: a scripted run within budget; a run that would pass a budget stops
// with a named reason; a tool outside the allow-list is refused and never runs; a tool that
// needs approval does not run without an answer.
package main

import (
	"strings"
	"testing"
)

func ask(t *testing.T, turns []Turn, tools []Tool, allow map[string]bool, b Budget, approve func(ToolCall) *bool, log *strings.Builder) Run {
	t.Helper()
	a := &Agent{Model: NewScriptedModel(turns...), Tools: tools, Allow: allow, Budget: b, Approve: approve}
	if log != nil {
		a.Log = log
	}
	return a.Run("task")
}

func repeat(t Turn, n int) []Turn {
	out := make([]Turn, n)
	for i := range out {
		out[i] = t
	}
	return out
}

func TestAScriptedRunFinishesWithinTheBudget(t *testing.T) {
	run := (&Agent{Model: Demo(), Tools: Tools, Allow: Allow, Budget: DefaultBudget()}).Run("add 2 and 3")
	if run.Stop != Done || run.Text != "2 + 3 = 5" || run.ToolCalls != 1 {
		t.Fatalf("unexpected run: %+v", run)
	}
	if run.Steps > DefaultBudget().MaxSteps {
		t.Fatalf("steps %d over budget", run.Steps)
	}
	if run.History[1].Result != 5 {
		t.Fatalf("tool result: %v", run.History[1].Result)
	}
}

func TestARunThatWouldPassABudgetStopsAndNamesIt(t *testing.T) {
	add := Turn{Call: &ToolCall{Name: "add", Args: map[string]any{"a": 1, "b": 1}}, Tokens: 10}
	if got := ask(t, repeat(add, 50), Tools, Allow, Budget{8, 3, 4000}, nil, nil).Stop; got != BudgetToolCalls {
		t.Fatalf("tool calls: %s", got)
	}
	if got := ask(t, repeat(add, 50), Tools, Allow, Budget{2, 9, 4000}, nil, nil).Stop; got != BudgetSteps {
		t.Fatalf("steps: %s", got)
	}
	heavy := Turn{Call: add.Call, Tokens: 3000}
	run := ask(t, repeat(heavy, 5), Tools, Allow, Budget{8, 9, 4000}, nil, nil)
	if run.Stop != BudgetTokens || run.Tokens > 4000 {
		t.Fatalf("tokens: %+v", run)
	}
}

func TestAToolOutsideTheAllowListIsRefusedAndNeverRuns(t *testing.T) {
	ran := 0
	spy := Tool{Name: "shell", Fn: func(map[string]any) (any, error) { ran++; return nil, nil }}
	run := ask(t,
		[]Turn{{Call: &ToolCall{Name: "shell", Args: map[string]any{"cmd": "rm -rf /"}}}, {Text: "ok"}},
		append(append([]Tool{}, Tools...), spy), Allow, DefaultBudget(), nil, nil)
	if ran != 0 || run.History[1].Err != "not_allowed" || run.Stop != Done || run.ToolCalls != 0 {
		t.Fatalf("ran=%d run=%+v", ran, run)
	}
}

func TestAToolThatNeedsApprovalDoesNotRunWithoutAnAnswer(t *testing.T) {
	ran := 0
	tools := []Tool{{Name: "delete_note", Fn: func(map[string]any) (any, error) { ran++; return nil, nil }, NeedsApproval: true}}
	allow := map[string]bool{"delete_note": true}
	call := Turn{Call: &ToolCall{Name: "delete_note", Args: map[string]any{"id": "n1"}}}
	b := DefaultBudget()
	if got := ask(t, []Turn{call}, tools, allow, b, nil, nil).Stop; got != AwaitingApproval {
		t.Fatalf("no callback: %s", got)
	}
	if got := ask(t, []Turn{call}, tools, allow, b, func(ToolCall) *bool { return nil }, nil).Stop; got != AwaitingApproval {
		t.Fatalf("no answer: %s", got)
	}
	denied := ask(t, []Turn{call, {Text: "ok"}}, tools, allow, b, func(ToolCall) *bool { return No() }, nil)
	if denied.Stop != Done || denied.History[1].Err != "denied" || ran != 0 {
		t.Fatalf("denied: %+v ran=%d", denied, ran)
	}
	yes := ask(t, []Turn{call, {Text: "ok"}}, tools, allow, b, func(ToolCall) *bool { return Yes() }, nil)
	if yes.Stop != Done || ran != 1 {
		t.Fatalf("yes: %+v ran=%d", yes, ran)
	}
}

func TestTheLogHoldsWhatHappenedAndNeverAToolArgument(t *testing.T) {
	var log strings.Builder
	ask(t, []Turn{{Call: &ToolCall{Name: "add", Args: map[string]any{"a": 424242, "b": 1}}}, {Text: "secret-answer"}},
		Tools, Allow, DefaultBudget(), nil, &log)
	s := log.String()
	if !strings.Contains(s, `"event":"tool"`) {
		t.Fatalf("no tool event: %s", s)
	}
	if strings.Contains(s, "424242") || strings.Contains(s, "secret-answer") {
		t.Fatalf("the log holds data: %s", s)
	}
}
