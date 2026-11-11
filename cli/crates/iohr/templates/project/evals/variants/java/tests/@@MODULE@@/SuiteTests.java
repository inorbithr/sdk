package @@MODULE@@;

import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.PrintStream;
import java.lang.reflect.Method;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.HashSet;
import java.util.List;
import java.util.Map;

/**
 * The passes-on-init test: the fixture suite meets its bar offline, the bar bites when an answer
 * is wrong, and the harness refuses a broken dataset or bar instead of grading less.
 *
 * <p>Plain JDK on purpose: a test runner (JUnit) is a download and a build tool, and this project
 * has neither. Every method named test* runs; a failed assertion fails the run (exit 1). Move to
 * JUnit when you add a build tool; the bodies are ordinary assertions and carry over.
 */
public final class SuiteTests {
    private record Out(int code, String out, String err) {}

    private static Out run(String... args) {
        return run(null, args);
    }

    private static Out run(Solver solver, String... args) {
        ByteArrayOutputStream o = new ByteArrayOutputStream();
        ByteArrayOutputStream e = new ByteArrayOutputStream();
        int code = Suite.run(args, new PrintStream(o, true, StandardCharsets.UTF_8),
                new PrintStream(e, true, StandardCharsets.UTF_8), solver);
        return new Out(code, o.toString(StandardCharsets.UTF_8), e.toString(StandardCharsets.UTF_8));
    }

    private static void check(boolean ok, String what) {
        if (!ok) {
            throw new AssertionError(what);
        }
    }

    private static void eq(Object want, Object got, String what) {
        check(want.equals(got), what + ": want " + want + ", got " + got);
    }

    private static Path tmp(String name, String text) throws IOException {
        Path dir = Files.createTempDirectory("evals-");
        Path file = dir.resolve(name);
        Files.writeString(file, text);
        return file;
    }

    private static List<Case> cases() throws SuiteError {
        return Dataset.load(Path.of(Dataset.DEFAULT_DATASET));
    }

    private static Bar bar() throws SuiteError {
        return Bar.load(Path.of(Dataset.DEFAULT_BAR), new ArrayList<>(Scorers.ALL.keySet()));
    }

    public void testTheFixtureMeetsItsBarWithNoNetwork() {
        // Nothing here can reach a network: the project has no dependency and the stub is local rules.
        Out r = run();
        eq(0, r.code(), r.out());
        check(r.out().contains("BAR MET"), r.out());
    }

    public void testTheDatasetIsADozenLabelledCasesCoveringEveryLabel() throws Exception {
        List<Case> cases = cases();
        eq(12, cases.size(), "cases");
        check(new HashSet<>(cases.stream().map(Case::expected).toList()).containsAll(StubSolver.LABELS), "a label has no case");
    }

    public void testTheReportSaysStubAndHoldsIdsAndScoresNeverInputs() throws Exception {
        Out r = run("--json");
        eq(0, r.code(), "exit");
        check(r.out().contains("\"stub\": true"), r.out());
        check(r.out().contains("\"solver\": \"stub-"), r.out());
        check(r.out().contains("\"bar_met\": true"), r.out());
        check(r.out().contains("\"pass_rate\": 1.0"), r.out());
        for (Case c : cases()) {
            check(!r.out().contains(c.input()), c.input());
        }
    }

    public void testTwoRunsGiveTheSameReport() {
        eq(run("--json"), run("--json"), "two runs");
    }

    public void testLoweringOneExpectedAnswerFailsTheSuiteAndNamesTheCase() throws Exception {
        List<String> lines = new ArrayList<>(Files.readAllLines(Path.of(Dataset.DEFAULT_DATASET)));
        String first = lines.get(0).replace("\"expected\": \"billing\"", "\"expected\": \"other\"");
        check(!first.equals(lines.get(0)), "the first case was not changed");
        lines.set(0, first);
        Path file = tmp("cases.jsonl", String.join("\n", lines) + "\n");
        Out r = run("--dataset", file.toString());
        eq(1, r.code(), r.out());
        check(r.out().contains("BAR MISSED"), r.out());
        check(r.out().contains("FAILED billing-double-charge"), r.out());
    }

