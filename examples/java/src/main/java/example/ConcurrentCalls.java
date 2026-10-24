package example;

import hr.inorbit.sdk.Response;
import hr.inorbit.sdk.generated.Me;
import hr.inorbit.sdk.generated.Public;
import java.util.List;
import java.util.concurrent.CompletableFuture;

/**
 * Concurrent use: one client shared by several calls in flight, each a {@link
 * CompletableFuture}. Callers that need a token at the same moment share one exchange.
 */
public final class ConcurrentCalls {

    private ConcurrentCalls() {}

    /**
     * Makes three calls at once.
     *
     * @param args unused
     */
    public static void main(String[] args) {
        Public api = Public.fromEnv();
        List<CompletableFuture<Response<Me>>> calls = List.of(api.meAsync(), api.meAsync(), api.meAsync());
        List<String> subjects = calls.stream().map(f -> f.join().value().subject()).toList();
        System.out.println(subjects.size() + " calls at once: " + String.join(", ", subjects));
    }
}
