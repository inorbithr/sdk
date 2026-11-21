package @@MODULE@@;

import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * A small strict JSON reader: the dataset is JSON Lines and the JDK has no JSON reader. It reads
 * the whole of JSON and refuses anything after the value. Objects come back as Map, arrays as
 * List, strings as String, numbers as Double, true and false as Boolean and null as null.
 */
final class Json {
    private final String text;
    private int at;

    private Json(String text) {
        this.text = text;
    }

    static Object parse(String text) {
        Json p = new Json(text);
        Object value = p.value(0);
        p.space();
        if (p.at != text.length()) {
            throw p.fail("unexpected text");
        }
        return value;
    }

    private IllegalArgumentException fail(String what) {
        return new IllegalArgumentException(what + " at column " + (at + 1));
    }

    private void space() {
        while (at < text.length() && " \t\n\r".indexOf(text.charAt(at)) >= 0) {
            at++;
        }
    }

    private boolean word(String word) {
        if (text.startsWith(word, at)) {
            at += word.length();
            return true;
        }
        return false;
    }

    private Object value(int depth) {
        if (depth > 64) {
            throw fail("nested too deep");
        }
        space();
        if (at >= text.length()) {
            throw fail("unexpected end");
        }
        char c = text.charAt(at);
        switch (c) {
            case '{':
                return object(depth);
            case '[':
                return array(depth);
            case '"':
                return string();
            case 't':
                if (word("true")) return Boolean.TRUE;
                throw fail("unexpected text");
            case 'f':
                if (word("false")) return Boolean.FALSE;
                throw fail("unexpected text");
            case 'n':
                if (word("null")) return null;
                throw fail("unexpected text");
            default:
                if (c == '-' || (c >= '0' && c <= '9')) {
                    return number();
                }
                throw fail("unexpected character");
        }
    }

    private Object number() {
        int start = at;
        while (at < text.length() && "+-.eE0123456789".indexOf(text.charAt(at)) >= 0) {
            at++;
        }
        try {
            return Double.valueOf(text.substring(start, at));
        } catch (NumberFormatException e) {
            throw fail("bad number");
        }
    }

    private String string() {
        at++; // the opening quote
        StringBuilder out = new StringBuilder();
        while (true) {
            if (at >= text.length()) {
                throw fail("unterminated string");
            }
            char c = text.charAt(at++);
            if (c == '"') {
                return out.toString();
            }
            if (c < 0x20) {
                throw fail("control character in string");
            }
            if (c != '\\') {
                out.append(c);
                continue;
            }
            if (at >= text.length()) {
                throw fail("unterminated string");
            }
            char e = text.charAt(at++);
            switch (e) {
                case '"', '\\', '/' -> out.append(e);
                case 'b' -> out.append('\b');
                case 'f' -> out.append('\f');
                case 'n' -> out.append('\n');
                case 'r' -> out.append('\r');
                case 't' -> out.append('\t');
                case 'u' -> {
                    // A surrogate pair arrives as two escapes, each appended as one char.
                    if (at + 4 > text.length()) {
                        throw fail("bad \\u escape");
                    }
                    try {
                        out.append((char) Integer.parseInt(text.substring(at, at + 4), 16));
                    } catch (NumberFormatException ex) {
                        throw fail("bad \\u escape");
                    }
                    at += 4;
                }
                default -> throw fail("bad escape");
            }
        }
    }

    private List<Object> array(int depth) {
        at++;
        List<Object> items = new ArrayList<>();
        space();
        if (at < text.length() && text.charAt(at) == ']') {
            at++;
            return items;
        }
        while (true) {
            items.add(value(depth + 1));
            space();
            char c = at < text.length() ? text.charAt(at) : '\0';
            if (c == ',') {
                at++;
            } else if (c == ']') {
                at++;
                return items;
            } else {
                throw fail("expected , or ]");
            }
        }
    }

    private Map<String, Object> object(int depth) {
        at++;
        Map<String, Object> map = new LinkedHashMap<>();
        space();
        if (at < text.length() && text.charAt(at) == '}') {
            at++;
            return map;
        }
        while (true) {
            space();
            if (at >= text.length() || text.charAt(at) != '"') {
                throw fail("expected a key");
            }
            String key = string();
            space();
            if (at >= text.length() || text.charAt(at) != ':') {
                throw fail("expected :");
            }
            at++;
            map.put(key, value(depth + 1));
            space();
            char c = at < text.length() ? text.charAt(at) : '\0';
            if (c == ',') {
                at++;
            } else if (c == '}') {
                at++;
                return map;
            } else {
                throw fail("expected , or }");
            }
        }
    }

    /** A string as JSON text, escaped. */
    static String quote(String s) {
        StringBuilder out = new StringBuilder("\"");
        for (int i = 0; i < s.length(); i++) {
            char c = s.charAt(i);
            switch (c) {
                case '"' -> out.append("\\\"");
                case '\\' -> out.append("\\\\");
                case '\n' -> out.append("\\n");
                case '\r' -> out.append("\\r");
                case '\t' -> out.append("\\t");
                default -> {
                    if (c < 0x20) {
                        out.append(String.format("\\u%04x", (int) c));
                    } else {
                        out.append(c);
                    }
                }
            }
        }
        return out.append('"').toString();
    }
}
