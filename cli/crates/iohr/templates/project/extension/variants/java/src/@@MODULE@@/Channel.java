package @@MODULE@@;

import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.net.StandardProtocolFamily;
import java.net.UnixDomainSocketAddress;
import java.nio.ByteBuffer;
import java.nio.channels.SocketChannel;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Map;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

/**
 * Tokens from iohr's extension channel (the SDK's ADR 0012; PRD 0084 slice 1).
 *
 * <p>iohr passes IOHR_EXT_TOKEN_SOCKET (a 0600 Unix socket). The program sends
 * {@code POST /token {"scopes": [...]}} and gets {@code {"access_token", "expires_at"}} for scopes its
 * manifest declares, and a refusal for any other. It is the only credential this project has:
 * never read IOHR_TOKEN or run `iohr auth token`.
 */
public final class Channel {
    private static final int MAX_ANSWER = 64 * 1024;
    private static final long WAIT_MS = 10_000;
    private static final Pattern STRING_FIELD = Pattern.compile("\"%s\"\\s*:\\s*\"((?:[^\"\\\\]|\\\\.)*)\"");

    private Channel() {}

    /** What {@link #token} throws when iohr refuses, or its channel cannot be reached. */
    public static final class Refused extends Exception {
        private static final long serialVersionUID = 1L;

        public Refused(String message) {
            super(message);
        }
    }

    private static String field(String json, String key) {
        Matcher m = Pattern.compile(String.format(STRING_FIELD.pattern(), key)).matcher(json);
        return m.find() ? m.group(1) : "";
    }

    /** Returns an access token for the scopes; the message of {@link Refused} says iohr's reason. */
    public static String token(String[] scopes, Map<String, String> env) throws Refused {
        String path = env.getOrDefault("IOHR_EXT_TOKEN_SOCKET", "").strip();
        if (path.isEmpty()) {
            throw new Refused("IOHR_EXT_TOKEN_SOCKET is not set: run this with iohr ext dev");
        }
        StringBuilder list = new StringBuilder();
        for (String s : scopes) {
            list.append(list.length() == 0 ? "" : ",").append('"').append(s).append('"');
        }
        byte[] body = ("{\"scopes\":[" + list + "]}").getBytes(StandardCharsets.UTF_8);
        byte[] head = ("POST /token HTTP/1.1\r\nhost: iohr\r\ncontent-type: application/json\r\ncontent-length: "
                + body.length + "\r\nconnection: close\r\n\r\n").getBytes(StandardCharsets.UTF_8);
        ByteArrayOutputStream raw = new ByteArrayOutputStream();
        long deadline = System.currentTimeMillis() + WAIT_MS;
        try (SocketChannel ch = SocketChannel.open(StandardProtocolFamily.UNIX)) {
            ch.connect(UnixDomainSocketAddress.of(Path.of(path)));
            // One buffer, written until empty: the channel sees the head and the body together.
            ByteBuffer request = ByteBuffer.allocate(head.length + body.length).put(head).put(body).flip();
            while (request.hasRemaining()) {
                ch.write(request);
            }
            ch.configureBlocking(false);
            ByteBuffer buf = ByteBuffer.allocate(8192);
            while (raw.size() < MAX_ANSWER) {
                buf.clear();
                int n = ch.read(buf);
                if (n < 0) {
                    break;
                }
                if (n == 0) {
                    if (System.currentTimeMillis() > deadline) {
                        throw new IOException("no answer in 10 s");
                    }
                    Thread.sleep(10);
                    continue;
                }
                raw.write(buf.array(), 0, n);
            }
        } catch (IOException e) {
            throw new Refused("cannot reach iohr's token channel: " + e.getMessage());
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            throw new Refused("interrupted while asking iohr for a token");
        }
        String text = raw.toString(StandardCharsets.UTF_8);
        int cut = text.indexOf("\r\n\r\n");
        String top = cut < 0 ? text : text.substring(0, cut);
        String payload = cut < 0 ? "" : text.substring(cut + 4);
        String[] fields = top.split(" ");
        String status = fields.length > 1 ? fields[1] : "no answer";
        if (!status.equals("200")) {
            throw new Refused(status + " " + field(payload, "error") + ": " + field(payload, "message"));
        }
        String access = field(payload, "access_token");
        if (access.isEmpty()) {
            throw new Refused("iohr's answer holds no token");
        }
        return access;
    }
}
