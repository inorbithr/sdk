// Your tools. Two examples: one that is safe to run, one that changes something and so needs
// an answer first. Replace them; keep `needsApproval` on anything that writes, sends or deletes.

public enum Tools {
    public static let all: [Tool] = [
        Tool("add") { args in
            guard case .int(let a)? = args["a"], case .int(let b)? = args["b"] else { return nil }
            return String(a + b)
        },
        // An example of a tool that changes something: it only pretends to here.
        Tool("delete_note", needsApproval: true) { args in
            guard case .string(let id)? = args["id"] else { return nil }
            return "deleted \(id)"
        },
    ]

    /// Only these may run. A tool the model asks for that is not here is refused.
    public static let allow: Set<String> = ["add"]
}
