package example;

import hr.inorbit.sdk.errors.ApiException;
import hr.inorbit.sdk.errors.Code;
import hr.inorbit.sdk.generated.Digest;
import hr.inorbit.sdk.generated.ListDigestsResponse;
import hr.inorbit.sdk.generated.Me;
import hr.inorbit.sdk.generated.Public;
import hr.inorbit.sdk.generated.RadarListDigestsParams;

/**
 * Who the API thinks you are, and the latest radar digests, with the public surface. Run with
 * INORBIT_TOKEN set (an API token from the console or {@code iohr token create}).
 */
public final class Whoami {

    private Whoami() {}

    /**
     * Prints the caller and three digests.
     *
     * @param args unused
     */
    public static void main(String[] args) {
        Public api = Public.fromEnv();
        try {
            Me me = api.me().value();
            System.out.println("subject " + me.subject() + ", scopes " + String.join(" ", me.scopes()));
            ListDigestsResponse page = api.radar()
                    .listDigests(RadarListDigestsParams.builder().limit(3).build())
                    .value();
            for (Digest d : page.digests()) {
                System.out.println(d.week() + " " + d.language() + ": " + d.summary());
            }
        } catch (ApiException e) {
            if (!e.code().equals(Code.FORBIDDEN)) {
                throw e;
            }
            System.err.println("the token lacks a scope this example needs (identity:read, radar:read)");
        }
    }
}
