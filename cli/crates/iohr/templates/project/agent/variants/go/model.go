package main

// Stub labels every place a scripted model prints, so it never passes for a real one.
const Stub = "stub: scripted model, no real model was called"

// ScriptedModel answers from a list, so a run is repeatable and costs nothing. It is a stub.
// To use a real model, write a type with Next(history) Turn that calls your provider and
// returns its text or tool call and the tokens it used; the loop does not change.
type ScriptedModel struct{ turns []Turn }

func NewScriptedModel(turns ...Turn) *ScriptedModel { return &ScriptedModel{turns: turns} }

func (m *ScriptedModel) Next(history []Message) Turn {
	if len(m.turns) == 0 {
		return Turn{Text: "(the script has ended)"}
	}
	t := m.turns[0]
	m.turns = m.turns[1:]
	return t
}

func Demo() *ScriptedModel {
	return NewScriptedModel(
		Turn{Call: &ToolCall{Name: "add", Args: map[string]any{"a": 2, "b": 3}}, Tokens: 40},
		Turn{Text: "2 + 3 = 5", Tokens: 20},
	)
}
