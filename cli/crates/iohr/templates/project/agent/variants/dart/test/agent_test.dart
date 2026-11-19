// The passes-on-init test: a scripted run within budget; a run that would pass a budget stops
// with a named reason; a tool outside the allow-list is refused and never runs; a tool that
// needs approval does not run without an answer.
import 'package:@@MODULE@@/agent.dart';
import 'package:@@MODULE@@/scripted_model.dart';
import 'package:@@MODULE@@/tools.dart' as mine;
import 'package:test/test.dart';

Run ask(
  List<Turn> turns, {
  List<Tool>? tools,
  Set<String>? allow,
  Budget budget = const Budget(),
  bool? Function(ToolCall)? approve,
  void Function(String)? log,
}) =>
    Agent(
      model: ScriptedModel(turns),
      tools: tools ?? mine.tools,
      allow: allow ?? mine.allow,
      budget: budget,
      approve: approve,
      log: log,
    ).run('task');

Turn add(int tokens) => Turn(call: const ToolCall('add', {'a': 1, 'b': 1}), tokens: tokens);

void main() {
  test('a scripted run finishes within the budget', () {
    final run = Agent(model: ScriptedModel.demo(), tools: mine.tools, allow: mine.allow).run('add 2 and 3');
    expect(run.stop, Stop.done);
    expect(run.text, '2 + 3 = 5');
    expect(run.toolCalls, 1);
    expect(run.steps, lessThanOrEqualTo(const Budget().maxSteps));
    expect(run.history[1].content, '5');
  });

  test('a run that would pass a budget stops and names it', () {
    final forever = List.generate(50, (_) => add(10));
    expect(ask(forever, budget: const Budget(maxToolCalls: 3)).stop, Stop.budgetToolCalls);
    expect(ask(forever, budget: const Budget(maxSteps: 2, maxToolCalls: 9)).stop, Stop.budgetSteps);
    final heavy = ask(List.generate(5, (_) => add(3000)), budget: const Budget(maxToolCalls: 9));
    expect(heavy.stop, Stop.budgetTokens);
    expect(heavy.tokens, lessThanOrEqualTo(4000));
  });

  test('a tool outside the allow-list is refused and never runs', () {
    var ran = 0;
    final spy = Tool('shell', (_) {
      ran++;
      return '';
    });
    final run = ask(
      [const Turn(call: ToolCall('shell', {'cmd': 'rm -rf /'})), const Turn(text: 'ok')],
      tools: [...mine.tools, spy],
    );
    expect(ran, 0);
    expect(run.history[1].error, 'not_allowed');
    expect(run.stop, Stop.done);
    expect(run.toolCalls, 0);
  });

  test('a tool that needs approval does not run without an answer', () {
    var ran = 0;
    final tools = [
      Tool('delete_note', needsApproval: true, (_) {
        ran++;
        return '';
      }),
    ];
    const allow = {'delete_note'};
    final script = [const Turn(call: ToolCall('delete_note', {'id': 'n1'})), const Turn(text: 'ok')];
    expect(ask(script, tools: tools, allow: allow).stop, Stop.awaitingApproval);
    expect(ask(script, tools: tools, allow: allow, approve: (_) => null).stop, Stop.awaitingApproval);
    final denied = ask(script, tools: tools, allow: allow, approve: (_) => false);
    expect(denied.stop, Stop.done);
    expect(denied.history[1].error, 'denied');
    expect(ran, 0);
    expect(ask(script, tools: tools, allow: allow, approve: (_) => true).stop, Stop.done);
    expect(ran, 1);
  });

  test('the log holds what happened and never a tool argument', () {
    final lines = <String>[];
    ask(
      [const Turn(call: ToolCall('add', {'a': 424242, 'b': 1})), const Turn(text: 'secret-answer')],
      log: lines.add,
    );
    final text = lines.join('\n');
    expect(text, contains('"event":"tool"'));
    expect(text, isNot(contains('424242')));
    expect(text, isNot(contains('secret-answer')));
  });
}
