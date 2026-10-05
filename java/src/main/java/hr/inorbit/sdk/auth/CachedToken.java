package hr.inorbit.sdk.auth;

import java.util.Objects;
import java.util.function.BiConsumer;

/**
 * Gives any {@link TokenProvider} the caching rules of docs/config.md section 5.3: cached in
 * memory, refreshed when less than a fifth of its lifetime is left, one refresh at a time, and a
 * still-valid token kept when a refresh fails. A token without an expiry is kept until the API
 * refuses it. Wrap a provider that fetches from a vault or a secrets manager in it.
 *
 * <pre>{@code
 * TokenProvider vault = () -> new Token(fetchFromVault(), Instant.now().plusSeconds(900));
 * Client client = Client.builder().tokenProvider(new CachedToken(vault)).load();
 * }</pre>
 */
public final class CachedToken implements TokenProvider, Observable {

    private final TokenProvider provider;
    private final TokenCache cache;

    /**
     * Caches {@code provider}'s tokens.
     *
     * @param provider the provider to ask when a token is needed
     */
    public CachedToken(TokenProvider provider) {
        this.provider = Objects.requireNonNull(provider, "provider");
        this.cache = new TokenCache(provider::token);
    }

    @Override
    public Token token() {
        return cache.get();
    }

    /** Drops the cached token and tells the provider. */
    @Override
    public void invalidate() {
        cache.invalidate();
        provider.invalidate();
    }

    @Override
    public void listen(BiConsumer<String, String> listener) {
        cache.listen(listener);
    }

    /** The wrapped provider's own text. */
    @Override
    public String toString() {
        return "CachedToken(" + provider + ")";
    }
}
