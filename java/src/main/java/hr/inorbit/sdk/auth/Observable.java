package hr.inorbit.sdk.auth;

import java.util.function.BiConsumer;

/**
 * A provider whose token fetches can be watched, for the client's logging and metrics: the
 * events {@code token_exchange} (the value is the error's type, or {@code null} for success) and
 * {@code token_refresh_failed} (the value is the error's kind). Never a token.
 */
public interface Observable {

    /**
     * Sets the one listener; {@code null} removes it.
     *
     * @param listener receives each event and its value
     */
    void listen(BiConsumer<String, String> listener);
}
