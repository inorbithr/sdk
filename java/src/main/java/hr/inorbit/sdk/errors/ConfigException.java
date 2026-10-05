package hr.inorbit.sdk.errors;

import java.util.List;
import java.util.Objects;

/**
 * The client was configured in a way it cannot work with; the message says what to set. A
 * configuration {@code load} refuses lists every problem it found in {@link #problems()}, each
 * with its setting and source (docs/config.md section 2.5).
 */
public final class ConfigException extends InOrbitException {

    private static final long serialVersionUID = 1L;

    private final transient List<Problem> problems;

    /**
     * A configuration problem.
     *
     * @param message what is wrong and what to do
     */
    public ConfigException(String message) {
        super(message, null);
        this.problems = List.of();
    }

    /**
     * Every problem a resolution found.
     *
     * @param message the text listing them
     * @param problems the problems, in catalogue order
     */
    public ConfigException(String message, List<Problem> problems) {
        super(message, null);
        this.problems = List.copyOf(problems);
    }

    /**
     * The problems as data: setting, source and message, in catalogue order. Empty for an error
     * that is not about one setting.
     *
     * @return the problems
     */
    public List<Problem> problems() {
        return problems == null ? List.of() : problems;
    }

    @Override
    public String kind() {
        return "config";
    }

    /**
     * One problem: the setting, where its value came from ({@code code}, {@code env
     * INORBIT_TIMEOUT}, {@code file <path> [sdk]}, or empty), and what to do. Never a secret's
     * value.
     *
     * @param setting the catalogue name, or {@code credential} for the chain
     * @param source the source label
     * @param message what is wrong and what to do
     */
    public record Problem(String setting, String source, String message) {
        /**
         * A problem.
         *
         * @param setting the catalogue name
         * @param source the source label
         * @param message the message
         */
        public Problem {
            Objects.requireNonNull(setting, "setting");
            Objects.requireNonNull(source, "source");
            Objects.requireNonNull(message, "message");
        }
    }
}
