// A scripted model: it answers from a list, so a run is repeatable and costs nothing. It is a
// stub, labelled as one everywhere it prints. To use a real model, write a class that
// implements IModel and calls your provider; the loop does not change.
namespace App;

public sealed class ScriptedModel(IEnumerable<Turn> turns) : IModel
{
    public const string Stub = "stub: scripted model, no real model was called";

    private readonly Queue<Turn> _turns = new(turns);

    public Turn Next(IReadOnlyList<Message> history) =>
        _turns.TryDequeue(out var turn) ? turn : new Turn(Text: "(the script has ended)");

    public static ScriptedModel Demo() => new(
    [
        new Turn(Call: new ToolCall("add", new Dictionary<string, object> { ["a"] = 2, ["b"] = 3 }), Tokens: 40),
        new Turn(Text: "2 + 3 = 5", Tokens: 20),
    ]);
}
