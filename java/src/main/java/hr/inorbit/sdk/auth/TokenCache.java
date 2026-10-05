package hr.inorbit.sdk.auth;

import hr.inorbit.sdk.errors.InOrbitException;
import java.time.Duration;
import java.time.Instant;
import java.util.function.BiConsumer;
import java.util.function.Supplier;

/**
 * The caching rules of docs/config.md section 5.3 for one provider: in memory only, refresh
 * when less than a fifth of the lifetime is left, one refresh at a time (callers wait for the same
 * one), and soft expiry: when a refresh fails and the cached token is still valid it is used, and
 * the next refresh is tried no sooner than 5 s later.
 */
final class TokenCache {

    private static final Duration SOFT_RETRY = Duration.ofSeconds(5);

    private final Object lock = new Object();
    private final Supplier<Token> fetch;
    private volatile BiConsumer<String, String> listener;
    private Token token;
    private Instant issued;
    private Instant notBefore = Instant.MIN;

    TokenCache(Supplier<Token> fetch) {
        this.fetch = fetch;
    }

    void listen(BiConsumer<String, String> listener) {
        this.listener = listener;
    }

    Token get() {
        synchronized (lock) {
            Instant now = Instant.now();
            Token t = usable(now);
            if (t != null) {
                return t;
            }
            Token fresh;
            try {
                fresh = fetch.get();
            } catch (InOrbitException e) {
                tell("token_exchange", e.getClass().getSimpleName());
                Token held = token;
                if (held == null || held.expiresAt().map(x -> !now.isBefore(x)).orElse(false)) {
                    throw e;
                }
                notBefore = now.plus(SOFT_RETRY);
                tell("token_refresh_failed", e.kind());
                return held;
            }
            token = fresh;
            issued = now;
            notBefore = Instant.MIN;
            tell("token_exchange", null);
            return fresh;
        }
    }

    private Token usable(Instant now) {
        Token t = token;
        if (t == null) {
            return null;
        }
        if (t.expiresAt().isEmpty()) {
            return t;
        }
        Instant expires = t.expiresAt().get();
        Duration lifetime = Duration.between(issued, expires);
        if (Duration.between(issued, now).compareTo(lifetime.multipliedBy(4).dividedBy(5)) < 0) {
            return t;
        }
        if (now.isBefore(notBefore) && now.isBefore(expires)) {
            return t;
        }
        return null;
    }

    void invalidate() {
        synchronized (lock) {
            token = null;
            notBefore = Instant.MIN;
        }
    }

    private void tell(String event, String value) {
        BiConsumer<String, String> l = listener;
        if (l != null) {
            l.accept(event, value);
        }
    }
}
