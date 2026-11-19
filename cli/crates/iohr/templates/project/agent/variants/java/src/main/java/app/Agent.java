package app;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.Optional;
import java.util.Set;
import java.util.function.Consumer;
import java.util.function.Function;

/**
 * The loop. One model, a set of tools, and three limits you can read in one place:
 *
 * <ul>
 *   <li>a budget: steps, tool calls and tokens; a run that would pass one stops and says which;
 *   <li>an allow-list: a tool that is not on it is refused, and its code never runs;
 *   <li>approval: a tool marked {@code needsApproval} runs only when someone answers yes.
 * </ul>
 *
 * Every stop has a name ({@link Run#stop()}), so a caller never has to guess why a run ended.
 * The log holds what happened (tool names, outcomes, counts), never a tool's arguments or
 * results: those can be customer data.
 */
public final class Agent {
    /** Why a run stopped. */
    public enum Stop {
        DONE("done"),
        BUDGET_STEPS("budget_steps"),
        BUDGET_TOOL_CALLS("budget_tool_calls"),
        BUDGET_TOKENS("budget_tokens"),
        AWAITING_APPROVAL("awaiting_approval");

        public final String label;

        Stop(String label) {
            this.label = label;
        }
    }

    public record Budget(int maxSteps, int maxToolCalls, int maxTokens) {
        public static Budget defaults() {
            return new Budget(8, 4, 4000);
        }
    }

    public record ToolCall(String name, Map<String, Object> args) {}

    /** What the model answered: text (it is done) or a tool call, and the tokens it used. */
    public record Turn(String text, ToolCall call, int tokens) {
        public static Turn say(String text, int tokens) {
            return new Turn(text, null, tokens);
        }

        public static Turn call(String name, Map<String, Object> args, int tokens) {
            return new Turn(null, new ToolCall(name, args), tokens);
        }
    }

    /**
     * Anything with {@code next(history)}. Wrap your provider here, signed in the way your
     * provider's subscription allows; this template never holds a key.
     */
    public interface Model {
        Turn next(List<Message> history);
    }

    public record Tool(String name, Function<Map<String, Object>, Object> fn, boolean needsApproval) {
        public Tool(String name, Function<Map<String, Object>, Object> fn) {
            this(name, fn, false);
        }
    }

    /** One entry of the history: a user task, an assistant answer or a tool outcome. */
    public record Message(String role, String name, String content, Object result, String error) {}

    public record Run(Stop stop, String text, int steps, int toolCalls, int tokens, List<Message> history) {}

    private final Model model;
    private final Map<String, Tool> tools = new HashMap<>();
    private final Set<String> allow;
    private final Budget budget;
    private final Function<ToolCall, Optional<Boolean>> approve; // empty (no answer) stops the run
    private final Consumer<String> log;

    public Agent(
            Model model,
            List<Tool> tools,
            Set<String> allow,
            Budget budget,
            Function<ToolCall, Optional<Boolean>> approve,
            Consumer<String> log) {
        this.model = model;
        tools.forEach(t -> this.tools.put(t.name(), t));
        this.allow = allow;
        this.budget = budget;
        this.approve = approve;
        this.log = log;
    }

    private void log(String event, String fields) {
        if (log != null) {
            log.accept("{\"event\":\"" + event + "\"" + fields + "}");
        }
    }

    public Run run(String task) {
        List<Message> history = new ArrayList<>();
        history.add(new Message("user", null, task, null, null));
        int steps = 0;
        int toolCalls = 0;
        int tokens = 0;
        while (true) {
            if (steps >= budget.maxSteps()) {
                return stop(Stop.BUDGET_STEPS, null, steps, toolCalls, tokens, history);
            }
            Turn turn = model.next(history);
            steps++;
            if (tokens + turn.tokens() > budget.maxTokens()) {
                // this turn is dropped: it is over the budget
                return stop(Stop.BUDGET_TOKENS, null, steps, toolCalls, tokens, history);
            }
            tokens += turn.tokens();
            if (turn.call() == null) {
                String text = turn.text() == null ? "" : turn.text();
                history.add(new Message("assistant", null, text, null, null));
                return stop(Stop.DONE, text, steps, toolCalls, tokens, history);
            }
            ToolCall call = turn.call();
            if (toolCalls >= budget.maxToolCalls()) {
                return stop(Stop.BUDGET_TOOL_CALLS, null, steps, toolCalls, tokens, history);
            }
            Tool tool = tools.get(call.name());
            String shown = call.name().length() > 64 ? call.name().substring(0, 64) : call.name();
            if (tool == null || !allow.contains(call.name())) {
                log("refused", ",\"tool\":\"" + shown + "\",\"why\":\"not_allowed\"");
                history.add(new Message("tool", call.name(), null, null, "not_allowed"));
                continue;
            }
            if (tool.needsApproval()) {
                Optional<Boolean> answer = approve == null ? Optional.empty() : approve.apply(call);
                if (answer.isEmpty()) {
                    log("approval", ",\"tool\":\"" + shown + "\",\"answer\":\"none\"");
                    return stop(Stop.AWAITING_APPROVAL, null, steps, toolCalls, tokens, history);
                }
                if (!answer.get()) {
                    log("approval", ",\"tool\":\"" + shown + "\",\"answer\":\"no\"");
                    history.add(new Message("tool", call.name(), null, null, "denied"));
                    continue;
                }
                log("approval", ",\"tool\":\"" + shown + "\",\"answer\":\"yes\"");
            }
            Object result = tool.fn().apply(call.args());
            toolCalls++;
            log("tool", ",\"tool\":\"" + shown + "\",\"ok\":true");
            history.add(new Message("tool", call.name(), null, result, null));
        }
    }

    private Run stop(Stop reason, String text, int steps, int toolCalls, int tokens, List<Message> history) {
        log("stop", ",\"reason\":\"" + reason.label + "\",\"steps\":" + steps + ",\"tool_calls\":" + toolCalls
                + ",\"tokens\":" + tokens);
        return new Run(reason, text, steps, toolCalls, tokens, history);
    }
}
