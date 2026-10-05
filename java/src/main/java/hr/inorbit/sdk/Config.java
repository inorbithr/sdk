package hr.inorbit.sdk;

import com.fasterxml.jackson.core.JsonProcessingException;
import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.node.ArrayNode;
import com.fasterxml.jackson.databind.node.JsonNodeFactory;
import com.fasterxml.jackson.databind.node.ObjectNode;
import com.fasterxml.jackson.dataformat.toml.TomlMapper;
import hr.inorbit.sdk.errors.ConfigException;
import java.io.File;
import java.io.IOException;
import java.io.InputStream;
import java.net.URI;
import java.net.URISyntaxException;
import java.nio.charset.CharacterCodingException;
import java.nio.charset.CodingErrorAction;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.InvalidPathException;
import java.nio.file.Path;
import java.time.Duration;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Base64;
import java.util.Comparator;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Set;
import java.util.function.Function;
import java.util.function.Predicate;

/**
 * Configuration resolution: code, the environment, the config file, defaults (docs/config.md
 * sections 2 to 5). A port of the reference resolver (the Rust runtime's {@code config} module,
 * which {@code iohr sdk config} runs). Resolution is pure: the environment, the OS, the home and
 * working directories and the file reader are inputs, so the conformance vectors run without
 * touching the process.
 */
final class Config {

    /** The built-in pipeline, outermost first (section 7.2). */
    static final List<String> PIPELINE = List.of(
            "request_id",
            "user_agent",
            "idempotency_key",
            "call_tracing",
            "deadline",
            "retry",
            "auth",
            "rate_limit",
            "attempt_tracing",
            "logging",
            "hooks",
            "timeout");

    /** The largest config file read (section 4.1). */
    static final int MAX_FILE = 1024 * 1024;

    static final String REDACTED = "<redacted>";

    /** The keys of a profile table the command line owns (section 4.2). */
    static final Set<String> CLI_KEYS = Set.of("kind", "account", "storage", "issuer", "client_id");

    /** The credential sources {@code credential_sources} may name, in chain order. */
    static final List<String> SOURCES = List.of("env", "workload", "file", "cli");

    private static final JsonNodeFactory NODES = JsonNodeFactory.instance;

    private Config() {}

    // --- the catalogue -----------------------------------------------------------------

    enum Ty {
        DURATION,
        INT,
        BOOL,
        SCOPES,
        LIST,
        URL,
        PATH,
        SECRET,
        STR,
        ENUM,
        PROXY,
        PINS,
        RESERVED
    }

    record Setting(
            String name,
            Ty ty,
            boolean inFile,
            boolean credential,
            boolean transport,
            Object defaultValue,
            List<String> choices) {}

    private static Setting s(String name, Ty ty) {
        return new Setting(name, ty, true, false, false, null, List.of());
    }

    private static Setting d(String name, Ty ty, Object def) {
        return new Setting(name, ty, true, false, false, def, List.of());
    }

    private static Setting cred(String name, Ty ty, boolean inFile) {
        return new Setting(name, ty, inFile, true, false, null, List.of());
    }

    private static Setting net(String name, Ty ty, Object def) {
        return new Setting(name, ty, true, false, true, def, List.of());
    }

    private static Setting choice(String name, Object def, String... choices) {
        return new Setting(name, Ty.ENUM, true, false, false, def, List.of(choices));
    }

    /** The catalogue (section 3), in its order; problems are reported in this order. */
    static final List<Setting> CATALOGUE = List.of(
            d("base_url", Ty.URL, Client.DEFAULT_BASE_URL),
            d("token_url", Ty.URL, "https://auth.inorbit.hr/oauth2/token"),
            s("region", Ty.RESERVED),
            cred("key_id", Ty.STR, true),
            cred("key_secret", Ty.SECRET, false),
            cred("key_secret_file", Ty.PATH, true),
            s("scopes", Ty.SCOPES),
            cred("token", Ty.SECRET, false),
            cred("token_file", Ty.PATH, true),
            d("credential_sources", Ty.LIST, SOURCES),
            s("cli_path", Ty.PATH),
            net("connect_timeout", Ty.DURATION, "10s"),
            d("timeout", Ty.DURATION, "30s"),
            d("total_timeout", Ty.DURATION, "120s"),
            d("stream_idle_timeout", Ty.DURATION, "45s"),
            d("max_retries", Ty.INT, 2),
            d("retry_base_delay", Ty.DURATION, "500ms"),
            d("retry_max_delay", Ty.DURATION, "8s"),
            d("retry_after_max", Ty.DURATION, "60s"),
            d("retry_budget", Ty.BOOL, Boolean.TRUE),
            choice("streams", "sse", "sse", "socket"),
            net("proxy", Ty.PROXY, null),
            net("no_proxy", Ty.LIST, null),
            net("ca_bundle", Ty.PATH, null),
            net("system_trust", Ty.BOOL, Boolean.TRUE),
            net("client_cert", Ty.PATH, null),
            net("client_key", Ty.PATH, null),
            new Setting("client_key_password", Ty.SECRET, false, false, true, null, List.of()),
            net("pinned_keys", Ty.PINS, null),
            choice("log", "off", "off", "error", "warn", "info", "debug"),
            d("log_headers", Ty.BOOL, Boolean.FALSE),
            s("log_allow_headers", Ty.LIST),
            s("tracing", Ty.BOOL),
            s("metrics", Ty.BOOL),
            choice("rate_limit", "observe", "observe", "wait", "off"),
            s("user_agent_suffix", Ty.STR));

    private static final Map<String, Setting> BY_NAME = new LinkedHashMap<>();

    static {
        for (Setting x : CATALOGUE) {
            BY_NAME.put(x.name(), x);
        }
    }

    private static int order(String setting) {
        if (setting.equals("profile")) {
            return 0;
        }
        if (setting.equals("config_file")) {
            return 1;
        }
        if (setting.equals("credential")) {
            return 1_000_001;
        }
        int i = 0;
        for (Setting x : CATALOGUE) {
            if (x.name().equals(setting)) {
                return i + 2;
            }
            i++;
        }
        return 1_000_000;
    }

    // --- paths ---------------------------------------------------------------------------

    /** The OS this process runs on, by the conventions that matter for paths. */
    static String currentOs() {
        String name = System.getProperty("os.name", "").toLowerCase(Locale.ROOT);
        if (name.startsWith("mac") || name.startsWith("darwin")) {
            return "macos";
        }
        if (name.startsWith("windows")) {
            return "windows";
        }
        return "linux";
    }

