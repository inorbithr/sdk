package example;

import hr.inorbit.sdk.Client;
import hr.inorbit.sdk.Response;
import hr.inorbit.sdk.generated.Me;
import hr.inorbit.sdk.generated.Public;
import hr.inorbit.sdk.middleware.Middleware;
import java.time.Duration;

/**
 * A client built with {@code load}, a middleware of your own, and logging. {@code load} takes
 * each setting from code, then the {@code INORBIT_*} variables, then your profile in the {@code
 * iohr} config file, then the default (docs/config.md). Run {@code iohr login} once, or set
 * INORBIT_TOKEN.
 */
public final class Configured {

    private Configured() {}

    /**
     * Prints the effective configuration, then the caller.
     *
     * @param args unused
     */
    public static void main(String[] args) {
        // Per retry, the middleware sees the finished request; it times the rest of the pipeline.
        Middleware timing = Middleware.of("timing", (request, chain) -> {
            long started = System.nanoTime();
            var response = chain.proceed(request);
            System.out.printf(
                    "%s attempt %d: %d, %d ms%n",
                    request.info().operation(),
                    request.info().attempt(),
                    response.status(),
                    (System.nanoTime() - started) / 1_000_000);
            return response;
        });
        Client client = Client.builder()
                .timeout(Duration.ofSeconds(10))
                .log("info")
                .pipeline(p -> p.addPerRetry(timing))
                .load();
        System.out.println(client.config().toJson());
        Response<Me> me = new Public(client).me();
        System.out.println("subject " + me.value().subject() + ", request id " + me.raw().requestId()
                + ", rate limit " + me.raw().rateLimit().map(Object::toString).orElse("none"));
    }
}
