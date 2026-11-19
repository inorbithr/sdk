package main

import "fmt"

// Your tools. Two examples: one that is safe to run, one that changes something and so needs
// an answer first. Replace them; keep NeedsApproval on anything that writes, sends or deletes.

func add(args map[string]any) (any, error) {
	a, aok := args["a"].(int)
	b, bok := args["b"].(int)
	if !aok || !bok {
		return nil, fmt.Errorf("add takes integers a and b")
	}
	return a + b, nil
}

func deleteNote(args map[string]any) (any, error) {
	// An example of a tool that changes something: it only pretends to here.
	return fmt.Sprintf("deleted %v", args["id"]), nil
}

var Tools = []Tool{
	{Name: "add", Fn: add},
	{Name: "delete_note", Fn: deleteNote, NeedsApproval: true},
}

// Only these may run. A tool the model asks for that is not here is refused.
var Allow = map[string]bool{"add": true}
