package hr.inorbit.sdk;

import java.util.Map;
import java.util.Objects;

/**
 * What {@link Client#load} reads instead of the process (docs/config.md section 9.2): the
 * environment, the operating system whose conventions apply to paths, the home directory and the
 * working directory. Every value left out is the process's own, so a test can resolve a
 * configuration without touching {@link System#getenv()}.
 *
 * <pre>{@code
 * Client client = Client.builder()
 *         .load(LoadOptions.builder().env(Map.of("INORBIT_TOKEN", "t")).home("").build());
 * }</pre>
 */
public final class LoadOptions {

    private final Map<String, String> env;
    private final String os;
    private final String home;
    private final String cwd;

    private LoadOptions(Builder b) {
        this.env = b.env == null ? null : Map.copyOf(b.env);
        this.os = b.os;
        this.home = b.home;
        this.cwd = b.cwd;
    }

    /**
     * A builder; nothing set reads the process.
     *
     * @return the builder
     */
    public static Builder builder() {
        return new Builder();
    }

    /**
     * The environment {@code load} sees, or {@code null} for the process's.
     *
     * @return the environment
     */
    public Map<String, String> env() {
        return env;
    }

    /**
     * {@code linux}, {@code macos} or {@code windows}, or {@code null} for the running one.
     *
     * @return the operating system
     */
    public String os() {
        return os;
    }

    /**
     * The home directory; {@code ""} means there is none; {@code null} the user's own.
     *
     * @return the home directory
     */
    public String home() {
        return home;
    }

    /**
     * The working directory, for relative paths in code and the environment; {@code null} the
     * process's own.
     *
     * @return the working directory
     */
    public String cwd() {
        return cwd;
    }

    /** The fields set, never the environment's values. */
    @Override
    public String toString() {
        return "LoadOptions[env=" + (env == null ? "process" : env.size() + " variables") + ", os=" + os + ", home="
                + home + ", cwd=" + cwd + "]";
    }

    /** Builds {@link LoadOptions}. */
    public static final class Builder {
        private Map<String, String> env;
        private String os;
        private String home;
        private String cwd;

        private Builder() {}

        /**
         * The environment to read; an empty value is unset.
         *
         * @param env the variables
         * @return this builder
         */
        public Builder env(Map<String, String> env) {
            this.env = Objects.requireNonNull(env, "env");
            return this;
        }

        /**
         * The operating system whose config file location and path rules apply.
         *
         * @param os {@code linux}, {@code macos} or {@code windows}
         * @return this builder
         */
        public Builder os(String os) {
            if (!os.equals("linux") && !os.equals("macos") && !os.equals("windows")) {
                throw new IllegalArgumentException("os is linux, macos or windows");
            }
            this.os = os;
            return this;
        }

        /**
         * The home directory; {@code ""} for none (a sandbox, a distroless container).
         *
         * @param home the directory
         * @return this builder
         */
        public Builder home(String home) {
            this.home = Objects.requireNonNull(home, "home");
            return this;
        }

        /**
         * The working directory relative paths in code and the environment resolve against.
         *
         * @param cwd the directory
         * @return this builder
         */
        public Builder cwd(String cwd) {
            this.cwd = Objects.requireNonNull(cwd, "cwd");
            return this;
        }

        /**
         * The options.
         *
         * @return the options
         */
        public LoadOptions build() {
            return new LoadOptions(this);
        }
    }
}
