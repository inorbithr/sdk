package example;

import hr.inorbit.sdk.Client;
import hr.inorbit.sdk.Response;
import hr.inorbit.sdk.generated.Me;
import hr.inorbit.sdk.generated.Public;

/**
 * The SDK retries for you. The API answers 429 with Retry-After; the SDK waits that long and
 * tries again, and the answer says how many attempts it took. Tune it with {@code maxRetries}.
 */
public final class Retries {

    private Retries() {}

    /**
     * Calls the API and says how many attempts it took.
     *
     * @param args unused
     */
    public static void main(String[] args) {
        Response<Me> me = new Public(Client.builder().maxRetries(3).load()).me();
        System.out.println(me.value().subject() + " after " + me.raw().attempts() + " attempts");
    }
}