    private static String sep(String os) {
        return os.equals("windows") ? "\\" : "/";
    }

    static boolean isAbsolute(String os, String p) {
        if (os.equals("windows")) {
            return p.startsWith("\\")
                    || p.startsWith("/")
                    || (p.length() >= 3
                            && Character.isLetter(p.charAt(0))
                            && p.charAt(0) < 128
                            && p.charAt(1) == ':'
                            && (p.charAt(2) == '\\' || p.charAt(2) == '/'));
        }
        try {
            if (Path.of(p).isAbsolute()) {
                return true;
            }
        } catch (InvalidPathException e) {
            // Not a path on this host; the OS rule decides.
        }
        return p.startsWith("/");
    }

    private static String join(String os, String dir, String rest) {
        int end = dir.length();
        while (end > 0 && (dir.charAt(end - 1) == '/' || dir.charAt(end - 1) == '\\')) {
            end--;
        }
        return dir.substring(0, end) + sep(os) + rest;
    }

    private static String parent(String os, String path) {
        int i = Math.max(path.lastIndexOf('/'), path.lastIndexOf('\\'));
        String parent = i < 0 ? "." : i == 0 ? path.substring(0, 1) : path.substring(0, i);
        return os.equals("windows") ? parent.replace('/', '\\') : parent.replace('\\', '/');
    }

    /** Where the config file is, which rule named it, and whether a missing file is an error. */
    record Located(String path, boolean named, String label) {}

    /**
     * Where the config file is (section 4.1); {@code null} reads no file.
     *
     * @param os the OS whose default location applies
     * @param env the environment; an empty value is unset
     * @param home the home directory, or {@code null} when there is none
     * @param code {@code config_file} set in code, or {@code null}
     */
    static Located configPath(String os, Map<String, String> env, String home, String code) {
        Function<String, String> var = k -> {
            String v = env.get(k);
            return v == null || v.isEmpty() ? null : v;
        };
        if (code != null) {
            return code.equals("off") ? null : new Located(code, true, "code");
        }
        String named = var.apply("INORBIT_CONFIG_FILE");
        if (named != null) {
            return named.equals("off") ? null : new Located(named, true, "env INORBIT_CONFIG_FILE");
        }
        String dir = var.apply("IOHR_CONFIG_DIR");
        if (dir != null) {
            return new Located(join(os, dir, "config.toml"), false, "env IOHR_CONFIG_DIR");
        }
        if (os.equals("linux")) {
            String xdg = var.apply("XDG_CONFIG_HOME");
            if (xdg != null && isAbsolute(os, xdg)) {
                return new Located(join(os, xdg, "iohr/config.toml"), false, "default");
            }
            if (home != null && !home.isEmpty()) {
                return new Located(join(os, home, ".config/iohr/config.toml"), false, "default");
            }
            return null;
        }
        if (os.equals("macos")) {
            if (home == null || home.isEmpty()) {
                return null;
            }
            return new Located(
                    join(os, home, "Library/Application Support/hr.InOrbit.iohr/config.toml"), false, "default");
        }
        String appdata = var.apply("APPDATA");
        if (appdata == null) {
            return null;
        }
        return new Located(join(os, appdata, "InOrbit\\iohr\\config\\config.toml"), false, "default");
    }

    // --- value syntax ------------------------------------------------------------------

    /**
     * A duration string in milliseconds: digits, then {@code ms}, {@code s}, {@code m} or {@code
     * h}, above zero; {@code -1} when the text is not one.
     */
    static long parseDuration(String value) {
        int i = 0;
        while (i < value.length() && value.charAt(i) >= '0' && value.charAt(i) <= '9') {
            i++;
        }
        if (i == 0 || i == value.length() || i > 15) {
            return -1;
        }
        long n = Long.parseLong(value.substring(0, i));
        long scale = switch (value.substring(i)) {
            case "ms" -> 1;
            case "s" -> 1_000;
            case "m" -> 60_000;
            case "h" -> 3_600_000;
            default -> -1;
        };
        if (scale < 0) {
            return -1;
        }
        long ms = n * scale;
        return ms > 0 ? ms : -1;
    }

    /** A duration as {@code describe()} shows it: whole seconds as {@code 30s}, else {@code 500ms}. */
    static String showDuration(long ms) {
        return ms % 1000 == 0 ? (ms / 1000) + "s" : ms + "ms";
    }

    /** Whether {@code host} is {@code localhost} or a loopback address (brackets allowed). */
    static boolean isLoopback(String host) {
        String h = strip(host);
        if (h.equalsIgnoreCase("localhost")) {
            return true;
        }
        byte[] ip = NoProxy.literal(h);
        if (ip == null) {
            return false;
        }
        if (ip.length == 4) {
            return ip[0] == 127;
        }
        for (int i = 0; i < 15; i++) {
            if (ip[i] != 0) {
                return false;
            }
        }
        return ip[15] == 1;
    }

    static String strip(String host) {
        if (host.startsWith("[") && host.endsWith("]") && host.length() >= 2) {
            return host.substring(1, host.length() - 1);
        }
        return host;
    }

    /** A URL with its user-info replaced by {@code <redacted>}. */
    static String redactUserinfo(String raw) {
        int sch = raw.indexOf("://");
        if (sch < 0) {
            return raw;
        }
        String rest = raw.substring(sch + 3);
        int end = rest.length();
        for (char c : new char[] {'/', '?', '#'}) {
            int i = rest.indexOf(c);
            if (i >= 0 && i < end) {
                end = i;
            }
        }
        int at = rest.substring(0, end).lastIndexOf('@');
        return at < 0 ? raw : raw.substring(0, sch) + "://" + REDACTED + "@" + rest.substring(at + 1);
    }

    private static URI uri(String raw) {
        try {
            URI u = new URI(raw);
            return u.getScheme() != null && u.getHost() != null ? u : null;
        } catch (URISyntaxException e) {
            return null;
        }
    }

    private static boolean hasUserinfo(String raw) {
        URI u = uri(raw);
        return u != null && u.getRawUserInfo() != null;
    }

    /** A profile's environment form: upper case, {@code -} as {@code _}. */
    static String envName(String profile) {
        return profile.toUpperCase(Locale.ROOT).replace('-', '_');
    }

