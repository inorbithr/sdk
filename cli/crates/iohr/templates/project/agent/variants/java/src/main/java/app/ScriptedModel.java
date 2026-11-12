package app;

import java.util.ArrayDeque;
import java.util.Deque;
import java.util.List;
import java.util.Map;

/**
 * A scripted model: it answers from a list, so a run is repeatable and costs nothing. It is a
 * stub, labelled as one everywhere it prints. To use a real model, write a class that
 * implements {@link Agent.Model} and calls your provider; the loop does not change.
 */
public final class ScriptedModel implements Agent.Model {
    public static final String STUB = "stub: scripted model, no real model was called";

    private final Deque<Agent.Turn> turns;

    public ScriptedModel(List<Agent.Turn> turns) {
        this.turns = new ArrayDeque<>(turns);
    }

    @Override
    public Agent.Turn next(List<Agent.Message> history) {
        Agent.Turn turn = turns.pollFirst();
        return turn != null ? turn : Agent.Turn.say("(the script has ended)", 0);
    }

    public static ScriptedModel demo() {
        return new ScriptedModel(List.of(
                Agent.Turn.call("add", Map.of("a", 2, "b", 3), 40), Agent.Turn.say("2 + 3 = 5", 20)));
    }
}
