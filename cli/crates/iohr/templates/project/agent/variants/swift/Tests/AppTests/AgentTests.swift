// The passes-on-init test: a scripted run within budget; a run that would pass a budget stops
// with a named reason; a tool outside the allow-list is refused and never runs; a tool that
// needs approval does not run without an answer.
import Testing

@testable import AgentKit

private final class Counter: @unchecked Sendable { var n = 0 }

private func ask(
    _ turns: [Turn], tools: [Tool] = Tools.all, allow: Set<String> = Tools.allow,
    budget: Budget = Budget(), approve: ((ToolCall) -> Bool?)? = nil, log: ((String) -> Void)? = nil
) -> Run {
    var agent = Agent(model: ScriptedModel(turns), tools: tools, allow: allow, budget: budget, approve: approve, log: log)
    return agent.run("task")
}

private func add(_ tokens: Int) -> Turn { Turn(call: ToolCall("add", ["a": .int(1), "b": .int(1)]), tokens: tokens) }

@Suite struct AgentTests {
    @Test func aScriptedRunFinishesWithinTheBudget() {
        var agent = Agent(model: ScriptedModel.demo(), tools: Tools.all, allow: Tools.allow)
        let run = agent.run("add 2 and 3")
        #expect(run.stop == .done)
        #expect(run.text == "2 + 3 = 5")
        #expect(run.toolCalls == 1)
        #expect(run.steps <= Budget().maxSteps)
        #expect(run.history[1].content == "5")
    }

    @Test func aRunThatWouldPassABudgetStopsAndNamesIt() {
        let forever = Array(repeating: add(10), count: 50)
        #expect(ask(forever, budget: Budget(maxSteps: 8, maxToolCalls: 3, maxTokens: 4000)).stop == .budgetToolCalls)
        #expect(ask(forever, budget: Budget(maxSteps: 2, maxToolCalls: 9, maxTokens: 4000)).stop == .budgetSteps)
        let heavy = ask(Array(repeating: add(3000), count: 5), budget: Budget(maxSteps: 8, maxToolCalls: 9, maxTokens: 4000))
        #expect(heavy.stop == .budgetTokens)
        #expect(heavy.tokens <= 4000)
    }

    @Test func aToolOutsideTheAllowListIsRefusedAndNeverRuns() {
        let ran = Counter()
        let spy = Tool("shell") { _ in ran.n += 1; return "" }
        let run = ask(
            [Turn(call: ToolCall("shell", ["cmd": .string("rm -rf /")])), Turn(text: "ok")], tools: Tools.all + [spy])
        #expect(ran.n == 0)
        #expect(run.history[1].error == "not_allowed")
        #expect(run.stop == .done)
        #expect(run.toolCalls == 0)
    }

    @Test func aToolThatNeedsApprovalDoesNotRunWithoutAnAnswer() {
        let ran = Counter()
        let tools = [Tool("delete_note", needsApproval: true) { _ in ran.n += 1; return "" }]
        let allow: Set<String> = ["delete_note"]
        let script = [Turn(call: ToolCall("delete_note", ["id": .string("n1")])), Turn(text: "ok")]
        #expect(ask(script, tools: tools, allow: allow).stop == .awaitingApproval)
        #expect(ask(script, tools: tools, allow: allow, approve: { _ in nil }).stop == .awaitingApproval)
        let denied = ask(script, tools: tools, allow: allow, approve: { _ in false })
        #expect(denied.stop == .done)
        #expect(denied.history[1].error == "denied")
        #expect(ran.n == 0)
        let yes = ask(script, tools: tools, allow: allow, approve: { _ in true })
        #expect(yes.stop == .done)
        #expect(ran.n == 1)
    }

    @Test func theLogHoldsWhatHappenedAndNeverAToolArgument() {
        let lines = Lines()
        _ = ask(
            [Turn(call: ToolCall("add", ["a": .int(424242), "b": .int(1)])), Turn(text: "secret-answer")],
            log: { lines.items.append($0) })
        let text = lines.items.joined(separator: "\n")
        #expect(text.contains(#""event":"tool""#))
        #expect(!text.contains("424242"))
        #expect(!text.contains("secret-answer"))
    }
}

private final class Lines: @unchecked Sendable { var items: [String] = [] }
