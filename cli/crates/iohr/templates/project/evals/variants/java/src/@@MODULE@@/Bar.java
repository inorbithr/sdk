package @@MODULE@@;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.TreeMap;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

/** What the suite must reach to pass, read from thresholds.toml. */
public record Bar(double minPassRate, int maxErrors, Map<String, Double> scorers) {
    private static final Pattern TABLE = Pattern.compile("^\\[([A-Za-z0-9_.]+)\\]$");
    private static final Pattern KEY = Pattern.compile("^([A-Za-z0-9_]+)\\s*=\\s*([+-]?[0-9]+(\\.[0-9]+)?)$");

    /**
     * Reads the small part of TOML the bar uses: `[table]` lines, `key = number` lines and `#`
     * comments. The JDK has no TOML reader, and the bar needs nothing more, so anything else is
     * refused rather than guessed at.
     */
    private static Map<String, Map<String, String>> tables(String name, String text) throws SuiteError {
        Map<String, Map<String, String>> tables = new LinkedHashMap<>();
        Map<String, String> current = null;
        String[] lines = text.split("\r?\n", -1);
        for (int i = 0; i < lines.length; i++) {
            String line = lines[i];
            int hash = line.indexOf('#');
            if (hash >= 0) {
                line = line.substring(0, hash);
            }
            line = line.strip();
            if (line.isEmpty()) {
                continue;
            }
            Matcher table = TABLE.matcher(line);
            if (table.matches()) {
                if (tables.containsKey(table.group(1))) {
                    throw new SuiteError(name + ": not valid TOML (line " + (i + 1) + ": table [" + table.group(1) + "] twice)");
                }
                current = new LinkedHashMap<>();
                tables.put(table.group(1), current);
                continue;
            }
            Matcher key = KEY.matcher(line);
            if (!key.matches() || current == null) {
                throw new SuiteError(name + ": not valid TOML (line " + (i + 1)
                        + ": only `[table]` and `key = number` are read here)");
            }
            if (current.put(key.group(1), key.group(2)) != null) {
                throw new SuiteError(name + ": not valid TOML (line " + (i + 1) + ": key " + key.group(1) + " twice)");
            }
        }
        return tables;
    }

    public static Bar load(Path path, List<String> names) throws SuiteError {
        String file = path.getFileName().toString();
        String text;
        try {
            text = Files.readString(path, StandardCharsets.UTF_8);
        } catch (IOException e) {
            throw new SuiteError("cannot read the bar " + path + ": " + Dataset.reason(e));
        }
        Map<String, Map<String, String>> tables = tables(file, text);
        Map<String, String> bar = tables.get("bar");
        if (bar == null) {
            throw new SuiteError(file + ": the [bar] table is missing");
        }
        Map<String, String> per = tables.get("bar.scorers");
        if (!bar.containsKey("min_pass_rate") || !bar.containsKey("max_errors") || per == null
                || bar.get("max_errors").contains(".")) {
            throw new SuiteError(file + ": [bar] needs min_pass_rate, max_errors and [bar.scorers]");
        }
        double rate = Double.parseDouble(bar.get("min_pass_rate"));
        long errors = Long.parseLong(bar.get("max_errors").replace("+", ""));
        String range = file + ": every rate is a number from 0 to 1";
        if (rate < 0 || rate > 1) {
            throw new SuiteError(range);
        }
        Map<String, Double> scorers = new TreeMap<>();
        for (Map.Entry<String, String> e : per.entrySet()) {
            double f = Double.parseDouble(e.getValue());
            if (f < 0 || f > 1) {
                throw new SuiteError(range);
            }
            scorers.put(e.getKey(), f);
        }
        if (errors < 0) {
            throw new SuiteError(file + ": max_errors cannot be negative");
        }
        List<String> unguarded = names.stream().filter(n -> !scorers.containsKey(n)).sorted().toList();
        List<String> unknown = scorers.keySet().stream().filter(k -> !names.contains(k)).sorted().toList();
        if (!unguarded.isEmpty()) {
            throw new SuiteError(file + ": no bar for the scorer(s) " + String.join(", ", unguarded)
                    + "; a scorer with no bar guards nothing");
        }
        if (!unknown.isEmpty()) {
            throw new SuiteError(file + ": a bar for " + String.join(", ", unknown) + ", which is not a scorer");
        }
        return new Bar(rate, (int) Math.min(errors, Integer.MAX_VALUE), scorers);
    }

    /** Every line of the bar the report does not reach; empty means the bar is met. */
    public List<String> missed(Runner.Report report) {
        List<String> out = new ArrayList<>();
        if (report.passRate() < minPassRate) {
            out.add(String.format(Locale.ROOT, "pass rate %.3f is below %.3f", report.passRate(), minPassRate));
        }
        if (report.errors() > maxErrors) {
            out.add(report.errors() + " error(s), at most " + maxErrors + " allowed");
        }
        for (Map.Entry<String, Double> e : scorers.entrySet()) {
            double mean = report.mean(e.getKey());
            if (mean < e.getValue()) {
                out.add(String.format(Locale.ROOT, "scorer %s: mean %.3f is below %.3f", e.getKey(), mean, e.getValue()));
            }
        }
        return out;
    }
}