    /** A profile name the command line accepts (section 2.3). */
    static boolean validProfile(String name) {
        if (name.isEmpty() || name.length() > 64) {
            return false;
        }
        for (int i = 0; i < name.length(); i++) {
            char c = name.charAt(i);
            boolean ok = (c >= 'a' && c <= 'z') || (c >= '0' && c <= '9') || c == '-' || c == '_';
            if (!ok || (i == 0 && (c == '-' || c == '_'))) {
                return false;
            }
        }
        return true;
    }

    private static String q(String v) {
        return "\"" + v.replace("\\", "\\\\").replace("\"", "\\\"") + "\"";
    }

    private static String tomlType(JsonNode v) {
        if (v.isBoolean()) {
            return "boolean";
        }
        if (v.isIntegralNumber()) {
            return "integer";
        }
        if (v.isFloatingPointNumber()) {
            return "float";
        }
        if (v.isTextual()) {
            return "string";
        }
        if (v.isArray()) {
            return "array";
        }
        if (v.isObject()) {
            return "table";
        }
        return "datetime";
    }

    // --- what resolution gives ---------------------------------------------------------

    /** The credential {@code load} chose: its source and kind, and what builds it. Holds secrets. */
    static final class Credential {
        final String source;
        final String kind;
        String token;
        String tokenFile;
        String keyId;
        String keySecret;
        String keySecretFile;
        List<String> scopes = List.of();
        String cliPath = "iohr";
        String profile;
        Object provider;

        Credential(String source, String kind) {
            this.source = source;
            this.kind = kind;
        }

        /** The source and kind, never a secret. */
        @Override
        public String toString() {
            return "Credential[" + source + ", " + kind + "]";
        }
    }

    /** What resolution found: the {@code describe()} document, the typed values, the credential. */
    record Resolution(ObjectNode doc, Map<String, Object> values, Credential credential) {}

    /** A value as it came, and from where. */
    private record Raw(String origin, Object value) {}

    /** The default file reader: the bytes, at most one more than 1 MiB, or {@code null}. */
    static byte[] readFile(String path) {
        try {
            Path p = Path.of(path);
            if (!Files.isRegularFile(p)) {
                return null;
            }
            try (InputStream in = Files.newInputStream(p)) {
                return in.readNBytes(MAX_FILE + 1);
            }
        } catch (IOException | InvalidPathException | SecurityException e) {
            return null;
        }
    }

    /**
     * Whether {@code program} can be run: a path to a file, or a name found on {@code PATH} (with
     * {@code PATHEXT}'s extensions on Windows).
     */
    static boolean cliFound(String program, Map<String, String> env) {
        if (program.contains("/") || program.contains("\\") || program.contains(File.separator)) {
            try {
                return Files.isRegularFile(Path.of(program));
            } catch (InvalidPathException e) {
                return false;
            }
        }
        String path = env.get("PATH");
        if (path == null || path.isEmpty()) {
            path = env.get("Path");
        }
        if (path == null || path.isEmpty()) {
            return false;
        }
        List<String> names = new ArrayList<>();
        names.add(program);
        if (File.pathSeparatorChar == ';') {
            String ext = env.getOrDefault("PATHEXT", ".COM;.EXE;.BAT;.CMD");
            for (String e : ext.split(";")) {
                if (!e.isEmpty()) {
                    names.add(program + e.toLowerCase(Locale.ROOT));
                    names.add(program + e);
                }
            }
        }
        for (String dir : path.split(File.pathSeparator)) {
            if (dir.isEmpty()) {
                continue;
            }
            for (String n : names) {
                try {
                    Path p = Path.of(dir, n);
                    if (Files.isRegularFile(p) && (File.pathSeparatorChar == ';' || Files.isExecutable(p))) {
                        return true;
                    }
                } catch (InvalidPathException e) {
                    // Skip a directory this host cannot name.
                }
            }
        }
        return false;
    }

    /**
     * Resolves a configuration the way {@code load} does (sections 2 to 5).
     *
     * @param code options set in code, by catalogue name; durations as {@link Duration}, lists as
     *     {@link List}; {@code token_provider} and {@code http_client} as objects
     * @param options the environment, OS, home and working directory to use
     * @param profileType resolve for this typed (generated) profile, or {@code null}
     * @param found whether the {@code cli} source's program can run, or {@code null} for the default
     * @param read reads a file ({@code null} when it cannot), or {@code null} for the file system
     * @return the document, the typed values and the credential
     * @throws ConfigException every problem found, in catalogue order
     */
    static Resolution resolve(
            Map<String, Object> code,
            LoadOptions options,
            String profileType,
            Predicate<String> found,
            Function<String, byte[]> read) {
        LoadOptions o = options == null ? LoadOptions.builder().build() : options;
        Map<String, String> env = o.env() == null ? System.getenv() : o.env();
        String os = o.os() == null ? currentOs() : o.os();
        String home;
        if (o.home() == null) {
            String h = System.getProperty("user.home");
            home = h == null || h.isEmpty() || h.equals("?") ? null : h;
        } else {
            home = o.home().isEmpty() ? null : o.home();
        }
        String cwd = o.cwd() != null ? o.cwd() : System.getProperty("user.dir", ".");
        Resolver r = new Resolver(
                code,
                env,
                os,
                home,
                cwd,
                profileType,
                found != null ? found : p -> cliFound(p, env),
                read != null ? read : Config::readFile);
        ObjectNode doc = r.loadFile();
        String[] chosen = r.chooseProfile(doc);
        ObjectNode table = null;
        if (chosen != null && doc.path("profiles").path(chosen[0]) instanceof ObjectNode t) {
            table = t;
        }
        if (table != null) {
            r.layers.add(new Layer(table, r.fileLabel("profiles." + chosen[0]), true));
        }
        if (doc.get("sdk") instanceof ObjectNode sdk) {
            r.layers.add(new Layer(sdk, r.fileLabel("sdk"), false));
        }
        r.checkFileKeys();
        r.resolveSettings();
        Credential credential = r.chain(chosen != null ? chosen[0] : null, table);
        r.crossChecks();
        if (!r.problems.isEmpty()) {
            List<ConfigException.Problem> problems = new ArrayList<>(r.problems);
            problems.sort(Comparator.comparingInt(p -> order(p.setting())));
            throw new ConfigException(formatProblems(problems), problems);
        }
        r.values.put("profile", chosen != null ? chosen[0] : null);
        ObjectNode out = NODES.objectNode();
        if (chosen != null) {
            ObjectNode p = out.putObject("profile");
            p.put("name", chosen[0]);
            p.put("source", chosen[1]);
        } else {
            out.putNull("profile");
        }
        if (r.filePath != null) {
            out.put("config_file", r.filePath);
        } else {
            out.putNull("config_file");
        }
        out.set("settings", r.settings);
        ObjectNode c = out.putObject("credential");
        c.put("source", credential.source);
        c.put("kind", credential.kind);
        c.set("tried", r.tried);
        ArrayNode pipeline = out.putArray("pipeline");
        PIPELINE.forEach(pipeline::add);
        out.set("ignored", r.ignored);
        return new Resolution(out, r.values, credential);
    }

