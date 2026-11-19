package app;

import java.util.Optional;

/** {@code mvn -q compile exec:java}: runs the loop with the scripted model and prints the outcome. */
public final class Main {
    private Main() {}

    public static void main(String[] args) {
        String task = args.length == 0 ? "add 2 and 3" : String.join(" ", args);
        Agent agent = new Agent(
                ScriptedModel.demo(), Tools.TOOLS, Tools.ALLOW, Agent.Budget.defaults(), call -> Optional.empty(),
                System.err::println);
        Agent.Run run = agent.run(task);
        System.out.println("[" + ScriptedModel.STUB + "]");
        System.out.println(run.text() != null ? run.text() : "(stopped: " + run.stop().label + ")");
        System.out.println("stop=" + run.stop().label + " steps=" + run.steps() + " tool_calls=" + run.toolCalls()
                + " tokens=" + run.tokens());
        if (run.stop() != Agent.Stop.DONE) {
            System.exit(1);
        }
    }
}
