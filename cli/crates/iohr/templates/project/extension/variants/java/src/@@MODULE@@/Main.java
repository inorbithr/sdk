package @@MODULE@@;

import com.sun.net.httpserver.HttpExchange;
import com.sun.net.httpserver.HttpServer;
import java.io.IOException;
import java.net.InetAddress;
import java.net.InetSocketAddress;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardCopyOption;
import java.nio.file.attribute.PosixFilePermissions;
import java.time.Instant;
import java.time.temporal.ChronoUnit;
import java.util.Map;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicReference;

/**
 * @@ID@@: the demo project's private macOS service.
 *
 * <p>Every minute it writes $STATE/heartbeat.json (the health InOrbit reads) and asks iohr's token
 * channel for a token with the one scope its manifest declares. It also serves a tiny panel on
 * 127.0.0.1 that InOrbit Desktop can show as a pane: the page reads /api/heartbeat.
 *
 * <p>Run it with `java -cp $CACHE/classes @@MODULE@@.Main` under `iohr ext dev`'s STATE and CACHE, or
 * once with IOHR_EXT_ONCE=1 as the self-test does. JDK only (java.net.http is not even needed).
 */
public final class Main {
    public static final String ID = "@@ID@@";
    public static final String VERSION = "0.1.0";
    public static final String[] SCOPES = {"identity:read"};

    private Main() {}

    private static String quote(String s) {
        return "\"" + s.replace("\\", "\\\\").replace("\"", "\\\"") + "\"";
    }

    private static void writeAtomic(Path state, String name, String text) throws IOException {
        Path tmp = state.resolve("." + name + ".tmp");
        Files.writeString(tmp, text);
        // Not every file system has POSIX permissions (NTFS does not): there the user's profile guards it.
        if (tmp.getFileSystem().supportedFileAttributeViews().contains("posix")) {
            Files.setPosixFilePermissions(tmp, PosixFilePermissions.fromString("rw-------"));
        }
        Files.move(tmp, state.resolve(name), StandardCopyOption.ATOMIC_MOVE, StandardCopyOption.REPLACE_EXISTING);
    }

    /** One heartbeat: asks for a token (kept in memory only, never printed), writes heartbeat.json. */
    public static String heartbeat(Map<String, String> env, Path state, int beats) throws IOException {
        String tok = "granted";
        try {
            Channel.token(SCOPES, env);
        } catch (Channel.Refused e) {
            System.err.println("token refused: " + e.getMessage());
            tok = "refused";
        }
        String now = Instant.now().truncatedTo(ChronoUnit.SECONDS).toString();
        String beat = "{\"id\":" + quote(ID) + ",\"version\":" + quote(VERSION) + ",\"state\":\"ok\",\"at\":" + quote(now)
                + ",\"beats\":" + beats + ",\"token\":" + quote(tok) + ",\"dev\":" + "1".equals(env.get("IOHR_EXT_DEV")) + "}";
        writeAtomic(state, "heartbeat.json", beat);
        System.out.println("heartbeat " + beats + " at " + now + " (token " + tok + ")");
        return beat;
    }

    /** Where the panel page is: PANEL_FILE, or panel/index.html beside the classes, or in the folder it runs from. */
    private static Path panelFile(Map<String, String> env) {
        String named = env.getOrDefault("PANEL_FILE", "");
        if (!named.isEmpty()) {
            return Path.of(named);
        }
        try {
            Path code = Path.of(Main.class.getProtectionDomain().getCodeSource().getLocation().toURI());
            Path beside = code.resolveSibling("panel").resolve("index.html");
            if (Files.exists(beside)) {
                return beside;
            }
        } catch (Exception e) {
            // Fall through to the working folder.
        }
        return Path.of("panel", "index.html");
    }

    private static void send(HttpExchange x, int status, String type, byte[] body) throws IOException {
        x.getResponseHeaders().set("content-type", type);
        x.sendResponseHeaders(status, body.length == 0 ? -1 : body.length);
        if (body.length > 0) {
            x.getResponseBody().write(body);
        }
        x.close();
    }

