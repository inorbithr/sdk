package app;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.util.ArrayList;
import java.util.Collections;
import java.util.List;
import java.util.Map;
import java.util.Optional;
import java.util.Set;
import java.util.function.Function;
import org.junit.jupiter.api.Test;

/**
 * The passes-on-init test: a scripted run within budget; a run that would pass a budget stops
 * with a named reason; a tool outside the allow-list is refused and never runs; a tool that
 * needs approval does not run without an answer.
 */
class AgentTest {
    private static Agent.Run ask(
            List<Agent.Turn> turns,
            List<Agent.Tool> tools,
            Set<String> allow,
            Agent.Budget budget,
            Function<Agent.ToolCall, Optional<Boolean>> approve,
            List<String> log) {
        return new Agent(new ScriptedModel(turns), tools, allow, budget, approve, log == null ? null : log::add)
                .run("task");
    }

    private static Agent.Turn add(int tokens) {
        return Agent.Turn.call("add", Map.of("a", 1, "b", 1), tokens);
    }

    @Test
    void aScriptedRunFinishesWithinTheBudget() {
        Agent.Run run = new Agent(ScriptedModel.demo(), Tools.TOOLS, Tools.ALLOW, Agent.Budget.defaults(), null, null)
                .run("add 2 and 3");
        assertEquals(Agent.Stop.DONE, run.stop());
        assertEquals("2 + 3 = 5", run.text());
        assertEquals(1, run.toolCalls());
        assertTrue(run.steps() <= Agent.Budget.defaults().maxSteps());
        assertEquals(5, run.history().get(1).result());
    }

    @Test
    void aRunThatWouldPassABudgetStopsAndNamesIt() {
        List<Agent.Turn> forever = Collections.nCopies(50, add(10));
        assertEquals(
                Agent.Stop.BUDGET_TOOL_CALLS,
                ask(forever, Tools.TOOLS, Tools.ALLOW, new Agent.Budget(8, 3, 4000), null, null).stop());
        assertEquals(
                Agent.Stop.BUDGET_STEPS,
                ask(forever, Tools.TOOLS, Tools.ALLOW, new Agent.Budget(2, 9, 4000), null, null).stop());
        Agent.Run heavy = ask(
                Collections.nCopies(5, add(3000)), Tools.TOOLS, Tools.ALLOW, new Agent.Budget(8, 9, 4000), null, null);
        assertEquals(Agent.Stop.BUDGET_TOKENS, heavy.stop());
        assertTrue(heavy.tokens() <= 4000);
    }

    @Test
    void aToolOutsideTheAllowListIsRefusedAndNeverRuns() {
        List<Object> ran = new ArrayList<>();
        List<Agent.Tool> tools = new ArrayList<>(Tools.TOOLS);
        tools.add(new Agent.Tool("shell", args -> ran.add(args)));
        Agent.Run run = ask(
                List.of(Agent.Turn.call("shell", Map.of("cmd", "rm -rf /"), 0), Agent.Turn.say("ok", 0)),
                tools, Tools.ALLOW, Agent.Budget.defaults(), null, null);
        assertTrue(ran.isEmpty());
        assertEquals("not_allowed", run.history().get(1).error());
        assertEquals(Agent.Stop.DONE, run.stop());
        assertEquals(0, run.toolCalls());
    }

    @Test
    void aToolThatNeedsApprovalDoesNotRunWithoutAnAnswer() {
        List<Object> ran = new ArrayList<>();
        List<Agent.Tool> tools = List.of(new Agent.Tool("delete_note", args -> ran.add(args), true));
        Set<String> allow = Set.of("delete_note");
        List<Agent.Turn> script =
                List.of(Agent.Turn.call("delete_note", Map.of("id", "n1"), 0), Agent.Turn.say("ok", 0));
        Agent.Budget b = Agent.Budget.defaults();
        assertEquals(Agent.Stop.AWAITING_APPROVAL, ask(script, tools, allow, b, null, null).stop());
        assertEquals(Agent.Stop.AWAITING_APPROVAL, ask(script, tools, allow, b, c -> Optional.empty(), null).stop());
        Agent.Run denied = ask(script, tools, allow, b, c -> Optional.of(false), null);
        assertEquals(Agent.Stop.DONE, denied.stop());
        assertEquals("denied", denied.history().get(1).error());
        assertTrue(ran.isEmpty());
        Agent.Run yes = ask(script, tools, allow, b, c -> Optional.of(true), null);
        assertEquals(Agent.Stop.DONE, yes.stop());
        assertEquals(1, ran.size());
    }

    @Test
    void theLogHoldsWhatHappenedAndNeverAToolArgument() {
        List<String> log = new ArrayList<>();
        ask(List.of(Agent.Turn.call("add", Map.of("a", 424242, "b", 1), 0), Agent.Turn.say("secret-answer", 0)),
                Tools.TOOLS, Tools.ALLOW, Agent.Budget.defaults(), null, log);
        String text = String.join("\n", log);
        assertTrue(text.contains("\"event\":\"tool\""));
        assertFalse(text.contains("424242"));
        assertFalse(text.contains("secret-answer"));
    }
}
