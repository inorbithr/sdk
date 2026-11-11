package @@MODULE@@;

import java.io.PrintStream;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;

/**
 * Grade the solver on the dataset and check the pass bar. Exit 0 when the bar is met, 1 when it is
 * missed, 2 when the dataset or the bar is wrong. The report says which solver ran and whether it
 * is a stub. It holds case ids and scores only; the inputs and answers stay out of it (they can be
 * customer data), unless you ask for --show, which prints the failed cases to your terminal and
 * nowhere else.
 */
public final class Suite {
    private Suite() {}

    private static double round4(double x) {
        return Math.round(x * 1e4) / 1e4;
    }

    /** A number as JSON, always with a point so 1 reads as 1.0. */
    private static String num(double x) {
        return Double.toString(round4(x));
    }

    private static String list(List<String> items) {
        return items.isEmpty() ? "[]" : "[\n" + String.join(",\n", items) + "\n  ]";
    }

    static String reportJson(Runner.Report report, List<String> missed) {
        List<String> scorers = new ArrayList<>();
        for (String n : Scorers.ALL.keySet()) {
            scorers.add("    " + Json.quote(n) + ": " + num(report.mean(n)));
        }
        List<String> results = new ArrayList<>();
        for (Runner.Result r : report.results()) {
            List<String> scores = new ArrayList<>();
            r.scores().forEach((k, v) -> scores.add(Json.quote(k) + ": " + v));
            results.add("    {\n      \"id\": " + Json.quote(r.id()) + ",\n      \"passed\": " + r.passed()
                    + ",\n      \"scores\": {" + String.join(", ", scores) + "},\n      \"error\": "
                    + (r.error().isEmpty() ? "null" : Json.quote(r.error())) + "\n    }");
        }
        List<String> miss = new ArrayList<>();
        for (String m : missed) {
            miss.add("    " + Json.quote(m));
        }
        return "{\n  \"solver\": " + Json.quote(report.solver()) + ",\n  \"stub\": " + report.stub()
                + ",\n  \"cases\": " + report.results().size() + ",\n  \"pass_rate\": " + num(report.passRate())
                + ",\n  \"errors\": " + report.errors() + ",\n  \"scorers\": {\n" + String.join(",\n", scorers)
                + "\n  },\n  \"bar_met\": " + missed.isEmpty() + ",\n  \"missed\": " + list(miss)
                + ",\n  \"results\": " + list(results) + "\n}";
    }

    /** Runs the whole thing and returns the exit code; `solver` is null for the stub. */
    public static int run(String[] args, PrintStream out, PrintStream err, Solver solver) {
        String dataset = Dataset.DEFAULT_DATASET;
        String barPath = Dataset.DEFAULT_BAR;
        boolean asJson = false;
        boolean show = false;
        for (int i = 0; i < args.length; i++) {
            switch (args[i]) {
                case "--dataset", "--bar" -> {
                    if (i + 1 >= args.length) {
                        err.println("usage error: " + args[i] + " needs a value");
                        return 2;
                    }
                    if (args[i].equals("--dataset")) {
                        dataset = args[++i];
                    } else {
                        barPath = args[++i];
                    }
                }
                case "--json" -> asJson = true;
                case "--show" -> show = true;
                default -> {
                    err.println("usage error: unknown argument " + args[i]);
                    return 2;
                }
            }
        }
        if (solver == null) {
            solver = new StubSolver();
        }
        List<Case> cases;
        Bar bar;
        try {
            cases = Dataset.load(Path.of(dataset));
            bar = Bar.load(Path.of(barPath), new ArrayList<>(Scorers.ALL.keySet()));
        } catch (SuiteError e) {
            err.println("suite error: " + e.getMessage());
            return 2;
        }
        Runner.Report report = Runner.evaluate(cases, solver);
        List<String> missed = bar.missed(report);
        if (asJson) {
            out.println(reportJson(report, missed));
        } else {
            String label = report.stub() ? "STUB solver, not a model: " : "";
            out.println(label + report.solver() + ": " + report.passes() + "/" + report.results().size() + " cases pass");
            for (String n : Scorers.ALL.keySet()) {
                out.println(String.format(Locale.ROOT, "  %s: mean %.3f (bar %.3f)", n, report.mean(n), bar.scorers().get(n)));
            }
            out.println(String.format(Locale.ROOT, "  pass rate %.3f (bar %.3f), errors %d (at most %d)",
                    report.passRate(), bar.minPassRate(), report.errors(), bar.maxErrors()));
            for (Runner.Result r : report.results()) {
                if (r.passed()) {
                    continue;
                }
                String why = r.error();
                if (why.isEmpty()) {
                    List<String> zero = new ArrayList<>();
                    for (String n : Scorers.ALL.keySet()) {
                        if (r.scores().get(n) == 0) {
                            zero.add(n);
                        }
                    }
                    why = String.join(", ", zero);
                }
                out.println("  FAILED " + r.id() + ": " + why);
            }
            out.println(missed.isEmpty() ? "BAR MET" : "BAR MISSED: " + String.join("; ", missed));
        }
        if (show) {
            Map<String, Case> byId = new HashMap<>();
            cases.forEach(c -> byId.put(c.id(), c));
            for (Runner.Result r : report.results()) {
                if (!r.passed()) {
                    Case c = byId.get(r.id());
                    err.println("  " + r.id() + ": input=" + Json.quote(c.input()) + " expected=" + Json.quote(c.expected())
                            + " answer=" + Json.quote(r.answer()));
                }
            }
        }
        return missed.isEmpty() ? 0 : 1;
    }
}