    /** A {@code ConfigException}'s text: the chain's message alone, or every problem listed. */
    static String formatProblems(List<ConfigException.Problem> problems) {
        if (problems.size() == 1 && problems.get(0).setting().equals("credential")) {
            return problems.get(0).message();
        }
        int n = problems.size();
        StringBuilder out =
                new StringBuilder("configuration is invalid (" + n + " problem" + (n == 1 ? "" : "s") + "):");
        for (ConfigException.Problem p : problems) {
            String[] lines = p.message().split("\n", -1);
            out.append("\n  ").append(p.setting()).append(": ").append(lines[0]);
            if (!p.source().isEmpty()) {
                out.append(" (from ").append(p.source()).append(')');
            }
            for (int i = 1; i < lines.length; i++) {
                out.append("\n    ").append(lines[i]);
            }
        }
        return out.toString();
    }

    private record Layer(ObjectNode table, String label, boolean profile) {}

    /** A value that could not be used; its message names what to do. */
    private static final class Bad extends Exception {
        private static final long serialVersionUID = 1L;

        Bad(String message) {
            super(message, null, false, false);
        }
    }

    private static final class Resolver {
        final Map<String, Object> code = new LinkedHashMap<>();
        final Map<String, String> env;
        final String os;
        final String home;
        final String cwd;
        final String profileType;
        final Predicate<String> found;
        final Function<String, byte[]> read;
        final String prefix;
        final List<ConfigException.Problem> problems = new ArrayList<>();
        final ObjectNode settings = NODES.objectNode();
        final Map<String, Object> values = new LinkedHashMap<>();
        final ArrayNode ignored = NODES.arrayNode();
        final List<Layer> layers = new ArrayList<>();
        String filePath;
        String fileDir;
        ArrayNode tried = NODES.arrayNode();

        Resolver(
                Map<String, Object> code,
                Map<String, String> env,
                String os,
                String home,
                String cwd,
                String profileType,
                Predicate<String> found,
                Function<String, byte[]> read) {
            code.forEach((k, v) -> {
                if (v != null) {
                    this.code.put(k, v);
                }
            });
            this.env = env;
            this.os = os;
            this.home = home;
            this.cwd = cwd;
            this.fileDir = cwd;
            this.profileType = profileType;
            this.found = found;
            this.read = read;
            this.prefix = profileType != null ? "INORBIT_" + envName(profileType) + "_" : null;
        }

        String var(String k) {
            String v = env.get(k);
            return v == null || v.isEmpty() ? null : v;
        }

        void problem(String setting, String source, String message) {
            problems.add(new ConfigException.Problem(setting, source, message));
        }

        boolean httpClient() {
            return code.containsKey("http_client");
        }

        String fileLabel(String suffix) {
            if (filePath == null) {
                return "";
            }
            return "file " + filePath + (suffix.isEmpty() ? "" : " [" + suffix + "]");
        }

        ObjectNode loadFile() {
            Object codeFile = code.get("config_file");
            Located located = configPath(os, env, home, codeFile != null ? codeFile.toString() : null);
            if (located == null) {
                return NODES.objectNode();
            }
            String p = isAbsolute(os, located.path()) ? located.path() : join(os, cwd, located.path());
            byte[] data = read.apply(p);
            if (data == null) {
                if (located.named()) {
                    problem("config_file", located.label(), "there is no readable file at " + p);
                }
                return NODES.objectNode();
            }
            if (data.length > MAX_FILE) {
                problem("config_file", located.label(), p + " is larger than 1 MiB");
                return NODES.objectNode();
            }
            String text;
            try {
                text = StandardCharsets.UTF_8
                        .newDecoder()
                        .onMalformedInput(CodingErrorAction.REPORT)
                        .onUnmappableCharacter(CodingErrorAction.REPORT)
                        .decode(java.nio.ByteBuffer.wrap(data))
                        .toString();
            } catch (CharacterCodingException e) {
                problem("config_file", located.label(), p + " is not valid TOML: it is not UTF-8");
                return NODES.objectNode();
            }
            JsonNode doc;
            try {
                doc = Toml.MAPPER.readTree(text);
            } catch (JsonProcessingException e) {
                // The position only: a snippet of the file could quote a secret.
                var at = e.getLocation();
                String where = at == null ? "" : " at line " + at.getLineNr() + ", column " + at.getColumnNr();
                problem("config_file", located.label(), p + " is not valid TOML" + where);
                return NODES.objectNode();
            }
            filePath = p;
            fileDir = parent(os, p);
            return doc instanceof ObjectNode obj ? obj : NODES.objectNode();
        }

        String[] chooseProfile(ObjectNode doc) {
            if (profileType != null) {
                return new String[] {profileType, "code"};
            }
            String[] chosen = null;
            Object codeProfile = code.get("profile");
            String v;
            if (codeProfile instanceof String cp) {
                chosen = new String[] {cp, "code"};
            } else if ((v = var("INORBIT_PROFILE")) != null) {
                chosen = new String[] {v, "env INORBIT_PROFILE"};
            } else if (doc.path("default").isTextual()) {
                chosen = new String[] {doc.path("default").asText(), fileLabel("")};
            }
            if (chosen == null) {
                return null;
            }
            String name = chosen[0];
            boolean has =
                    doc.path("profiles").isObject() && doc.path("profiles").has(name);
            if (!validProfile(name)) {
                problem(
                        "profile",
                        chosen[1],
                        q(name) + " is not a profile name: 1 to 64 lower-case letters, digits, "
                                + "'-' or '_', starting with a letter or digit");
                return null;
            }
            if (!has) {
                String where =
                        filePath == null ? "no config file was read" : filePath + " has no [profiles." + name + "]";
                problem(
                        "profile",
                        chosen[1],
                        "there is no profile " + q(name) + ": " + where + "; `iohr profile list` shows the profiles");
                return null;
            }
            return chosen;
        }

