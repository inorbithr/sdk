// Your tools. Two examples: one that is safe to run, one that changes something and so needs
// an answer first. Replace them; keep NeedsApproval on anything that writes, sends or deletes.
namespace App;

public static class Tools
{
    public static IReadOnlyList<Tool> All { get; } =
    [
        new Tool("add", args => (int)args["a"] + (int)args["b"]),
        // An example of a tool that changes something: it only pretends to here.
        new Tool("delete_note", args => $"deleted {args["id"]}", NeedsApproval: true),
    ];

    /// <summary>Only these may run. A tool the model asks for that is not here is refused.</summary>
    public static ISet<string> Allow { get; } = new HashSet<string> { "add" };
}
