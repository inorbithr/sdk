package hr.inorbit.sdk;

/**
 * A typed answer and the raw one beside it.
 *
 * @param value the answer, typed
 * @param raw the answer as it came
 * @param <T> the answer's type
 */
public record Response<T>(T value, RawResponse raw) {}
