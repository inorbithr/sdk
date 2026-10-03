package hr.inorbit.sdk;

/**
 * 64-bit integers on the wire. The API sends them as decimal strings, so no value above 2^53 loses
 * precision in a JSON parser; the generated models carry them as {@code long} and read and write
 * the decimal form at the boundary.
 */
public final class Int64 {

    private Int64() {}

    /**
     * Reads a 64-bit integer: a decimal string, or a number.
     *
     * @param value the wire value
     * @return the integer
     * @throws NumberFormatException when the value is neither
     */
    public static long parse(Object value) {
        if (value instanceof Number n && !(value instanceof Double) && !(value instanceof Float)) {
            return n.longValue();
        }
        if (value instanceof String s && s.matches("-?\\d+")) {
            return Long.parseLong(s);
        }
        throw new NumberFormatException("not a 64-bit integer: " + value);
    }

    /**
     * Writes a 64-bit integer as the API reads it.
     *
     * @param value the integer
     * @return its decimal string
     */
    public static String format(long value) {
        return Long.toString(value);
    }
}
