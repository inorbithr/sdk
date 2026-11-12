import app.Cli;
import java.io.ByteArrayOutputStream;
import java.io.PrintStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.regex.Pattern;

/**
 * The passes-on-init test, with no test framework to install: golden output in text and JSON
 * (golden/), the exit codes, and a version line that carries a commit. Run from the project
 * folder; it exits 1 with the first failure.
 */
public final class CliTest {
    private static final String COMMIT = "0123456789ab";

    private record Result(String out, String err, int code) {}

    private static Result callAt(String commit, String... args) {
        ByteArrayOutputStream o = new ByteArrayOutputStream();
        ByteArrayOutputStream e = new ByteArrayOutputStream();
        int code = Cli.run(args, new PrintStream(o, true, StandardCharsets.UTF_8),
                new PrintStream(e, true, StandardCharsets.UTF_8), commit);
        return new Result(o.toString(StandardCharsets.UTF_8), e.toString(StandardCharsets.UTF_8), code);
    }

    private static Result call(String... args) {
        return callAt(COMMIT, args);
    }

    private static void check(boolean ok, String what) {
        if (!ok) {
            System.err.println("FAIL: " + what);
            System.exit(1);
        }
    }

    private static void golden(String file, String... args) throws Exception {
        Result r = call(args);
        String want = Files.readString(Path.of("golden", file));
        check(r.code() == 0 && r.out().equals(want),
                String.join(" ", args) + ": exit " + r.code() + ", output " + r.out() + " differs from golden/" + file);
    }

    private record Case(int code, String... args) {}

    public static void main(String[] args) throws Exception {
        golden("help.txt", "--help");
        golden("greet.txt", "greet", "Ada");
        golden("version.txt", "version");
        golden("greet.json", "--json", "greet", "Ada");
        golden("version.json", "version", "--json");

        for (Case c : new Case[] {
            new Case(Cli.OK, "greet"),
            new Case(Cli.USAGE),
            new Case(Cli.USAGE, "nope"),
            new Case(Cli.USAGE, "greet", "a", "b"),
            new Case(Cli.FAILURE, "greet", "   "),
            new Case(Cli.FAILURE, "greet", "x".repeat(65)),
        }) {
            Result r = call(c.args());
            check(r.code() == c.code(), String.join(" ", c.args()) + ": exit " + r.code() + ", want " + c.code());
        }
        check(call("nope").out().isEmpty(), "a usage error writes to stderr, not stdout");

        // The commit a real build stamps (or "unknown" without one) shows up in the version line.
        String line = callAt(Cli.buildCommit(), "version").out().strip();
        check(Pattern.compile("\\(commit ([0-9a-f]{7,40}|unknown)\\)$").matcher(line).find(),
                "the version line has a commit: " + line);

        Result hostile = call("\u001b[31mred" + "x".repeat(100));
        check(hostile.err().indexOf('\u001b') < 0 && hostile.err().length() < 200,
                "the error echoed its argument raw: " + hostile.err());
        System.out.println("ok: golden text and JSON, exit codes, version commit, hostile argument");
    }
}