        void checkFileKeys() {
            for (Layer layer : layers) {
                for (Map.Entry<String, JsonNode> e : layer.table().properties()) {
                    String k = e.getKey();
                    if (layer.profile() && CLI_KEYS.contains(k)) {
                        continue;
                    }
                    Setting st = BY_NAME.get(k);
                    if (st != null && !st.inFile()) {
                        String wayOut = switch (st.name()) {
                            case "key_secret" -> "key_secret_file, the environment, or iohr login";
                            case "token" -> "token_file, the environment, or iohr login";
                            default -> "the environment or code";
                        };
                        problem(k, layer.label(), "secrets are not allowed in the config file; use " + wayOut);
                    } else if (st != null
                            && st.ty() == Ty.PROXY
                            && e.getValue().isTextual()
                            && hasUserinfo(e.getValue().asText())) {
                        problem(
                                k,
                                layer.label(),
                                "a proxy URL with a user name or password holds a secret, which is not "
                                        + "allowed in the config file; set it in INORBIT_PROXY or in code");
                    } else if (st == null && (k.equals("profile") || k.equals("config_file"))) {
                        ignore(k, layer.label(), "not read from the config file");
                    } else if (st == null) {
                        ignore(k, layer.label(), "unknown key");
                    }
                }
            }
        }

        void ignore(String key, String source, String reason) {
            ObjectNode i = ignored.addObject();
            i.put("key", key);
            i.put("source", source);
            i.put("reason", reason);
        }

        List<String> envNames(Setting st) {
            String upper = st.name().toUpperCase(Locale.ROOT);
            List<String> names = new ArrayList<>();
            if (prefix != null) {
                names.add(prefix + upper);
            }
            if (!(st.credential() && prefix != null)) {
                names.add("INORBIT_" + upper);
            }
            return names;
        }

        Object[] raw(Setting st) {
            if (code.containsKey(st.name())) {
                return new Object[] {new Raw("code", code.get(st.name())), "code"};
            }
            for (String n : envNames(st)) {
                String v = var(n);
                if (v != null) {
                    return new Object[] {new Raw("env", v), "env " + n};
                }
            }
            if (st.inFile()) {
                for (Layer layer : layers) {
                    if (layer.table().has(st.name())) {
                        return new Object[] {new Raw("file", layer.table().get(st.name())), layer.label()};
                    }
                }
            }
            List<String> after = switch (st.name()) {
                case "proxy" -> List.of("https_proxy", "HTTPS_PROXY");
                case "no_proxy" -> List.of("no_proxy", "NO_PROXY");
                default -> List.of();
            };
            for (String n : after) {
                String v = var(n);
                if (v != null) {
                    return new Object[] {new Raw("env", v), "env " + n};
                }
            }
            return null;
        }

        void show(String name, JsonNode shown, String source, Object value) {
            ObjectNode e = NODES.objectNode();
            e.set("value", shown);
            e.put("source", source);
            settings.set(name, e);
            values.put(name, value);
        }

        void show(String name, String shown, String source, Object value) {
            show(name, NODES.textNode(shown), source, value);
        }

        void resolveSettings() {
            for (Setting st : CATALOGUE) {
                if (st.credential()) {
                    continue;
                }
                Object[] found = raw(st);
                if (found == null) {
                    if (st.defaultValue() != null && !(st.transport() && httpClient())) {
                        try {
                            Object[] parsed = parse(st, new Raw("code", st.defaultValue()));
                            show(st.name(), (JsonNode) parsed[0], "default", parsed[1]);
                        } catch (Bad e) {
                            throw new IllegalStateException("a default does not parse: " + st.name(), e);
                        }
                    }
                    continue;
                }
                Raw raw = (Raw) found[0];
                String source = (String) found[1];
                if (st.transport() && httpClient()) {
                    if (source.equals("code")) {
                        problem(st.name(), source, "configure this on your HTTP client, or leave http_client out");
                    } else {
                        ignore(st.name(), source, "the caller's HTTP client decides this");
                    }
                    continue;
                }
                try {
                    Object[] parsed = parse(st, raw);
                    show(st.name(), (JsonNode) parsed[0], source, parsed[1]);
                } catch (Bad e) {
                    problem(st.name(), source, e.getMessage());
                }
            }
        }

        String path(String p, boolean fromFile) throws Bad {
            if (p.startsWith("~/")) {
                if (home == null) {
                    throw new Bad(p + " starts with ~/ but there is no home directory");
                }
                return join(os, home, p.substring(2));
            }
            if (isAbsolute(os, p)) {
                return p;
            }
            return join(os, fromFile ? fileDir : cwd, p);
        }