    public void testAChattyAnswerFailsTheFormatScorerEvenWhenItIsRight() throws Exception {
        Solver chatty = new Solver() {
            public String name() {
                return "stub-chatty";
            }

            public boolean stub() {
                return true;
            }

            public String solve(Case c) {
                return "The label is " + new StubSolver().solve(c) + ".";
            }
        };
        Runner.Report report = Runner.evaluate(cases(), chatty);
        eq(0.0, report.mean("format"), "format mean");
        check(!bar().missed(report).isEmpty(), "the bar did not bite");
    }

    public void testASolverThatThrowsIsAnErrorAndNeverAPass() throws Exception {
        Solver broken = new Solver() {
            public String name() {
                return "stub-broken";
            }

            public boolean stub() {
                return true;
            }

            public String solve(Case c) {
                throw new IllegalStateException("provider down");
            }
        };
        Runner.Report report = Runner.evaluate(cases(), broken);
        eq(12L, report.errors(), "errors");
        eq(0.0, report.passRate(), "pass rate");
        check(bar().missed(report).stream().anyMatch(m -> m.contains("error")), "no error line");
        eq(1, run(broken).code(), "exit");
    }

    public void testABrokenDatasetStopsTheRun() throws Exception {
        String row = "{\"id\": \"a\", \"input\": \"i\", \"expected\": \"bug\"}\n";
        Map<String, String> broken = new java.util.LinkedHashMap<>();
        broken.put("twice", row + row);
        broken.put("input", "{\"id\": \"a\", \"input\": \"\", \"expected\": \"bug\"}\n");
        broken.put("no cases", "\n");
        broken.put("not JSON", "not json\n");
        for (Map.Entry<String, String> e : broken.entrySet()) {
            Out r = run("--dataset", tmp("c.jsonl", e.getValue()).toString());
            eq(2, r.code(), e.getKey());
            check(r.err().contains(e.getKey()), e.getKey() + ": " + r.err());
        }
    }

    public void testAScorerWithNoBarStopsTheRun() throws Exception {
        Path file = tmp("t.toml", "[bar]\nmin_pass_rate = 0.5\nmax_errors = 0\n[bar.scorers]\nexact = 0.5\n");
        Out r = run("--bar", file.toString());
        eq(2, r.code(), "exit");
        check(r.err().contains("no bar for the scorer(s) format"), r.err());
    }

    public void testABarThatIsNotUnderstoodStopsTheRun() throws Exception {
        for (String text : List.of(
                "[bar]\nmin_pass_rate = high\n",
                "min_pass_rate = 1\n",
                "[bar]\nmin_pass_rate = 2\nmax_errors = 0\n[bar.scorers]\nexact = 1\nformat = 1\n")) {
            eq(2, run("--bar", tmp("t.toml", text).toString()).code(), text);
        }
    }

    public void testAMissingBarOrDatasetStopsTheRun() {
        eq(2, run("--bar", "/nonexistent/t.toml").code(), "bar");
        eq(2, run("--dataset", "/nonexistent/c.jsonl").code(), "dataset");
    }

    public void testTheScorers() throws Exception {
        Case billing = cases().get(0);
        eq(1, Scorers.exact(billing, " Billing\n"), "exact");
        eq(0, Scorers.exact(billing, "bug"), "exact wrong");
        eq(1, Scorers.format(billing, "billing"), "format");
        for (String bad : List.of("Billing", "billing.", "billing, bug", "")) {
            eq(0, Scorers.format(billing, bad), "format " + bad);
        }
    }

    public void testEveryScorerHasABar() throws Exception {
        eq(new HashSet<>(Scorers.ALL.keySet()), bar().scorers().keySet(), "scorers with a bar");
    }

    public static void main(String[] args) throws Exception {
        int failed = 0;
        int ran = 0;
        SuiteTests tests = new SuiteTests();
        List<Method> methods = new ArrayList<>(List.of(SuiteTests.class.getDeclaredMethods()));
        methods.sort((a, b) -> a.getName().compareTo(b.getName()));
        for (Method m : methods) {
            if (!m.getName().startsWith("test")) {
                continue;
            }
            ran++;
            try {
                m.invoke(tests);
                System.out.println("ok   " + m.getName());
            } catch (java.lang.reflect.InvocationTargetException e) {
                failed++;
                System.out.println("FAIL " + m.getName() + ": " + e.getCause());
            }
        }
        System.out.println(ran + " tests, " + failed + " failed");
        System.exit(failed == 0 && ran > 0 ? 0 : 1);
    }
}
