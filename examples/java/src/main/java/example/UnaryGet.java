package example;

import hr.inorbit.sdk.Response;
import hr.inorbit.sdk.generated.Me;
import hr.inorbit.sdk.generated.Public;

/**
 * One unary GET: who the API thinks you are. The answer carries the typed value and the raw
 * response (request id, attempts, rate limit). Run with INORBIT_TOKEN set (or INORBIT_KEY_ID,
 * INORBIT_KEY_SECRET and INORBIT_SCOPES).
 */
public final class UnaryGet {

    private UnaryGet() {}

    /**
     * Prints the caller.
     *
     * @param args unused
     */
    public static void main(String[] args) {
        Response<Me> me = Public.fromEnv().me();
        System.out.println("subject " + me.value().subject() + ", scopes "
                + String.join(" ", me.value().scopes()) + ", request " + me.raw().requestId());
    }
}
