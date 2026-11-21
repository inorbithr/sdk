package @@MODULE@@;

import java.util.LinkedHashMap;
import java.util.List;
import java.util.ArrayList;
import java.util.Map;

/** The runner: the solver over the cases, every answer scored. */
public final class Runner {
    private Runner() {}

    /** One case, graded. `error` is empty unless the solver failed. */
    public record Result(String id, String answer, Map<String, Integer> scores, String error) {
        public boolean passed() {
            return error.isEmpty() && scores.values().stream().allMatch(s -> s == 1);
        }
    }

    /** Every result and which solver made them. */
    public record Report(String solver, boolean stub, List<Result> results) {
        public long passes() {
            return results.stream().filter(Result::passed).count();
        }

        public double passRate() {
            return (double) passes() / results.size();
        }

        public long errors() {
            return results.stream().filter(r -> !r.error().isEmpty()).count();
        }

        public double mean(String scorer) {
            long sum = 0;
            for (Result r : results) {
                sum += r.scores().getOrDefault(scorer, 0);
            }
            return (double) sum / results.size();
        }
    }

    /**
     * Runs the solver over the cases and scores each answer. A solver that fails is an error case
     * with every score 0, never a pass. The error kept is the exception's type, never its message:
     * a provider's message can echo the input.
     */
    public static Report evaluate(List<Case> cases, Solver solver) {
        List<Result> results = new ArrayList<>();
        for (Case c : cases) {
            String answer = "";
            String error = "";
            try {
                answer = solver.solve(c);
            } catch (Throwable t) {
                error = t.getClass().getSimpleName();
            }
            Map<String, Integer> scores = new LinkedHashMap<>();
            for (Map.Entry<String, java.util.function.BiFunction<Case, String, Integer>> s : Scorers.ALL.entrySet()) {
                scores.put(s.getKey(), error.isEmpty() ? s.getValue().apply(c, answer) : 0);
            }
            results.add(new Result(c.id(), answer, scores, error));
        }
        return new Report(solver.name(), solver.stub(), results);
    }
}
