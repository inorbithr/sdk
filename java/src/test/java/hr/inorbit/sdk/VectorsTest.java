package hr.inorbit.sdk;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertTrue;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.node.ObjectNode;
import hr.inorbit.sdk.errors.ConfigException;
import java.io.File;
import java.io.IOException;
import java.net.URI;
import java.net.http.HttpClient;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import org.junit.jupiter.api.Assumptions;
import org.junit.jupiter.api.BeforeAll;
import org.junit.jupiter.api.Tag;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

/**
 * The conformance vectors ({@code conformance/vectors/}, docs/config.md section 9.2): configuration
 * resolution, config file paths, {@code no_proxy}, rate-limit headers and durations, each through
 * the runtime's own functions. The replay binary prints them as JSON ({@code replay vectors}), so
 * no YAML reader is needed; without it the tests skip, unless {@code IOHR_TEST_REQUIRE_REPLAY} is
 * set.
 */
@Tag("vectors")
class VectorsTest {

    private static final ObjectMapper JSON = new ObjectMapper();
    private static final Map<String, List<JsonNode>> BY_KIND = new HashMap<>();

    @BeforeAll
    static void read() throws Exception {
        Path root = Path.of("..").toAbsolutePath().normalize();
        String exe = System.getProperty("os.name", "").startsWith("Windows") ? ".exe" : "";
        Path bin = root.resolve("conformance/server/bin/replay" + exe);
        if (!Files.isExecutable(bin)) {
            String note = "vectors: no replay binary; run `mise run conformance:server:build`";
            assertTrue(System.getenv("IOHR_TEST_REQUIRE_REPLAY") == null, note);
            Assumptions.abort(note);
        }
        Process p = new ProcessBuilder(
                        bin.toString(),
                        "vectors",
                        root.resolve("conformance/vectors").toString())
                .redirectError(ProcessBuilder.Redirect.INHERIT)
                .start();
        JsonNode all = JSON.readTree(p.getInputStream());
        assertEquals(0, p.waitFor());
        for (JsonNode v : all.path("vectors")) {
            if (contains(v.path("pending"), "java")) {
                continue;
            }
            BY_KIND.computeIfAbsent(v.path("kind").asText(), k -> new ArrayList<>())
                    .add(v);
        }
    }

    private static boolean contains(JsonNode list, String value) {
        for (JsonNode v : list) {
            if (v.asText().equals(value)) {
                return true;
            }
        }
        return false;
    }

    private static List<JsonNode> vectors(String kind) {
        List<JsonNode> got = BY_KIND.getOrDefault(kind, List.of());
        assertTrue(!got.isEmpty(), "no " + kind + " vectors ran");
        return got;
    }

    /** {@code want} is in {@code got}: objects by key, lists element by element, paths by {@code /}. */
    static boolean subset(JsonNode want, JsonNode got) {
        if (got == null) {
            return false;
        }
        if (want.isObject()) {
            if (!got.isObject()) {
                return false;
            }
            for (Map.Entry<String, JsonNode> e : want.properties()) {
                if (!subset(e.getValue(), got.get(e.getKey()))) {
                    return false;
                }
            }
            return true;
        }
        if (want.isArray()) {
            if (!got.isArray() || got.size() != want.size()) {
                return false;
            }
            for (int i = 0; i < want.size(); i++) {
                if (!subset(want.get(i), got.get(i))) {
                    return false;
                }
            }
            return true;
        }
        if (want.isTextual() && got.isTextual()) {
            return want.asText().replace('\\', '/').equals(got.asText().replace('\\', '/'));
        }
        if (want.isNumber() && got.isNumber()) {
            return want.asLong() == got.asLong();
        }
        return want.equals(got);
    }

    private static JsonNode substitute(JsonNode v, String dir, String file) throws IOException {
        String text = JSON.writeValueAsString(v);
        String d = JSON.writeValueAsString(dir);
        String f = JSON.writeValueAsString(file);
        text = text.replace("{file}", f.substring(1, f.length() - 1)).replace("{dir}", d.substring(1, d.length() - 1));
        return JSON.readTree(text);
    }

    private static Map<String, String> strings(JsonNode obj, String dir) {
        Map<String, String> out = new LinkedHashMap<>();
        obj.properties().forEach(e -> out.put(e.getKey(), e.getValue().asText().replace("{dir}", dir)));
        return out;
    }

    private static Object codeValue(JsonNode v) {
        if (v.isArray()) {
            List<String> l = new ArrayList<>();
            v.forEach(x -> l.add(x.asText()));
            return l;
        }
        if (v.isBoolean()) {
            return v.asBoolean();
        }
        if (v.isIntegralNumber()) {
            return v.asInt();
        }
        return v.asText();
    }

    /** A directory holding an executable {@code iohr} (with Windows' extension there), for {@code PATH}. */
    private static String fakeIohr(Path dir) throws IOException {
        Path bin = Files.createDirectories(dir.resolve("bin"));
        boolean windows = File.pathSeparatorChar == ';';
        Path exe = bin.resolve(windows ? "iohr.bat" : "iohr");
        Files.writeString(exe, windows ? "@echo off\r\n" : "#!/bin/sh\n");
        exe.toFile().setExecutable(true);
        return bin.toString();
    }