    /** Listens on 127.0.0.1 only; the port is PANEL_PORT or one the system picks, written to $STATE/panel.json. */
    public static HttpServer serve(Map<String, String> env, Path state, AtomicReference<String> latest) throws IOException {
        String raw = env.getOrDefault("PANEL_PORT", "");
        int port;
        try {
            port = raw.isEmpty() ? 0 : Integer.parseInt(raw);
        } catch (NumberFormatException e) {
            throw new IOException("PANEL_PORT is not a number: " + raw);
        }
        HttpServer server = HttpServer.create(new InetSocketAddress(InetAddress.getLoopbackAddress(), port), 0);
        server.createContext("/", x -> {
            String path = x.getRequestURI().getPath();
            if (!x.getRequestMethod().equals("GET") || !(path.equals("/") || path.equals("/api/heartbeat"))) {
                send(x, 404, "text/plain", new byte[0]);
            } else if (path.equals("/api/heartbeat")) {
                send(x, 200, "application/json", latest.get().getBytes(StandardCharsets.UTF_8));
            } else {
                Path page = panelFile(env);
                byte[] html = Files.exists(page)
                        ? Files.readAllBytes(page)
                        : "<p>panel/index.html is missing</p>".getBytes(StandardCharsets.UTF_8);
                send(x, 200, "text/html; charset=utf-8", html);
            }
        });
        server.start();
        String url = "http://127.0.0.1:" + server.getAddress().getPort() + "/";
        writeAtomic(state, "panel.json", "{\"id\":" + quote(ID) + ",\"url\":" + quote(url) + "}");
        System.out.println("panel at " + url);
        return server;
    }

    /** Runs until SIGINT or SIGTERM (or once, with IOHR_EXT_ONCE) and returns the exit code. */
    public static int run(Map<String, String> env) {
        String state = env.getOrDefault("STATE", "");
        if (state.isEmpty() || env.getOrDefault("CACHE", "").isEmpty()) {
            System.err.println("STATE and CACHE are not set: run this with iohr ext dev");
            return 2;
        }
        int every = 60;
        try {
            int parsed = Integer.parseInt(env.getOrDefault("HEARTBEAT_EVERY", ""));
            every = parsed > 0 ? parsed : 60;
        } catch (NumberFormatException e) {
            // The default stands.
        }
        boolean once = !env.getOrDefault("IOHR_EXT_ONCE", "").isEmpty();
        System.out.println(ID + " " + VERSION + " starting; state in " + state);
        AtomicReference<String> latest = new AtomicReference<>("{}");
        CountDownLatch stop = new CountDownLatch(1);
        CountDownLatch done = new CountDownLatch(1);
        HttpServer server = null;
        try {
            Path dir = Path.of(state);
            if (!once) {
                server = serve(env, dir, latest);
                // A stop signal ends the JVM with 143; wait for the loop to finish, then leave with 0.
                Runtime.getRuntime().addShutdownHook(new Thread(() -> {
                    stop.countDown();
                    try {
                        done.await(5, TimeUnit.SECONDS);
                    } catch (InterruptedException e) {
                        Thread.currentThread().interrupt();
                    }
                    Runtime.getRuntime().halt(0);
                }));
            }
            for (int beats = 1;; beats++) {
                latest.set(heartbeat(env, dir, beats));
                if (once) {
                    return 0;
                }
                if (stop.await(every, TimeUnit.SECONDS)) {
                    System.out.println("stopping");
                    return 0;
                }
            }
        } catch (IOException e) {
            System.err.println(e.getMessage());
            return 1;
        } catch (InterruptedException e) {
            return 0;
        } finally {
            if (server != null) {
                server.stop(0);
            }
            done.countDown();
        }
    }

    public static void main(String[] args) {
        System.exit(run(System.getenv()));
    }
}