        /** The value as {@code describe()} shows it, and as the client uses it. */
        @SuppressWarnings("unchecked")
        Object[] parse(Setting st, Raw raw) throws Bad {
            Object v = raw.value();
            String origin = raw.origin();
            if (v instanceof JsonNode node && !origin.equals("file")) {
                v = node.isTextual() ? node.asText() : node;
            }
            final Object value = v;
            switch (st.ty()) {
                case DURATION -> {
                    long ms;
                    if (origin.equals("code") && value instanceof Duration dur) {
                        ms = dur.toMillis();
                        if (ms <= 0) {
                            throw new Bad("must be a duration greater than zero");
                        }
                    } else {
                        String t = text(value, origin, "a duration string such as \"30s\"");
                        ms = parseDuration(t);
                        if (ms < 0) {
                            if (parseDuration(t + "s") >= 0) {
                                throw new Bad(q(t) + " is not a duration; write it with a unit, such as 30s");
                            }
                            throw new Bad(q(t) + " is not a duration greater than zero: digits, then ms, s, m "
                                    + "or h, such as 30s");
                        }
                    }
                    return new Object[] {NODES.textNode(showDuration(ms)), Duration.ofMillis(ms)};
                }
                case INT -> {
                    long n;
                    if (origin.equals("env")) {
                        String t = (String) value;
                        if (!t.matches("[0-9]{1,10}") || Long.parseLong(t) > 0xFFFFFFFFL) {
                            throw new Bad(q(t) + " is not a whole number of 0 or more");
                        }
                        n = Long.parseLong(t);
                    } else if (value instanceof JsonNode node && node.isIntegralNumber()) {
                        n = node.asLong();
                    } else if (value instanceof Integer || value instanceof Long) {
                        n = ((Number) value).longValue();
                    } else {
                        throw new Bad(
                                origin.equals("file")
                                        ? "must be an integer, not " + tomlType((JsonNode) value)
                                        : "must be an integer");
                    }
                    if (n < 0 || n > 0xFFFFFFFFL) {
                        throw new Bad("must be a whole number of 0 or more");
                    }
                    return new Object[] {NODES.numberNode(n), n};
                }
                case BOOL -> {
                    if (origin.equals("env")) {
                        String t = ((String) value).toLowerCase(Locale.ROOT);
                        if (t.equals("true") || t.equals("1")) {
                            return new Object[] {NODES.booleanNode(true), Boolean.TRUE};
                        }
                        if (t.equals("false") || t.equals("0")) {
                            return new Object[] {NODES.booleanNode(false), Boolean.FALSE};
                        }
                        throw new Bad(q((String) value) + " is not true, false, 1 or 0");
                    }
                    if (value instanceof Boolean b) {
                        return new Object[] {NODES.booleanNode(b), b};
                    }
                    if (value instanceof JsonNode node && node.isBoolean()) {
                        return new Object[] {NODES.booleanNode(node.asBoolean()), node.asBoolean()};
                    }
                    throw new Bad(
                            origin.equals("file")
                                    ? "must be a boolean, not " + tomlType((JsonNode) value)
                                    : "must be a boolean");
                }
                case SCOPES -> {
                    List<String> got = items(value, origin, false);
                    return new Object[] {array(got), got};
                }
                case LIST -> {
                    List<String> got = items(value, origin, true);
                    if (st.name().equals("credential_sources")) {
                        for (String x : got) {
                            if (!SOURCES.contains(x)) {
                                throw new Bad(q(x) + " is not a credential source; use env, workload, file or cli");
                            }
                        }
                    }
                    if (st.name().equals("no_proxy")) {
                        for (String x : got) {
                            if (!NoProxy.validEntry(x)) {
                                throw new Bad(q(x) + " is not a no_proxy entry: a host, .domain, host:port, an IP "
                                        + "address or a CIDR range");
                            }
                        }
                    }
                    if (st.name().equals("log_allow_headers")) {
                        got = got.stream().map(x -> x.toLowerCase(Locale.ROOT)).toList();
                    }
                    return new Object[] {array(got), got};
                }
                case URL -> {
                    String t = text(value, origin, "a URL string");
                    URI u = uri(t);
                    if (u == null) {
                        throw new Bad(q(t) + " is not an absolute URL");
                    }
                    if (!(u.getScheme().equals("https") || (u.getScheme().equals("http") && isLoopback(u.getHost())))) {
                        throw new Bad(q(t) + " must use https (plain http is allowed only for localhost and "
                                + "loopback addresses)");
                    }
                    return new Object[] {NODES.textNode(t), t};
                }
                case PATH -> {
                    String t = value instanceof Path p ? p.toString() : text(value, origin, "a path string");
                    String p = path(t, origin.equals("file"));
                    return new Object[] {NODES.textNode(p), p};
                }
                case SECRET -> {
                    if (value instanceof String sv) {
                        return new Object[] {NODES.textNode(REDACTED), sv};
                    }
                    throw new Bad("must be a string");
                }
                case STR -> {
                    String t = text(value, origin, "a string");
                    if (st.name().equals("user_agent_suffix") && (t.length() > 128 || !t.matches("[\\x20-\\x7e]*"))) {
                        throw new Bad("must be product tokens (such as myapp/1.2), at most 128 printable ASCII "
                                + "characters");
                    }
                    return new Object[] {NODES.textNode(t), t};
                }
                case ENUM -> {
                    String t = text(value, origin, "a string");
                    if (!st.choices().contains(t)) {
                        throw new Bad(q(t) + " is not one of " + String.join(", ", st.choices()));
                    }
                    return new Object[] {NODES.textNode(t), t};
                }
                case PROXY -> {
                    String t = text(value, origin, "a URL string");
                    if (t.equals("off")) {
                        return new Object[] {NODES.textNode("off"), "off"};
                    }
                    String shown = redactUserinfo(t);
                    URI u = uri(t);
                    if (u == null) {
                        throw new Bad(q(shown) + " is not an absolute URL");
                    }
                    if (!u.getScheme().equals("http") && !u.getScheme().equals("https")) {
                        throw new Bad(q(shown) + " must be an http:// or https:// proxy URL, or off");
                    }
                    return new Object[] {NODES.textNode(shown), t};
                }
                case PINS -> {
                    List<String> got = items(value, origin, true);
                    if (got.size() < 2) {
                        throw new Bad("pin at least two keys (the current one and a backup)");
                    }
                    for (String p : got) {
                        boolean ok;
                        try {
                            ok = Base64.getDecoder().decode(p).length == 32;
                        } catch (IllegalArgumentException e) {
                            ok = false;
                        }
                        if (!ok) {
                            throw new Bad(q(p) + " is not a base64 SHA-256 of a public key");
                        }
                    }
                    return new Object[] {array(got), got};
                }
                default -> throw new Bad("region is reserved until the API offers regions; remove it");
            }
        }

        private static ArrayNode array(List<String> items) {
            ArrayNode a = NODES.arrayNode();
            items.forEach(a::add);
            return a;
        }

        private static String text(Object v, String origin, String what) throws Bad {
            if (v instanceof String s) {
                return s;
            }
            if (v instanceof JsonNode node && node.isTextual()) {
                return node.asText();
            }
            if (origin.equals("file") && v instanceof JsonNode node) {
                throw new Bad("must be " + what + ", not " + tomlType(node));
            }
            throw new Bad("must be " + what);
        }

        private static List<String> items(Object v, String origin, boolean comma) throws Bad {
            if (v instanceof String s && !origin.equals("file")) {
                if (comma) {
                    return Arrays.stream(s.split(","))
                            .map(String::strip)
                            .filter(x -> !x.isEmpty())
                            .toList();
                }
                return Arrays.stream(s.strip().split("\\s+"))
                        .filter(x -> !x.isEmpty())
                        .toList();
            }
            if (v instanceof List<?> list) {
                List<String> out = new ArrayList<>();
                for (Object x : list) {
                    if (!(x instanceof String sx)) {
                        throw new Bad("must be a list of strings");
                    }
                    out.add(sx);
                }
                return out;
            }
            if (v instanceof JsonNode node && node.isArray()) {
                List<String> out = new ArrayList<>();
                for (JsonNode x : node) {
                    if (!x.isTextual()) {
                        throw new Bad(
                                origin.equals("file") ? "must be an array of strings" : "must be a list of strings");
                    }
                    out.add(x.asText());
                }
                return out;
            }
            if (origin.equals("file") && v instanceof JsonNode node) {
                throw new Bad("must be an array of strings, not " + tomlType(node));
            }
            throw new Bad("must be a list of strings");
        }

