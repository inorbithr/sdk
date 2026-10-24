package example;

import hr.inorbit.sdk.Client;
import hr.inorbit.sdk.auth.Token;
import hr.inorbit.sdk.auth.TokenProvider;
import hr.inorbit.sdk.generated.Public;

/**
 * Auth with a token provider of your own, here a stand-in for a secret store. The SDK asks it
 * before every attempt and calls {@code invalidate()} when the API refused the last token. It
 * must be safe to call from several threads. Never log the token.
 */
public final class AuthCustomToken {

    private AuthCustomToken() {}

    /**
     * Calls the API with the provider.
     *
     * @param args unused
     */
    public static void main(String[] args) {
        TokenProvider vault = () -> new Token("custom-token-1", null); // pass an expiry when you know it
        Client.Builder builder = Client.builder().tokenProvider(vault);
        String base = System.getenv("INORBIT_BASE_URL");
        if (base != null && !base.isEmpty()) {
            builder.baseUrl(base);
        }
        System.out.println(new Public(builder.build()).me().value().subject() + " signed in with a custom provider");
    }
}
