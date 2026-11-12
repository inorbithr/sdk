// Your tools. Two examples: one that is safe to run, one that changes something and so needs
// an answer first. Replace them; keep `needsApproval` on anything that writes, sends or deletes.
import 'agent.dart';

final tools = <Tool>[
  Tool('add', (args) {
    final a = args['a'], b = args['b'];
    return a is int && b is int ? '${a + b}' : null;
  }),
  // An example of a tool that changes something: it only pretends to here.
  Tool('delete_note', needsApproval: true, (args) {
    final id = args['id'];
    return id is String ? 'deleted $id' : null;
  }),
];

/// Only these may run. A tool the model asks for that is not here is refused.
const allow = {'add'};
