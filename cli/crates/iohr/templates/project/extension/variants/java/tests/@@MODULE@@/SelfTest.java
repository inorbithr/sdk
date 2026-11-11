package @@MODULE@@;

import com.sun.net.httpserver.HttpServer;
import java.io.IOException;
import java.net.StandardProtocolFamily;
import java.net.URI;
import java.net.UnixDomainSocketAddress;
import java.net.http.HttpClient;
import java.net.http.HttpRequest;
import java.net.http.HttpResponse;
import java.nio.ByteBuffer;
import java.nio.channels.ServerSocketChannel;
import java.nio.channels.SocketChannel;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.HashMap;
import java.util.Map;
import java.util.concurrent.atomic.AtomicReference;
import java.util.stream.Stream;

/**
 * The self-test: `java -cp $CACHE/classes @@MODULE@@.SelfTest` runs one heartbeat with a STATE and
 * CACHE of its own, then checks what it wrote and where. Plain JDK, so there is nothing to download.
 */
public final class SelfTest {
    private static int failed;

    private SelfTest() {}

    private static void check(boolean ok, String what) {
        if (!ok) {
            failed++;
        }
        System.out.println((ok ? "ok   " : "FAIL ") + what);
    }

    private static Map<String, String> env(Path state, Path cache, String... more) {
        Map<String, String> e = new HashMap<>();
        e.put("STATE", state.toString());
        e.put("CACHE", cache.toString());
        for (int i = 0; i + 1 < more.length; i += 2) {
            e.put(more[i], more[i + 1]);
        }
        return e;
    }

    private static long files(Path dir) throws IOException {
        try (Stream<Path> s = Files.list(dir)) {
            return s.count();
        }
    }

    public static void main(String[] args) throws Exception {
        Path state = Files.createTempDirectory("state-");
        Path cache = Files.createTempDirectory("cache-");
        long before = files(Path.of("."));

        check(Main.run(env(state, cache, "IOHR_EXT_ONCE", "1")) == 0, "one heartbeat exits 0");
        String beat = Files.readString(state.resolve("heartbeat.json"));
        check(beat.contains("\"id\":\"@@ID@@\"") && beat.contains("\"state\":\"ok\"") && beat.contains("\"beats\":1"),
                "the heartbeat says who and ok");
        // Granted when you are signed in and the channel is up, refused when not: either way it answered.
        check(beat.contains("\"token\":\"granted\"") || beat.contains("\"token\":\"refused\""), "the token ask was answered");
        check(files(Path.of(".")) == before, "nothing was written into the project folder");
        check(Main.run(Map.of()) == 2, "no STATE and CACHE is an error, exit 2");

        boolean refused = false;
        try {
            Channel.token(new String[] {"tokens:write"}, Map.of());
        } catch (Channel.Refused e) {
            refused = true;
        }
        check(refused, "without the channel the token ask is refused");

        // A fake channel: the one scope is granted, any other is refused.
        Path socket = Files.createTempDirectory("chan-").resolve("t.sock");
        try (ServerSocketChannel server = ServerSocketChannel.open(StandardProtocolFamily.UNIX)) {
            server.bind(UnixDomainSocketAddress.of(socket));
            Thread t = new Thread(() -> {
                for (int i = 0; i < 2; i++) {
                    try (SocketChannel c = server.accept()) {
                        ByteBuffer in = ByteBuffer.allocate(4096);
                        c.read(in);
                        boolean ok = new String(in.array(), 0, in.position(), StandardCharsets.UTF_8).contains("identity:read");
                        String body = ok
                                ? "{\"access_token\":\"t\",\"expires_at\":\"x\"}"
                                : "{\"error\":\"scope\",\"message\":\"not declared\"}";
                        String answer = "HTTP/1.1 " + (ok ? "200 OK" : "403 Forbidden") + "\r\ncontent-length: "
                                + body.length() + "\r\n\r\n" + body;
                        c.write(ByteBuffer.wrap(answer.getBytes(StandardCharsets.UTF_8)));
                    } catch (IOException e) {
                        return;
                    }
                }
            });
            t.start();
            Map<String, String> e = Map.of("IOHR_EXT_TOKEN_SOCKET", socket.toString());
            try {
                check(Channel.token(Main.SCOPES, e).equals("t"), "a declared scope is granted by the channel");
            } catch (Channel.Refused r) {
                check(false, "a declared scope is granted by the channel: " + r.getMessage());
            }
            String why = "";
            try {
                Channel.token(new String[] {"tokens:write"}, e);
            } catch (Channel.Refused r) {
                why = r.getMessage();
            }
            check(why.contains("403"), "an undeclared scope is refused by the channel");
            t.join();
        }

        HttpServer panel = Main.serve(env(state, cache, "PANEL_FILE", "panel/index.html"), state, new AtomicReference<>(beat));
        String base = "http://127.0.0.1:" + panel.getAddress().getPort();
        HttpClient http = HttpClient.newHttpClient();
        HttpResponse<String> api = http.send(HttpRequest.newBuilder(URI.create(base + "/api/heartbeat")).build(),
                HttpResponse.BodyHandlers.ofString());
        check(api.statusCode() == 200 && api.body().contains("\"beats\":1"), "the panel serves the heartbeat as JSON");
        HttpResponse<String> page = http.send(HttpRequest.newBuilder(URI.create(base + "/")).build(),
                HttpResponse.BodyHandlers.ofString());
        check(page.body().contains("/api/heartbeat"), "the panel page reads /api/heartbeat");
        HttpResponse<String> nope = http.send(HttpRequest.newBuilder(URI.create(base + "/nope")).build(),
                HttpResponse.BodyHandlers.ofString());
        check(nope.statusCode() == 404, "anything else is a 404");
        panel.stop(0);

        System.out.println(failed == 0 ? "selftest passed" : failed + " check(s) failed");
        System.exit(failed == 0 ? 0 : 1);
    }
}
