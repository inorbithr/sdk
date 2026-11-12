// `swift run @@NAME@@ "your task"`: runs the loop with the scripted model and prints the
// outcome, the stop reason and what it used.
import AgentKit
import Foundation

let task = CommandLine.arguments.dropFirst().joined(separator: " ")
var agent = Agent(
    model: ScriptedModel.demo(), tools: Tools.all, allow: Tools.allow,
    log: { FileHandle.standardError.write(Data(($0 + "\n").utf8)) })
let run = agent.run(task.isEmpty ? "add 2 and 3" : task)
print("[\(stubLabel)]")
print(run.text ?? "(stopped: \(run.stop.rawValue))")
print("stop=\(run.stop.rawValue) steps=\(run.steps) tool_calls=\(run.toolCalls) tokens=\(run.tokens)")
exit(run.stop == .done ? 0 : 1)
