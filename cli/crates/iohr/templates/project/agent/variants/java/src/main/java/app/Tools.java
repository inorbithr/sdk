package app;

import java.util.List;
import java.util.Set;

/**
 * Your tools. Two examples: one that is safe to run, one that changes something and so needs
 * an answer first. Replace them; keep {@code needsApproval} on anything that writes, sends or
 * deletes.
 */
public final class Tools {
    private Tools() {}

    public static final List<Agent.Tool> TOOLS = List.of(
            new Agent.Tool("add", args -> (Integer) args.get("a") + (Integer) args.get("b")),
            // An example of a tool that changes something: it only pretends to here.
            new Agent.Tool("delete_note", args -> "deleted " + args.get("id"), true));

    /** Only these may run. A tool the model asks for that is not here is refused. */
    public static final Set<String> ALLOW = Set.of("add");
}