    private static List<String> runConfig(JsonNode v, Path tmp) throws IOException {
        String d = tmp.toString();
        JsonNode in = v.path("input");
        String os = in.path("os").asText("linux");
        Map<String, String> env = strings(in.path("env"), d);
        String home = in.path("home").asBoolean(false) ? d + File.separator + "home" : "";
        if (!home.isEmpty()) {
            Files.createDirectories(Path.of(home));
        }
        for (Map.Entry<String, JsonNode> f : in.path("files").properties()) {
            Path p = tmp.resolve(f.getKey());
            Files.createDirectories(p.getParent());
            Files.writeString(p, f.getValue().asText());
        }
        Map<String, Object> code = new LinkedHashMap<>();
        in.path("code").properties().forEach(e -> code.put(e.getKey(), codeValue(e.getValue())));
        String file = d + File.separator + "config.toml";
        if (in.has("config_file")) {
            if (!home.isEmpty()) {
                Map<String, String> without = new HashMap<>(env);
                without.remove("INORBIT_CONFIG_FILE");
                file = Config.configPath(os, without, home, null).path();
                Files.createDirectories(Path.of(file).getParent());
            } else {
                code.put("config_file", file);
            }
            Files.writeString(Path.of(file), in.path("config_file").asText());
        }
        if ("custom".equals(code.get("http_client"))) {
            code.put("http_client", HttpClient.newHttpClient());
        }
        if ("present".equals(in.path("cli").asText())) {
            env.put("PATH", fakeIohr(tmp));
        }
        JsonNode expect = substitute(v.path("expect"), d, file);
        LoadOptions options =
                LoadOptions.builder().env(env).os(os).home(home).cwd(d).build();
        List<String> problems = new ArrayList<>();
        ObjectNode doc = null;
        ConfigException error = null;
        String shown;
        try {
            doc = Config.resolve(
                            code,
                            options,
                            in.path("profile_type").isTextual()
                                    ? in.path("profile_type").asText()
                                    : null,
                            null,
                            null)
                    .doc();
            shown = doc.toString();
        } catch (ConfigException e) {
            error = e;
            shown = e.getMessage() + " " + e.problems();
        }
        for (JsonNode x : expect.path("excludes")) {
            if (shown.contains(x.asText())) {
                problems.add(x.asText() + " appears in " + shown);
            }
        }
        if (expect.has("error")) {
            JsonNode want = expect.path("error");
            if (error == null) {
                problems.add("want an error, got " + shown);
                return problems;
            }
            if (want.has("problems")) {
                List<ConfigException.Problem> have = error.problems();
                if (want.path("problems").size() != have.size()) {
                    problems.add("want " + want.path("problems").size() + " problems, got " + have);
                }
                for (int i = 0; i < Math.min(have.size(), want.path("problems").size()); i++) {
                    JsonNode w = want.path("problems").get(i);
                    ConfigException.Problem h = have.get(i);
                    if (w.has("setting") && !w.path("setting").asText().equals(h.setting())) {
                        problems.add("setting " + w.path("setting") + " != " + h);
                    }
                    if (w.has("source") && !w.path("source").asText().equals(h.source())) {
                        problems.add("source " + w.path("source") + " != " + h);
                    }
                    if (w.has("message_contains")
                            && !h.message().contains(w.path("message_contains").asText())) {
                        problems.add("message lacks " + w.path("message_contains") + ": " + h);
                    }
                }
            }
            for (JsonNode p : want.path("message_contains")) {
                if (!error.getMessage().contains(p.asText())) {
                    problems.add("the error lacks " + p + ":\n" + error.getMessage());
                }
            }
            return problems;
        }
        if (error != null) {
            problems.add("unexpected error:\n" + error.getMessage());
            return problems;
        }
        for (String key : List.of("profile", "settings", "credential", "pipeline")) {
            if (expect.has(key) && !subset(expect.get(key), doc.get(key))) {
                problems.add(key + ": want " + expect.get(key) + ", got " + doc.get(key));
            }
        }
        if (expect.has("config_file")) {
            JsonNode w = expect.get("config_file");
            JsonNode h = doc.get("config_file");
            if (w.isNull() != h.isNull() || (!w.isNull() && !subset(w, h))) {
                problems.add("config_file: want " + w + ", got " + h);
            }
        }
        for (JsonNode a : expect.path("settings_absent")) {
            if (doc.path("settings").has(a.asText())) {
                problems.add(a + " should not be in settings");
            }
        }
        for (JsonNode w : expect.path("ignored")) {
            boolean found = false;
            for (JsonNode h : doc.path("ignored")) {
                found |= subset(w, h);
            }
            if (!found) {
                problems.add("ignored lacks " + w + ": " + doc.path("ignored"));
            }
        }
        return problems;
    }

