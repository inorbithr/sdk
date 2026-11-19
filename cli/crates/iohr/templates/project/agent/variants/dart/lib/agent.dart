// The loop. One model, a set of tools, and three limits you can read in one place:
//
// - a budget: steps, tool calls and tokens; a run that would pass one stops and says which;
// - an allow-list: a tool that is not on it is refused, and its code never runs;
// - approval: a tool marked `needsApproval` runs only when someone answers yes.
//
// Every stop has a name (`Run.stop`), so a caller never has to guess why a run ended. The log
// holds what happened (tool names, outcomes, counts), never a tool's arguments or results:
// those can be customer data.
import 'dart:convert';

enum Stop {
  done('done'),
  budgetSteps('budget_steps'),
  budgetToolCalls('budget_tool_calls'),
  budgetTokens('budget_tokens'),
  awaitingApproval('awaiting_approval');

  const Stop(this.label);
  final String label;
}

class Budget {
  const Budget({this.maxSteps = 8, this.maxToolCalls = 4, this.maxTokens = 4000});
  final int maxSteps;
  final int maxToolCalls;
  final int maxTokens;
}

class ToolCall {
  const ToolCall(this.name, [this.args = const {}]);
  final String name;
  final Map<String, Object?> args;
}

/// What the model answered: text (it is done) or a tool call, and the tokens it used.
class Turn {
  const Turn({this.text, this.call, this.tokens = 0});
  final String? text;
  final ToolCall? call;
  final int tokens;
}

/// One entry of the history: a user task, an assistant answer or a tool outcome.
class Message {
  const Message(this.role, {this.name, this.content, this.error});
  final String role;
  final String? name;
  final String? content;
  final String? error;
}

/// Anything with `next(history)`. Wrap your provider here, signed in the way your provider's
/// subscription allows; this template never holds a key.
abstract interface class Model {
  Turn next(List<Message> history);
}

class Tool {
  const Tool(this.name, this.run, {this.needsApproval = false});
  final String name;
  final bool needsApproval;

  /// Returns null when the tool failed.
  final String? Function(Map<String, Object?> args) run;
}

class Run {
  const Run(this.stop, this.text, this.steps, this.toolCalls, this.tokens, this.history);
  final Stop stop;
  final String? text;
  final int steps;
  final int toolCalls;
  final int tokens;
  final List<Message> history;
}

class Agent {
  Agent({
    required this.model,
    required List<Tool> tools,
    required this.allow,
    this.budget = const Budget(),
    this.approve,
    this.log,
  }) : tools = {for (final t in tools) t.name: t};

  final Model model;
  final Map<String, Tool> tools;
  final Set<String> allow;
  final Budget budget;

  /// true runs the tool, false refuses it, null (no answer) stops the run.
  final bool? Function(ToolCall call)? approve;
  final void Function(String line)? log;

  Run run(String task) {
    final history = [Message('user', content: task)];
    var steps = 0, toolCalls = 0, tokens = 0;

    Run stopped(Stop reason, [String? text]) {
      log?.call(jsonEncode({
        'event': 'stop',
        'reason': reason.label,
        'steps': steps,
        'tool_calls': toolCalls,
        'tokens': tokens,
      }));
      return Run(reason, text, steps, toolCalls, tokens, history);
    }

    while (true) {
      if (steps >= budget.maxSteps) return stopped(Stop.budgetSteps);
      final turn = model.next(history);
      steps++;
      if (tokens + turn.tokens > budget.maxTokens) return stopped(Stop.budgetTokens); // this turn is dropped
      tokens += turn.tokens;
      final call = turn.call;
      if (call == null) {
        history.add(Message('assistant', content: turn.text ?? ''));
        return stopped(Stop.done, turn.text ?? '');
      }
      if (toolCalls >= budget.maxToolCalls) return stopped(Stop.budgetToolCalls);
      final shown = call.name.length > 64 ? call.name.substring(0, 64) : call.name;
      final tool = tools[call.name];
      if (tool == null || !allow.contains(call.name)) {
        log?.call(jsonEncode({'event': 'refused', 'tool': shown, 'why': 'not_allowed'}));
        history.add(Message('tool', name: call.name, error: 'not_allowed'));
        continue;
      }
      if (tool.needsApproval) {
        final answer = approve?.call(call);
        if (answer == null) {
          log?.call(jsonEncode({'event': 'approval', 'tool': shown, 'answer': 'none'}));
          return stopped(Stop.awaitingApproval);
        }
        if (!answer) {
          log?.call(jsonEncode({'event': 'approval', 'tool': shown, 'answer': 'no'}));
          history.add(Message('tool', name: call.name, error: 'denied'));
          continue;
        }
        log?.call(jsonEncode({'event': 'approval', 'tool': shown, 'answer': 'yes'}));
      }
      final result = tool.run(call.args);
      toolCalls++;
      log?.call(jsonEncode({'event': 'tool', 'tool': shown, 'ok': result != null}));
      history.add(result != null
          ? Message('tool', name: call.name, content: result)
          : Message('tool', name: call.name, error: 'failed'));
    }
  }
}
