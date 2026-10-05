package hr.inorbit.sdk.middleware;

import java.util.Collections;
import java.util.LinkedHashMap;
import java.util.Locale;
import java.util.Map;
import java.util.Objects;
import java.util.Optional;

/** A request's headers: one value per name, names compared without case, changed in place. */
public final class Headers {

    private final Map<String, String> values = new LinkedHashMap<>();

    /** No headers. */
    public Headers() {}

    /**
     * A copy of {@code other}.
     *
     * @param other the headers to copy
     */
    public Headers(Headers other) {
        values.putAll(other.values);
    }

    /**
     * Sets {@code name} to {@code value}, replacing what was there.
     *
     * @param name the header, any case
     * @param value its value
     * @return these headers
     */
    public Headers set(String name, String value) {
        values.put(name.toLowerCase(Locale.ROOT), Objects.requireNonNull(value, "value"));
        return this;
    }

    /**
     * The value of {@code name}.
     *
     * @param name the header, any case
     * @return the value, if set
     */
    public Optional<String> get(String name) {
        return Optional.ofNullable(values.get(name.toLowerCase(Locale.ROOT)));
    }

    /**
     * Whether {@code name} is set.
     *
     * @param name the header, any case
     * @return whether it is set
     */
    public boolean contains(String name) {
        return values.containsKey(name.toLowerCase(Locale.ROOT));
    }

    /**
     * Removes {@code name}.
     *
     * @param name the header, any case
     * @return these headers
     */
    public Headers remove(String name) {
        values.remove(name.toLowerCase(Locale.ROOT));
        return this;
    }

    /**
     * Every header, names in lower case, in the order set.
     *
     * @return a read-only view
     */
    public Map<String, String> asMap() {
        return Collections.unmodifiableMap(values);
    }

    /** The names only: values can hold credentials. */
    @Override
    public String toString() {
        return "Headers" + values.keySet();
    }
}
