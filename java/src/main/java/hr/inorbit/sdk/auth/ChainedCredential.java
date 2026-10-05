package hr.inorbit.sdk.auth;

import hr.inorbit.sdk.errors.AuthException;
import hr.inorbit.sdk.errors.ConfigException;
import hr.inorbit.sdk.errors.InOrbitException;
import java.util.ArrayList;
import java.util.List;

/**
 * Tries providers in order and keeps the first that gives a token (docs/config.md section 5.2).
 * When every one fails, the {@link AuthException} lists each and why.
 */
public final class ChainedCredential implements TokenProvider {

    private final List<TokenProvider> providers;
    private final Object lock = new Object();
    private volatile TokenProvider chosen;

    /**
     * A chain of {@code providers}, tried in this order.
     *
     * @param providers the providers
     */
    public ChainedCredential(TokenProvider... providers) {
        if (providers.length == 0) {
            throw new ConfigException("a chain needs at least one provider");
        }
        this.providers = List.of(providers);
    }

    @Override
    public Token token() {
        TokenProvider c = chosen;
        if (c != null) {
            return c.token();
        }
        synchronized (lock) {
            if (chosen != null) {
                return chosen.token();
            }
            List<String> tried = new ArrayList<>();
            for (TokenProvider p : providers) {
                try {
                    Token t = p.token();
                    chosen = p;
                    return t;
                } catch (InOrbitException e) {
                    tried.add(p + ": " + e.getMessage());
                }
            }
            throw new AuthException(
                    "no credential in the chain gave a token; tried:\n  " + String.join("\n  ", tried), "", null);
        }
    }

    @Override
    public void invalidate() {
        TokenProvider c = chosen;
        if (c != null) {
            c.invalidate();
        }
    }

    /** The providers, never a token. */
    @Override
    public String toString() {
        return "ChainedCredential" + providers;
    }
}
