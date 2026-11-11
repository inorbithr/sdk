package app;

import java.io.IOException;
import java.io.InputStream;
import java.io.PrintStream;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.List;
import java.util.regex.Pattern;

/**
 * @@NAME@@: a command line with text and JSON output, exit codes that mean something, and a
 * version that says which commit it was built from. The JDK's own classes only.
 */
public final class Cli {
    private Cli() {}

    static final String VERSION = "0.1.0";

    /** Exit codes: the contract scripts rely on. */
    public static final int OK = 0;
    public static final int FAILURE = 1;
    public static final int USAGE = 2;

    private static final Pattern COMMIT = Pattern.compile("[0-9a-f]{7,40}");

    static final String USAGE_TEXT = """
            Usage: @@NAME@@ [--json] <command>

            Commands:
              greet [NAME]   Say hello to NAME (default: world)
              version        Print the version and the commit it was built from

            Options:
                  --json     Machine-readable output
              -h, --help     Show this help

            Exit codes: 0 success, 1 failure, 2 usage error.
            """;

    /**
     * The commit this was built from: the build writes it to commit.txt next to the classes
     * (`git rev-parse --short=12 HEAD > out/commit.txt`, see the README); without that file it is
     * "unknown".
     */
    public static String buildCommit() {
        try (InputStream in = Cli.class.getResourceAsStream("/commit.txt")) {
            if (in != null) {
                String c = new String(in.readAllBytes(), StandardCharsets.UTF_8).strip();
                if (COMMIT.matcher(c).matches()) {
                    return c.length() > 12 ? c.substring(0, 12) : c;
                }
            }
        } catch (IOException ignored) {
            // an unreadable stamp is the same as none
        }
        return "unknown";
    }

    /** The whole program, with its streams as arguments so a test can read them. */
    public static int run(String[] args, PrintStream stdout, PrintStream stderr, String commit) {
        boolean asJson = false;
        List<String> rest = new ArrayList<>();
        for (String a : args) {
            switch (a) {
                case "--json" -> asJson = true;
                case "-h", "--help" -> {
                    stdout.print(USAGE_TEXT);
                    return OK;
                }
                default -> rest.add(a);
            }
        }
        if (rest.isEmpty()) {
            stderr.print(USAGE_TEXT);
            return USAGE;
        }
        switch (rest.get(0)) {
            case "greet":
                return greet(rest.subList(1, rest.size()), asJson, stdout, stderr);
            case "version":
            case "--version":
                if (rest.size() > 1) {
                    return usageError(stderr, "version takes no arguments");
                }
                return version(asJson, stdout, commit);
            default:
                return usageError(stderr, "unknown command " + quote(rest.get(0)));
        }
    }

    private static int usageError(PrintStream stderr, String msg) {
        stderr.println("@@NAME@@: " + msg);
        stderr.println("run `@@NAME@@ --help` for the commands");
        return USAGE;
    }

    /** Keeps a hostile argument from shaping the error line: it is cut and escaped. */
    private static String quote(String s) {
        int[] cps = s.codePoints().toArray();
        String shown = cps.length > 40 ? new String(cps, 0, 40) + "..." : s;
        StringBuilder b = new StringBuilder("\"");
        for (int i = 0; i < shown.length(); i++) {
            char c = shown.charAt(i);
            if (c == '"' || c == '\\') {
                b.append('\\').append(c);
            } else if (c < 0x20 || c == 0x7f) {
                b.append(String.format("\\x%02x", (int) c));
            } else {
                b.append(c);
            }
        }
        return b.append('"').toString();
    }

    private static String json(String s) {
        StringBuilder b = new StringBuilder("\"");
        for (int i = 0; i < s.length(); i++) {
            char c = s.charAt(i);
            switch (c) {
                case '"' -> b.append("\\\"");
                case '\\' -> b.append("\\\\");
                case '\n' -> b.append("\\n");
                case '\r' -> b.append("\\r");
                case '\t' -> b.append("\\t");
                default -> {
                    if (c < 0x20) {
                        b.append(String.format("\\u%04x", (int) c));
                    } else {
                        b.append(c);
                    }
                }
            }
        }
        return b.append('"').toString();
    }

    private static int greet(List<String> args, boolean asJson, PrintStream stdout, PrintStream stderr) {
        if (args.size() > 1) {
            return usageError(stderr, "greet takes at most one name");
        }
        String name = args.isEmpty() ? "world" : args.get(0).strip();
        if (name.isEmpty() || name.codePointCount(0, name.length()) > 64) {
            stderr.println("@@NAME@@: the name is 1 to 64 characters of text");
            return FAILURE;
        }
        String msg = "Hello, " + name;
        // Keys are written in sorted order: stable output for scripts.
        stdout.println(asJson ? "{\"message\":" + json(msg) + "}" : msg);
        return OK;
    }

    private static int version(boolean asJson, PrintStream stdout, String commit) {
        if (asJson) {
            stdout.println("{\"commit\":" + json(commit) + ",\"name\":\"@@NAME@@\",\"version\":\"" + VERSION + "\"}");
        } else {
            stdout.println("@@NAME@@ " + VERSION + " (commit " + commit + ")");
        }
        return OK;
    }
}
