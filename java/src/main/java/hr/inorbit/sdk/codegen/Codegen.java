package hr.inorbit.sdk.codegen;

import hr.inorbit.sdk.Pages;
import java.nio.charset.StandardCharsets;
import java.util.function.Function;

/**
 * What a generated surface uses from the runtime, and nothing else does. This class is a contract
 * with {@code iohr sdk generate}: a change that breaks generated code replaces {@link #V1}, so a
 * surface generated for another version fails to compile.
 */
public final class Codegen {

    /** The surface contract this runtime implements. */
    public static final int VERSION = 1;

    /**
     * Present only while the contract is version 1; every generated surface refers to it, so one
     * built for another version does not compile against this runtime.
     */
    public static final int V1 = VERSION;

    private static final byte[] HEX = "0123456789ABCDEF".getBytes(StandardCharsets.US_ASCII);

    private Codegen() {}

    /**
     * Percent-encodes {@code value} as one path segment: every byte but the RFC 3986 unreserved
     * characters, so a slash or a space in an id never changes the route.
     *
     * @param value the parameter
     * @return the encoded segment
     */
    public static String pathSegment(String value) {
        byte[] bytes = value.getBytes(StandardCharsets.UTF_8);
        StringBuilder out = new StringBuilder(bytes.length * 3);
        for (byte b : bytes) {
            int c = b & 0xff;
            if ((c >= 'a' && c <= 'z')
                    || (c >= 'A' && c <= 'Z')
                    || (c >= '0' && c <= '9')
                    || c == '-'
                    || c == '.'
                    || c == '_'
                    || c == '~') {
                out.append((char) c);
            } else {
                out.append('%').append((char) HEX[c >> 4]).append((char) HEX[c & 0xf]);
            }
        }
        return out.toString();
    }

    /**
     * A paged list's walk: {@code fetch} takes the token for the next page ({@code null} for the
     * first) and answers that page.
     *
     * @param fetch fetches one page
     * @param <T> the item type
     * @return the walk
     */
    public static <T> Pages<T> pages(Function<String, Pages.Page<T>> fetch) {
        return Pages.of(fetch);
    }
}