        boolean exists(String path) {
            return read.apply(path) != null;
        }

        @SuppressWarnings("unchecked")
        boolean allowed(String source) {
            Object got = values.get("credential_sources");
            return got == null || ((List<String>) got).contains(source);
        }

        void skip(ArrayNode into, String source, String reason) {
            ObjectNode t = into.addObject();
            t.put("source", source);
            t.put("result", "skipped");
            t.put("reason", reason);
        }

        void used(ArrayNode into, String source) {
            ObjectNode t = into.addObject();
            t.put("source", source);
            t.put("result", "used");
        }

        @SuppressWarnings("unchecked")
        Credential chain(String profile, ObjectNode table) {
            ArrayNode tr = NODES.arrayNode();
            String p = prefix != null ? prefix : "INORBIT_";
            Credential used = null;
            List<String> scopes = values.get("scopes") instanceof List<?> l ? (List<String>) l : List.of();

            // 1. Code.
            if (code.containsKey("token_provider")) {
                used = new Credential("code", "custom");
                used.provider = code.get("token_provider");
            } else if (code.containsKey("token")) {
                used = new Credential("code", "static_token");
                used.token = String.valueOf(code.get("token"));
                show("token", REDACTED, "code", used.token);
            } else if (code.containsKey("key_id")) {
                String codeKey = String.valueOf(code.get("key_id"));
                Object codeSecret = code.get("key_secret");
                Object codeSecretFile = code.get("key_secret_file");
                if (codeSecret != null && codeSecretFile != null) {
                    problem("key_secret", "code", "key_secret and key_secret_file are both set; set one");
                    return null;
                }
                if (codeSecret == null && codeSecretFile == null) {
                    problem("key_secret", "code", "key_id is set without key_secret or key_secret_file");
                    return null;
                }
                if (scopes.isEmpty()) {
                    problem("scopes", "code", "a key needs scopes: set scopes");
                    return null;
                }
                used = new Credential("code", "client_credentials");
                used.keyId = codeKey;
                used.scopes = scopes;
                show("key_id", codeKey, "code", codeKey);
                if (codeSecret != null) {
                    used.keySecret = String.valueOf(codeSecret);
                    show("key_secret", REDACTED, "code", used.keySecret);
                } else {
                    String path;
                    try {
                        path = path(codeSecretFile.toString(), false);
                    } catch (Bad e) {
                        problem("key_secret_file", "code", e.getMessage());
                        return null;
                    }
                    if (!exists(path)) {
                        problem("key_secret_file", "code", "cannot read " + path);
                        return null;
                    }
                    used.keySecretFile = path;
                    show("key_secret_file", path, "code", path);
                }
            } else if (code.containsKey("token_file")) {
                String path;
                try {
                    path = path(code.get("token_file").toString(), false);
                } catch (Bad e) {
                    problem("token_file", "code", e.getMessage());
                    return null;
                }
                if (!exists(path)) {
                    problem("token_file", "code", "cannot read " + path);
                    return null;
                }
                used = new Credential("code", "token_file");
                used.tokenFile = path;
                show("token_file", path, "code", path);
            }
            if (used != null) {
                used(tr, "code");
            } else {
                skip(tr, "code", "none set");
            }

            // 2. The environment.
            if (used == null) {
                String token = var(p + "TOKEN");
                String tokenFile = var(p + "TOKEN_FILE");
                String keyId = var(p + "KEY_ID");
                String secret = var(p + "KEY_SECRET");
                String secretFile = var(p + "KEY_SECRET_FILE");
                if (!allowed("env")) {
                    skip(tr, "env", "not in credential_sources");
                } else if (token == null && tokenFile == null && keyId == null) {
                    skip(tr, "env", p + "TOKEN, " + p + "TOKEN_FILE and " + p + "KEY_ID are not set");
                } else {
                    List<String> forms = new ArrayList<>();
                    if (token != null) {
                        forms.add("TOKEN");
                    }
                    if (tokenFile != null) {
                        forms.add("TOKEN_FILE");
                    }
                    if (keyId != null) {
                        forms.add("KEY_ID");
                    }
                    if (forms.size() > 1) {
                        problem(
                                forms.get(0).toLowerCase(Locale.ROOT),
                                "env " + p + forms.get(0),
                                String.join(
                                                " and ",
                                                forms.stream().map(x -> p + x).toList())
                                        + " are both set; set one credential");
                        return null;
                    }
                    if (token != null) {
                        show("token", REDACTED, "env " + p + "TOKEN", token);
                        used = new Credential("env", "static_token");
                        used.token = token;
                    } else if (tokenFile != null) {
                        String path;
                        try {
                            path = path(tokenFile, false);
                        } catch (Bad e) {
                            problem("token_file", "env " + p + "TOKEN_FILE", e.getMessage());
                            return null;
                        }
                        if (!exists(path)) {
                            problem("token_file", "env " + p + "TOKEN_FILE", "cannot read " + path);
                            return null;
                        }
                        show("token_file", path, "env " + p + "TOKEN_FILE", path);
                        used = new Credential("env", "token_file");
                        used.tokenFile = path;
                    } else {
                        if (secret != null && secretFile != null) {
                            problem(
                                    "key_secret",
                                    "env " + p + "KEY_SECRET",
                                    p + "KEY_SECRET and " + p + "KEY_SECRET_FILE are both set; set one");
                            return null;
                        }
                        if (secret == null && secretFile == null) {
                            problem(
                                    "key_secret",
                                    "env " + p + "KEY_ID",
                                    p + "KEY_ID is set without " + p + "KEY_SECRET or " + p + "KEY_SECRET_FILE");
                            return null;
                        }
                        if (scopes.isEmpty()) {
                            problem("scopes", "env " + p + "KEY_ID", "a key needs scopes: set " + p + "SCOPES");
                            return null;
                        }
                        show("key_id", keyId, "env " + p + "KEY_ID", keyId);
                        used = new Credential("env", "client_credentials");
                        used.keyId = keyId;
                        used.scopes = scopes;
                        if (secret != null) {
                            show("key_secret", REDACTED, "env " + p + "KEY_SECRET", secret);
                            used.keySecret = secret;
                        } else {
                            String path;
                            try {
                                path = path(secretFile, false);
                            } catch (Bad e) {
                                problem("key_secret_file", "env " + p + "KEY_SECRET_FILE", e.getMessage());
                                return null;
                            }
                            if (!exists(path)) {
                                problem("key_secret_file", "env " + p + "KEY_SECRET_FILE", "cannot read " + path);
                                return null;
                            }
                            show("key_secret_file", path, "env " + p + "KEY_SECRET_FILE", path);
                            used.keySecretFile = path;
                        }
                    }
                    used(tr, "env");
                }
            }

            // 3. Workload identity: reserved until the platform offers it (section 5.5).
            if (used == null) {
                skip(
                        tr,
                        "workload",
                        allowed("workload") ? "not offered by the platform yet" : "not in credential_sources");
            }

            // 4. The config file's profile table.
            if (used == null) {
                String label = layers.isEmpty() ? "" : layers.get(0).label();
                if (!allowed("file")) {
                    skip(tr, "file", "not in credential_sources");
                } else if (filePath == null) {
                    skip(tr, "file", "no config file was read");
                } else if (profile == null) {
                    skip(tr, "file", "no profile chosen");
                } else if (table != null && (table.has("token_file") || table.has("key_id"))) {
                    if (table.has("token_file") && table.has("key_id")) {
                        problem("token_file", label, "token_file and key_id are both set; set one credential");
                        return null;
                    }
                    JsonNode tf = table.get("token_file");
                    JsonNode kid = table.get("key_id");
                    if (tf != null && tf.isTextual()) {
                        String path;
                        try {
                            path = path(tf.asText(), true);
                        } catch (Bad e) {
                            problem("token_file", label, e.getMessage());
                            return null;
                        }
                        if (!exists(path)) {
                            problem("token_file", label, "cannot read " + path);
                            return null;
                        }
                        show("token_file", path, label, path);
                        used = new Credential("file", "token_file");
                        used.tokenFile = path;
                    } else if (kid != null && kid.isTextual()) {
                        if (table.has("key_secret")) {
                            return null; // Already reported: secrets are not allowed in the file.
                        }
                        JsonNode sf = table.get("key_secret_file");
                        if (sf == null || !sf.isTextual()) {
                            problem("key_secret", label, "key_id is set without key_secret_file");
                            return null;
                        }
                        if (scopes.isEmpty()) {
                            problem("scopes", label, "a key needs scopes: set scopes in the profile's table");
                            return null;
                        }
                        String path;
                        try {
                            path = path(sf.asText(), true);
                        } catch (Bad e) {
                            problem("key_secret_file", label, e.getMessage());
                            return null;
                        }
                        if (!exists(path)) {
                            problem("key_secret_file", label, "cannot read " + path);
                            return null;
                        }
                        show("key_id", kid.asText(), label, kid.asText());
                        show("key_secret_file", path, label, path);
                        used = new Credential("file", "client_credentials");
                        used.keyId = kid.asText();
                        used.keySecretFile = path;
                        used.scopes = scopes;
                    } else {
                        problem("token_file", label, "must be a path string");
                        return null;
                    }
                    used(tr, "file");
                } else {
                    skip(tr, "file", "profile " + profile + " sets no token_file or key_id");
                }
            }

            // 5. The iohr login.
            if (used == null) {
                String program = values.get("cli_path") instanceof String cp ? cp : "iohr";
                if (!allowed("cli")) {
                    skip(tr, "cli", "not in credential_sources");
                } else if (profile == null) {
                    skip(tr, "cli", "skipped, no profile chosen");
                } else if (table == null || !table.has("kind")) {
                    skip(tr, "cli", "profile " + profile + " was not made by iohr login");
                } else if (!found.test(program)) {
                    skip(tr, "cli", program.equals("iohr") ? "iohr not found on PATH" : "iohr not found at " + program);
                } else {
                    used(tr, "cli");
                    used = new Credential("cli", "cli");
                    used.cliPath = program;
                    used.profile = profile;
                }
            }

            if (used == null) {
                StringBuilder m = new StringBuilder(
                        "no credentials found for profile " + q(profile != null ? profile : "default") + "; tried:");
                for (JsonNode t : tr) {
                    m.append("\n  ")
                            .append(t.path("source").asText())
                            .append(": ")
                            .append(t.path("reason").asText(""));
                }
                m.append("\nSet ")
                        .append(p)
                        .append("KEY_ID, ")
                        .append(p)
                        .append("KEY_SECRET and ")
                        .append(p)
                        .append("SCOPES, or ")
                        .append(p)
                        .append("TOKEN, or run `iohr login`.");
                problem("credential", "", m.toString());
                return null;
            }
            if (!used.kind.equals("client_credentials") && !used.kind.equals("custom") && settings.has("scopes")) {
                JsonNode s = settings.remove("scopes");
                values.remove("scopes");
                ignore("scopes", s.path("source").asText(), "not used by this credential");
            }
            tried = tr;
            return used;
        }

        void crossChecks() {
            JsonNode trust = settings.get("system_trust");
            if (trust != null && !trust.path("value").asBoolean(true) && !settings.has("ca_bundle")) {
                problem(
                        "system_trust",
                        trust.path("source").asText(),
                        "system_trust = false needs a ca_bundle to trust instead");
            }
            JsonNode cert = settings.get("client_cert");
            JsonNode key = settings.get("client_key");
            if (cert != null && key == null) {
                problem("client_key", cert.path("source").asText(), "client_cert needs client_key");
            }
            if (cert == null && key != null) {
                problem("client_cert", key.path("source").asText(), "client_key needs client_cert");
            }
            for (String name : List.of("ca_bundle", "client_cert", "client_key")) {
                JsonNode v = settings.get(name);
                if (v != null && !exists(v.path("value").asText())) {
                    problem(
                            name,
                            v.path("source").asText(),
                            "cannot read " + v.path("value").asText());
                }
            }
        }
    }

    /** The TOML reader, made once. */
    private static final class Toml {
        static final TomlMapper MAPPER = new TomlMapper();

        private Toml() {}
    }
}
