package example;

import hr.inorbit.sdk.Client;
import hr.inorbit.sdk.generated.Public;

/**
 * Auth with a token you already have: an API token from the console or {@code iohr token
 * create}. The SDK sends it as a Bearer on every call and never exchanges it.
 */
public final class AuthToken {

    private AuthToken() {}

    /**
     * Calls the API with the token.
     *
     * @param args unused
     */
    public static void main(String[] args) {
        Client.Builder builder = Client.builder().token(System.getenv().getOrDefault("INORBIT_TOKEN", ""));
        String base = System.getenv("INORBIT_BASE_URL");
        if (base != null && !base.isEmpty()) {
            builder.baseUrl(base);
        }
        System.out.println(new Public(builder.build()).me().value().subject() + " signed in with a token");
    }
}
