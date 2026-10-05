package hr.inorbit.sdk.middleware;

import hr.inorbit.sdk.errors.InOrbitException;

/**
 * One named step a request passes through (docs/config.md section 7.13), OkHttp's interceptor
 * shape: change the request, answer without calling {@link Chain#proceed} (short-circuit), call
 * it once, or more than once (a retry of your own), and inspect what comes back.
 *
 * <pre>{@code
 * Middleware probe = Middleware.of("probe", (request, chain) -> {
 *     request.headers().set("x-team", "payments");
 *     return chain.proceed(request);
 * });
 * Client client = Client.builder().pipeline(p -> p.addPerRetry(probe)).load();
 * }</pre>
 *
 * <p>A middleware must not log secrets or bodies; the built-ins hold to section 7.9, a user's
 * middleware is the user's responsibility. One that runs per retry sees the finished request,
 * the {@code Authorization} header included.
 */
public interface Middleware {

    /**
     * The step's unique name in the pipeline.
     *
     * @return the name
     */
    String name();

    /**
     * Handles {@code request}, usually by calling {@code chain.proceed}.
     *
     * @param request the request; its headers may be changed in place
     * @param chain the rest of the pipeline
     * @return the answer
     * @throws InOrbitException to end the call with an error of the SDK's family
     */
    Response handle(Request request, Chain chain);

    /**
     * A middleware from a name and a function.
     *
     * @param name the unique name
     * @param handler what it does
     * @return the middleware
     */
    static Middleware of(String name, Handler handler) {
        java.util.Objects.requireNonNull(name, "name");
        java.util.Objects.requireNonNull(handler, "handler");
        return new Middleware() {
            @Override
            public String name() {
                return name;
            }

            @Override
            public Response handle(Request request, Chain chain) {
                return handler.handle(request, chain);
            }

            @Override
            public String toString() {
                return "Middleware[" + name + "]";
            }
        };
    }

    /** What {@link #of} wraps: the body of a middleware. */
    @FunctionalInterface
    interface Handler {
        /**
         * Handles {@code request}.
         *
         * @param request the request
         * @param chain the rest of the pipeline
         * @return the answer
         */
        Response handle(Request request, Chain chain);
    }
}
