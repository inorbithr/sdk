package @@MODULE@@;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.NoSuchFileException;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.HashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;

/** The dataset: JSON Lines, one case per line, loaded strictly. */
public final class Dataset {
    public static final String DEFAULT_DATASET = "datasets/tickets.jsonl";
    public static final String DEFAULT_BAR = "thresholds.toml";

    private Dataset() {}

    /** The reason an I/O error carries, without the OS code, so the message reads the same everywhere. */
    static String reason(IOException e) {
        return e instanceof NoSuchFileException ? "no such file or directory" : String.valueOf(e.getMessage());
    }

    public static List<Case> load(Path path) throws SuiteError {
        String text;
        try {
            text = Files.readString(path, StandardCharsets.UTF_8);
        } catch (IOException e) {
            throw new SuiteError("cannot read the dataset " + path + ": " + reason(e));
        }
        String name = path.getFileName().toString();
        List<Case> cases = new ArrayList<>();
        Set<String> seen = new HashSet<>();
        String[] lines = text.split("\r?\n", -1);
        for (int i = 0; i < lines.length; i++) {
            int number = i + 1;
            if (lines[i].isBlank()) {
                continue;
            }
            Object row;
            try {
                row = Json.parse(lines[i]);
            } catch (IllegalArgumentException e) {
                throw new SuiteError(name + " line " + number + ": not JSON (" + e.getMessage() + ")");
            }
            if (!(row instanceof Map<?, ?> map)) {
                throw new SuiteError(name + " line " + number + ": expected an object");
            }
            String[] field = new String[3];
            String[] keys = {"id", "input", "expected"};
            for (int k = 0; k < keys.length; k++) {
                if (!(map.get(keys[k]) instanceof String s) || s.isBlank()) {
                    throw new SuiteError(
                            name + " line " + number + ": `" + keys[k] + "` must be a non-empty string");
                }
                field[k] = s;
            }
            if (!seen.add(field[0])) {
                throw new SuiteError(name + " line " + number + ": the id `" + field[0] + "` is used twice");
            }
            cases.add(new Case(field[0], field[1], field[2]));
        }
        if (cases.isEmpty()) {
            throw new SuiteError(name + " has no cases: an empty suite passes nothing");
        }
        return cases;
    }
}
