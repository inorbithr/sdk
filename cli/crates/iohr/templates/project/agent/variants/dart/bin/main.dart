// `dart run bin/main.dart "your task"`: runs the loop with the scripted model and prints the
// outcome, the stop reason and what it used.
import 'dart:io';

import 'package:@@MODULE@@/agent.dart';
import 'package:@@MODULE@@/scripted_model.dart';
import 'package:@@MODULE@@/tools.dart';

void main(List<String> args) {
  final task = args.join(' ');
  final run = Agent(
    model: ScriptedModel.demo(),
    tools: tools,
    allow: allow,
    log: stderr.writeln,
  ).run(task.isEmpty ? 'add 2 and 3' : task);
  stdout.writeln('[$stubLabel]');
  stdout.writeln(run.text ?? '(stopped: ${run.stop.label})');
  stdout.writeln('stop=${run.stop.label} steps=${run.steps} tool_calls=${run.toolCalls} tokens=${run.tokens}');
  exit(run.stop == Stop.done ? 0 : 1);
}