    @Test
    void configVectorsResolveAsTheySay(@TempDir Path tmp) throws IOException {
        List<String> failed = new ArrayList<>();
        int n = 0;
        for (JsonNode v : vectors("config")) {
            Path dir = Files.createDirectories(tmp.resolve("v" + n++));
            List<String> problems = runConfig(v, dir.toRealPath());
            if (!problems.isEmpty()) {
                failed.add(v.path("name").asText() + ":\n  " + String.join("\n  ", problems));
            }
        }
        assertEquals(List.of(), failed, String.join("\n", failed));
    }

    @Test
    void theConfigFileIsWhereTheCommandLineKeepsIt() {
        for (JsonNode v : vectors("config-path")) {
            for (JsonNode c : v.path("checks")) {
                Map<String, String> env = strings(c.path("env"), "");
                String code = c.path("code").path("config_file").isTextual()
                        ? c.path("code").path("config_file").asText()
                        : null;
                String home = c.path("home").isNull() ? null : c.path("home").asText();
                Config.Located got = Config.configPath(c.path("os").asText(), env, home, code);
                String want =
                        c.path("expect").isNull() ? null : c.path("expect").asText();
                assertEquals(
                        want, got == null ? null : got.path(), c.path("summary").asText());
            }
        }
    }

    @Test
    void durationsHaveOneSyntax() {
        for (JsonNode v : vectors("durations")) {
            for (JsonNode c : v.path("checks")) {
                long want = c.path("expect").isNumber() ? c.path("expect").asLong() : -1;
                assertEquals(
                        want,
                        Config.parseDuration(c.path("value").asText()),
                        c.path("value").asText());
            }
        }
    }

    @Test
    void noProxyChoosesTheProxy() {
        for (JsonNode v : vectors("no-proxy")) {
            for (JsonNode c : v.path("checks")) {
                Map<String, Object> code = new LinkedHashMap<>();
                code.put("token", "t");
                code.put("config_file", "off");
                c.path("code").properties().forEach(e -> code.put(e.getKey(), codeValue(e.getValue())));
                Object got;
                try {
                    Config.Resolution r = Config.resolve(
                            code,
                            LoadOptions.builder()
                                    .env(strings(c.path("env"), ""))
                                    .home("")
                                    .build(),
                            null,
                            null,
                            null);
                    String source = r.doc()
                            .path("settings")
                            .path("proxy")
                            .path("source")
                            .asText("");
                    URI p = NoProxy.of(r.values(), source)
                            .proxyFor(URI.create(c.path("url").asText()));
                    got = p == null ? null : p.toString();
                } catch (ConfigException e) {
                    got = "{error: config}";
                }
                JsonNode e = c.path("expect");
                Object want = e.isNull() ? null : e.isObject() ? "{error: config}" : e.asText();
                assertEquals(want, got, c.path("summary").asText());
            }
        }
    }

    @Test
    void rateLimitHeadersGiveTheSnapshot() {
        for (JsonNode v : vectors("rate-limit")) {
            for (JsonNode c : v.path("checks")) {
                Map<String, String> h = new HashMap<>();
                c.path("headers")
                        .properties()
                        .forEach(e -> h.put(
                                e.getKey().toLowerCase(Locale.ROOT),
                                e.getValue().asText()));
                RateLimit snap = RateLimit.parse(h);
                JsonNode got = null;
                if (snap != null) {
                    ObjectNode o = JSON.createObjectNode();
                    snap.limit().ifPresent(x -> o.put("limit", x));
                    snap.remaining().ifPresent(x -> o.put("remaining", x));
                    snap.reset().ifPresent(x -> o.put("reset_ms", x.toMillis()));
                    snap.policy().ifPresent(p -> {
                        ObjectNode po = o.putObject("policy");
                        po.put("name", p.name());
                        if (p.quota() != null) {
                            po.put("quota", p.quota());
                        }
                        if (p.window() != null) {
                            po.put("window_ms", p.window().toMillis());
                        }
                    });
                    got = o;
                }
                JsonNode want = c.path("expect");
                if (want.isNull()) {
                    assertEquals(null, got, c.path("summary").asText());
                } else {
                    assertTrue(got != null && subset(want, got) && subset(got, want), c.path("summary") + ": " + got);
                }
            }
        }
    }

    @Test
    void loadUsesTheInjectedEnvironmentOnly() {
        Client client = Client.builder()
                .load(LoadOptions.builder()
                        .env(Map.of("INORBIT_TOKEN", "t", "INORBIT_TIMEOUT", "7s"))
                        .home("")
                        .build());
        JsonNode doc = client.config().describe();
        assertEquals("7s", doc.path("settings").path("timeout").path("value").asText());
        assertEquals("env", doc.path("credential").path("source").asText());
        assertTrue(!client.config().toJson().contains("\"t\""), "the token is redacted");
        assertTrue(new String(client.config().toJson().getBytes(StandardCharsets.UTF_8), StandardCharsets.UTF_8)
                .contains("<redacted>"));
    }
}
