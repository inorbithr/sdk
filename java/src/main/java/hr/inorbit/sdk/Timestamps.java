package hr.inorbit.sdk;

import java.time.OffsetDateTime;
import java.time.format.DateTimeParseException;
import java.util.Objects;
import java.util.Optional;
import java.util.regex.Pattern;

/**
 * Timestamps as the API sends them: RFC 3339 strings, {@code ""} when unset (rule N5 of {@code
 * spec/README.md}, settled behaviour). The generated models keep the string; this reads it.
 */
public final class Timestamps {

    private static final Pattern RFC3339 =
            Pattern.compile("\\d{4}-\\d{2}-\\d{2}[Tt]\\d{2}:\\d{2}:\\d{2}(\\.\\d+)?([Zz]|[+-]\\d{2}:\\d{2})");

    private Timestamps() {}

    /**
     * Reads a timestamp field.
     *
     * @param value the field as sent
     * @return the instant, or an empty {@code Optional} for {@code ""} (the field is unset)
     * @throws IllegalArgumentException when the value is neither {@code ""} nor RFC 3339
     */
    public static Optional<OffsetDateTime> parse(String value) {
        Objects.requireNonNull(value, "value");
        if (value.isEmpty()) {
            return Optional.empty();
        }
        if (!RFC3339.matcher(value).matches()) {
            throw new IllegalArgumentException("not an RFC 3339 timestamp");
        }
        try {
            return Optional.of(OffsetDateTime.parse(value.toUpperCase(java.util.Locale.ROOT)));
        } catch (DateTimeParseException e) {
            throw new IllegalArgumentException("not an RFC 3339 timestamp", e);
        }
    }
}
