// `go run . "your task"`: runs the loop with the scripted model and prints the outcome, the
// stop reason and what it used.
package main

import (
	"fmt"
	"os"
	"strings"
)

func main() {
	task := strings.Join(os.Args[1:], " ")
	if task == "" {
		task = "add 2 and 3"
	}
	run := (&Agent{Model: Demo(), Tools: Tools, Allow: Allow, Budget: DefaultBudget(), Log: os.Stderr}).Run(task)
	fmt.Printf("[%s]\n", Stub)
	if run.Text != "" {
		fmt.Println(run.Text)
	} else {
		fmt.Printf("(stopped: %s)\n", run.Stop)
	}
	fmt.Printf("stop=%s steps=%d tool_calls=%d tokens=%d\n", run.Stop, run.Steps, run.ToolCalls, run.Tokens)
	if run.Stop != Done {
		os.Exit(1)
	}
}
