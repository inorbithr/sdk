// A scripted model: it answers from a list, so a run is repeatable and costs nothing. It is a
// stub, labelled as one everywhere it prints. To use a real model, write a class that
// implements `Model` and calls your provider; the loop does not change.
import 'agent.dart';

const stubLabel = 'stub: scripted model, no real model was called';

class ScriptedModel implements Model {
  ScriptedModel(Iterable<Turn> turns) : _turns = List.of(turns);
  final List<Turn> _turns;

  @override
  Turn next(List<Message> history) =>
      _turns.isEmpty ? const Turn(text: '(the script has ended)') : _turns.removeAt(0);

  factory ScriptedModel.demo() => ScriptedModel(const [
        Turn(call: ToolCall('add', {'a': 2, 'b': 3}), tokens: 40),
        Turn(text: '2 + 3 = 5', tokens: 20),
      ]);
}
