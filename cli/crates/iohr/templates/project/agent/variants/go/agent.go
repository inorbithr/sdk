// The loop. One model, a set of tools, and three limits you can read in one place:
//
//   - a budget: steps, tool calls and tokens; a run that would pass one stops and says which;
//   - an allow-list: a tool that is not on it is refused, and its code never runs;
//   - approval: a tool marked NeedsApproval runs only when someone answers yes.
//
// Every stop has a name (Run.Stop), so a caller never has to guess why a run ended. The log
// holds what happened (tool names, outcomes, counts), never a tool's arguments or results: those
// can be customer data.
package main

import (
	"encoding/json"
	"fmt"
	"io"
)

// Why a run stopped.
const (
	Done             = "done"
	BudgetSteps      = "budget_steps"
	BudgetToolCalls  = "budget_tool_calls"
	BudgetTokens     = "budget_tokens"
	AwaitingApproval = "awaiting_approval"
)

type Budget struct {
	MaxSteps     int
	MaxToolCalls int
	MaxTokens    int
}

func DefaultBudget() Budget { return Budget{MaxSteps: 8, MaxToolCalls: 4, MaxTokens: 4000} }

type ToolCall struct {
	Name string
	Args map[string]any
}

// Turn is what the model answered: text (it is done) or a tool call, and the tokens it used.
type Turn struct {
	Text   string
	Call   *ToolCall
	Tokens int
}

// Model is anything with Next(history) Turn. Wrap your provider here, signed in the way your
// provider's subscription allows; this template never holds a key.
type Model interface {
	Next(history []Message) Turn
}

type Tool struct {
	Name          string
	Fn            func(args map[string]any) (any, error)
	NeedsApproval bool
}

// Message is one entry of the history: a user task, an assistant answer or a tool outcome.
type Message struct {
	Role    string
	Name    string
	Content string
	Result  any
	Err     string
}

type Run struct {
	Stop      string
	Text      string
	Steps     int
	ToolCalls int
	Tokens    int
	History   []Message
}

type Agent struct {
	Model  Model
	Tools  []Tool
	Allow  map[string]bool
	Budget Budget
	// Approve returns true to run the tool, false to refuse it, nil (no answer) to stop the run.
	Approve func(ToolCall) *bool
	Log     io.Writer
}

func (a *Agent) logf(event string, fields map[string]any) {
	if a.Log == nil {
		return
	}
	fields["event"] = event
	b, _ := json.Marshal(fields) // encoding/json sorts map keys
	fmt.Fprintln(a.Log, string(b))
}

func (a *Agent) Run(task string) Run {
	history := []Message{{Role: "user", Content: task}}
	steps, toolCalls, tokens := 0, 0, 0
	stop := func(reason, text string) Run {
		a.logf("stop", map[string]any{"reason": reason, "steps": steps, "tool_calls": toolCalls, "tokens": tokens})
		return Run{reason, text, steps, toolCalls, tokens, history}
	}
	tools := map[string]Tool{}
	for _, t := range a.Tools {
		tools[t.Name] = t
	}
	for {
		if steps >= a.Budget.MaxSteps {
			return stop(BudgetSteps, "")
		}
		turn := a.Model.Next(history)
		steps++
		if tokens+turn.Tokens > a.Budget.MaxTokens {
			return stop(BudgetTokens, "") // this turn is dropped: it is over the budget
		}
		tokens += turn.Tokens
		if turn.Call == nil {
			history = append(history, Message{Role: "assistant", Content: turn.Text})
			return stop(Done, turn.Text)
		}
		call := *turn.Call
		if toolCalls >= a.Budget.MaxToolCalls {
			return stop(BudgetToolCalls, "")
		}
		tool, known := tools[call.Name]
		if !known || !a.Allow[call.Name] {
			a.logf("refused", map[string]any{"tool": truncate(call.Name, 64), "why": "not_allowed"})
			history = append(history, Message{Role: "tool", Name: call.Name, Err: "not_allowed"})
			continue
		}
		if tool.NeedsApproval {
			var answer *bool
			if a.Approve != nil {
				answer = a.Approve(call)
			}
			if answer == nil {
				a.logf("approval", map[string]any{"tool": call.Name, "answer": "none"})
				return stop(AwaitingApproval, "")
			}
			if !*answer {
				a.logf("approval", map[string]any{"tool": call.Name, "answer": "no"})
				history = append(history, Message{Role: "tool", Name: call.Name, Err: "denied"})
				continue
			}
			a.logf("approval", map[string]any{"tool": call.Name, "answer": "yes"})
		}
		result, err := tool.Fn(call.Args)
		toolCalls++
		if err != nil {
			a.logf("tool", map[string]any{"tool": call.Name, "ok": false})
			history = append(history, Message{Role: "tool", Name: call.Name, Err: "failed"})
			continue
		}
		a.logf("tool", map[string]any{"tool": call.Name, "ok": true})
		history = append(history, Message{Role: "tool", Name: call.Name, Result: result})
	}
}

func truncate(s string, n int) string {
	if len(s) > n {
		return s[:n]
	}
	return s
}

// Yes and No are the answers an Approve callback returns; nil is "no answer".
func Yes() *bool { t := true; return &t }
func No() *bool  { f := false; return &f }
