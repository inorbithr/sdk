package @@MODULE@@;

import java.util.LinkedHashMap;
import java.util.Locale;
import java.util.Map;
import java.util.function.BiFunction;

/**
 * A scorer takes the case and the solver's answer and returns 0 or 1. It is a pure function: the
 * same case and answer give the same score, with no clock, no randomness and no network, so a
 * change in the score is a change in the solver. (A model that grades another model's answer is a
 * solver of its own: label it a stub in tests and give it a bar of its own.)
 */
public final class Scorers {
    /** Every scorer, by name, in the order the report lists them. */
    public static final Map<String, BiFunction<Case, String, Integer>> ALL = new LinkedHashMap<>();

    static {
        ALL.put("exact", Scorers::exact);
        ALL.put("format", Scorers::format);
    }

    private Scorers() {}

    /** The answer is the expected label, ignoring case and surrounding space. */
    public static int exact(Case c, String answer) {
        return answer.strip().toLowerCase(Locale.ROOT).equals(c.expected().strip().toLowerCase(Locale.ROOT)) ? 1 : 0;
    }

    /**
     * The answer is one bare label from the list and nothing else (no sentence, no full stop). A
     * model that is right but chatty breaks the program that reads its answer, so it fails.
     */
    public static int format(Case c, String answer) {
        return StubSolver.LABELS.contains(answer) ? 1 : 0;
    }
}
