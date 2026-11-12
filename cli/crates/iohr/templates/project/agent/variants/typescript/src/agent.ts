// The loop. One model, a set of tools, and three limits you can read in one place:
//
// - a budget: steps, tool calls and tokens; a run that would pass one stops and says which;
// - an allow-list: a tool that is not on it is refused, and its code never runs;
// - approval: a tool marked `needsApproval` runs only when someone answers yes.
//
// Every stop has a name (`Run.stop`), so a caller never has to guess why a run ended. The log
// holds what happened (tool names, outcomes, counts), never a tool's arguments or results:
// those can be customer data.

export type Stop = "done" | "budget_steps" | "budget_tool_calls" | "budget_tokens" | "awaiting_approval";

export interface Budget {
  maxSteps: number;
  maxToolCalls: number;
  maxTokens: number;
}
export const DEFAULT_BUDGET: Budget = { maxSteps: 8, maxToolCalls: 4, maxTokens: 4000 };

export interface ToolCall {
  name: string;
  args: Record<string, unknown>;
}

/** What the model answered: text (it is done) or a tool call, and the tokens it used. */
export interface Turn {
  text?: string;
  call?: ToolCall;
  tokens?: number;
}

/**
 * Anything with `next(history)`. Wrap your provider here, signed in the way your provider's
 * subscription allows; this template never holds a key.
 */
export interface Model {
  next(history: Message[]): Turn;
}

export interface Tool {
  name: string;
  fn: (args: Record<string, unknown>) => unknown;
  needsApproval?: boolean;
}

export type Message =
  | { role: "user" | "assistant"; content: string }
  | { role: "tool"; name: string; result?: unknown; error?: "not_allowed" | "denied" };

export interface Run {
  stop: Stop;
  text: string | undefined;
  steps: number;
  toolCalls: number;
  tokens: number;
  history: Message[];
}

export interface Options {
  model: Model;
  tools: Tool[];
  allow: Set<string>;
  budget?: Budget;
  /** true runs the tool, false refuses it, undefined (no answer) stops the run. */
  approve?: (call: ToolCall) => boolean | undefined;
  log?: { write(line: string): unknown };
}

export function runAgent(task: string, o: Options): Run {
  const budget = o.budget ?? DEFAULT_BUDGET;
  const tools = new Map(o.tools.map((t) => [t.name, t]));
  const history: Message[] = [{ role: "user", content: task }];
  let steps = 0;
  let toolCalls = 0;
  let tokens = 0;
  const log = (event: string, fields: Record<string, unknown> = {}) =>
    o.log?.write(JSON.stringify({ event, ...fields }) + "\n");
  const stop = (reason: Stop, text?: string): Run => {
    log("stop", { reason, steps, tool_calls: toolCalls, tokens });
    return { stop: reason, text, steps, toolCalls, tokens, history };
  };

  for (;;) {
    if (steps >= budget.maxSteps) return stop("budget_steps");
    const turn = o.model.next(history);
    steps += 1;
    const used = turn.tokens ?? 0;
    if (tokens + used > budget.maxTokens) return stop("budget_tokens"); // this turn is dropped
    tokens += used;
    if (!turn.call) {
      history.push({ role: "assistant", content: turn.text ?? "" });
      return stop("done", turn.text);
    }
    const call = turn.call;
    if (toolCalls >= budget.maxToolCalls) return stop("budget_tool_calls");
    const tool = tools.get(call.name);
    if (!tool || !o.allow.has(call.name)) {
      log("refused", { tool: call.name.slice(0, 64), why: "not_allowed" });
      history.push({ role: "tool", name: call.name, error: "not_allowed" });
      continue;
    }
    if (tool.needsApproval) {
      const answer = o.approve?.(call);
      if (answer === undefined) {
        log("approval", { tool: call.name, answer: "none" });
        return stop("awaiting_approval");
      }
      if (!answer) {
        log("approval", { tool: call.name, answer: "no" });
        history.push({ role: "tool", name: call.name, error: "denied" });
        continue;
      }
      log("approval", { tool: call.name, answer: "yes" });
    }
    const result = tool.fn(call.args);
    toolCalls += 1;
    log("tool", { tool: call.name, ok: true });
    history.push({ role: "tool", name: call.name, result });
  }
}
