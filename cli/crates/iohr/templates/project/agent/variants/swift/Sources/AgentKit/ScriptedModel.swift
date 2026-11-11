// A scripted model: it answers from a list, so a run is repeatable and costs nothing. It is a
// stub, labelled as one everywhere it prints. To use a real model, write a type that conforms
// to `Model` and calls your provider; the loop does not change.

public let stubLabel = "stub: scripted model, no real model was called"

public struct ScriptedModel: Model {
    private var turns: [Turn]

    public init(_ turns: [Turn]) { self.turns = turns }

    public mutating func next(_ history: [Message]) -> Turn {
        turns.isEmpty ? Turn(text: "(the script has ended)") : turns.removeFirst()
    }

    public static func demo() -> ScriptedModel {
        ScriptedModel([
            Turn(call: ToolCall("add", ["a": .int(2), "b": .int(3)]), tokens: 40),
            Turn(text: "2 + 3 = 5", tokens: 20),
        ])
    }
}
