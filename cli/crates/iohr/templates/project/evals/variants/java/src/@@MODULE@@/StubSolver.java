package @@MODULE@@;

import java.util.List;
import java.util.Locale;

/** Keyword rules. Not a model: nothing is learned, nothing is called. */
public final class StubSolver implements Solver {
    /** The answers the suite knows, in the order the rules list them. */
    public static final List<String> LABELS = List.of("billing", "bug", "feature_request", "account", "other");

    private record Rule(String label, List<String> words) {}

    /** First match wins, in this order. */
    private static final List<Rule> RULES = List.of(
            new Rule("billing", List.of("charged", "refund", "invoice", "payment", "card")),
            new Rule("bug", List.of("crash", "error", "blank screen")),
            new Rule("feature_request", List.of("would be great", "add an option", "feature")),
            new Rule("account", List.of("password", "account")));

    @Override
    public String name() {
        return "stub-keyword-rules-v1";
    }

    @Override
    public boolean stub() {
        return true;
    }

    @Override
    public String solve(Case c) {
        String text = c.input().toLowerCase(Locale.ROOT);
        for (Rule rule : RULES) {
            for (String word : rule.words()) {
                if (text.contains(word)) {
                    return rule.label();
                }
            }
        }
        return "other";
    }
}
