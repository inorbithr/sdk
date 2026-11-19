// The loop. One model, a set of tools, and three limits you can read in one place:
//
// - a budget: steps, tool calls and tokens; a run that would pass one stops and says which;
// - an allow-list: a tool that is not on it is refused, and its code never runs;
// - approval: a tool marked `needsApproval` runs only when someone answers yes.
//
// Every stop has a name (`Run.stop`), so a caller never has to guess why a run ended. The log
// holds what happened (tool names, outcomes, counts), never a tool's arguments or results:
// those can be customer data.

/// Why a run stopped.
public enum Stop: String, Sendable {
    case done
    case budgetSteps = "budget_steps"
    case budgetToolCalls = "budget_tool_calls"
    case budgetTokens = "budget_tokens"
    case awaitingApproval = "awaiting_approval"
}

public struct Budget: Sendable {
    public var maxSteps = 8
    public var maxToolCalls = 4
    public var maxTokens = 4000
    public init(maxSteps: Int = 8, maxToolCalls: Int = 4, maxTokens: Int = 4000) {
        self.maxSteps = maxSteps
        self.maxToolCalls = maxToolCalls
        self.maxTokens = maxTokens
    }
}

/// A tool argument: a number or a text. Read what you need in the tool.
public enum Arg: Sendable, Equatable {
    case int(Int)
    case string(String)
}

public struct ToolCall: Sendable {
    public let name: String
    public let args: [String: Arg]
    public init(_ name: String, _ args: [String: Arg] = [:]) {
        self.name = name
        self.args = args
    }
}

/// What the model answered: text (it is done) or a tool call, and the tokens it used.
public struct Turn: Sendable {
    public let text: String?
    public let call: ToolCall?
    public let tokens: Int
    public init(text: String? = nil, call: ToolCall? = nil, tokens: Int = 0) {
        self.text = text
        self.call = call
        self.tokens = tokens
    }
}

/// Anything with `next(history)`. Wrap your provider here, signed in the way your provider's
/// subscription allows; this template never holds a key.
public protocol Model {
    mutating func next(_ history: [Message]) -> Turn
}

public struct Tool: Sendable {
    public let name: String
    public let needsApproval: Bool
    public let run: @Sendable ([String: Arg]) -> String?  // nil means the tool failed
    public init(_ name: String, needsApproval: Bool = false, run: @escaping @Sendable ([String: Arg]) -> String?) {
        self.name = name
        self.needsApproval = needsApproval
        self.run = run
    }
}

/// One entry of the history: a user task, an assistant answer or a tool outcome.
public struct Message: Sendable, Equatable {
    public let role: String
    public let name: String?
    public let content: String?
    public let error: String?
    public init(role: String, name: String? = nil, content: String? = nil, error: String? = nil) {
        self.role = role
        self.name = name
        self.content = content
        self.error = error
    }
}

public struct Run {
    public let stop: Stop
    public let text: String?
    public let steps: Int
    public let toolCalls: Int
    public let tokens: Int
    public let history: [Message]
}

public struct Agent {
    var model: any Model
    let tools: [String: Tool]
    let allow: Set<String>
    let budget: Budget
    /// `true` runs the tool, `false` refuses it, `nil` (no answer) stops the run.
    let approve: ((ToolCall) -> Bool?)?
    let log: ((String) -> Void)?

    public init(
        model: any Model, tools: [Tool], allow: Set<String>, budget: Budget = Budget(),
        approve: ((ToolCall) -> Bool?)? = nil, log: ((String) -> Void)? = nil
    ) {
        self.model = model
        self.tools = Dictionary(uniqueKeysWithValues: tools.map { ($0.name, $0) })
        self.allow = allow
        self.budget = budget
        self.approve = approve
        self.log = log
    }

    public mutating func run(_ task: String) -> Run {
        var history = [Message(role: "user", content: task)]
        var steps = 0
        var toolCalls = 0
        var tokens = 0

        func stopped(_ reason: Stop, _ text: String? = nil) -> Run {
            log?(#"{"event":"stop","reason":"\#(reason.rawValue)","steps":\#(steps),"tool_calls":\#(toolCalls),"tokens":\#(tokens)}"#)
            return Run(stop: reason, text: text, steps: steps, toolCalls: toolCalls, tokens: tokens, history: history)
        }

        while true {
            if steps >= budget.maxSteps { return stopped(.budgetSteps) }
            let turn = model.next(history)
            steps += 1
            if tokens + turn.tokens > budget.maxTokens { return stopped(.budgetTokens) }  // this turn is dropped
            tokens += turn.tokens
            guard let call = turn.call else {
                history.append(Message(role: "assistant", content: turn.text ?? ""))
                return stopped(.done, turn.text ?? "")
            }
            if toolCalls >= budget.maxToolCalls { return stopped(.budgetToolCalls) }
            let shown = String(call.name.prefix(64))
            guard let tool = tools[call.name], allow.contains(call.name) else {
                log?(#"{"event":"refused","tool":"\#(shown)","why":"not_allowed"}"#)
                history.append(Message(role: "tool", name: call.name, error: "not_allowed"))
                continue
            }
            if tool.needsApproval {
                guard let answer = approve?(call) else {
                    log?(#"{"event":"approval","tool":"\#(shown)","answer":"none"}"#)
                    return stopped(.awaitingApproval)
                }
                if !answer {
                    log?(#"{"event":"approval","tool":"\#(shown)","answer":"no"}"#)
                    history.append(Message(role: "tool", name: call.name, error: "denied"))
                    continue
                }
                log?(#"{"event":"approval","tool":"\#(shown)","answer":"yes"}"#)
            }
            let result = tool.run(call.args)
            toolCalls += 1
            log?(#"{"event":"tool","tool":"\#(shown)","ok":\#(result != nil)}"#)
            history.append(
                result.map { Message(role: "tool", name: call.name, content: $0) }
                    ?? Message(role: "tool", name: call.name, error: "failed"))
        }
    }
}
