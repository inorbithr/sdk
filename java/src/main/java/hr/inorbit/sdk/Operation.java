package hr.inorbit.sdk;

import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import java.util.Objects;

/**
 * One call, as a generated surface builds it: the method, the path with its parameters bound and
 * encoded, the query, the JSON body, and what the call needs. Build one with {@link #builder}.
 */
public final class Operation {

    private final String name;
    private final Method method;
    private final String path;
    private final List<Map.Entry<String, String>> query;
    private final Object body;
    private final List<String> scopes;
    private final boolean idempotent;

    private Operation(Builder b) {
        this.name = b.name;
        this.method = b.method;
        this.path = b.path;
        this.query = List.copyOf(b.query);
        this.body = b.body;
        this.scopes = List.copyOf(b.scopes);
        this.idempotent = b.idempotent;
    }

    /**
     * A builder for a call to {@code path}.
     *
     * @param method the method
     * @param path the path, starting with one {@code /}, parameters bound and encoded
     * @return the builder
     */
    public static Builder builder(Method method, String path) {
        return new Builder(Objects.requireNonNull(method, "method"), Objects.requireNonNull(path, "path"));
    }

    /**
     * What hooks see ({@code radar.list_digests}), or the path for a raw call.
     *
     * @return the name
     */
    public String name() {
        return name == null ? path : name;
    }

    /**
     * The method.
     *
     * @return the method
     */
    public Method method() {
        return method;
    }

    /**
     * The path.
     *
     * @return the path
     */
    public String path() {
        return path;
    }

    /**
     * The query parameters, in order; a list value repeats its name.
     *
     * @return the parameters
     */
    public List<Map.Entry<String, String>> query() {
        return query;
    }

    /**
     * The JSON body.
     *
     * @return the body, or {@code null}
     */
    public Object body() {
        return body;
    }

    /**
     * The scopes the operation needs, for the record.
     *
     * @return the scopes
     */
    public List<String> scopes() {
        return scopes;
    }

    /**
     * Whether a failed attempt may be repeated: the method is idempotent, or the operation says so.
     *
     * @return whether the call is retried
     */
    public boolean retrySafe() {
        return idempotent || method.isIdempotent();
    }

    @Override
    public String toString() {
        return "Operation[" + method + " " + path + "]";
    }

    /** Builds an {@link Operation}. */
    public static final class Builder {

        private String name;
        private final Method method;
        private final String path;
        private final List<Map.Entry<String, String>> query = new ArrayList<>();
        private Object body;
        private final List<String> scopes = new ArrayList<>();
        private boolean idempotent;

        private Builder(Method method, String path) {
            this.method = method;
            this.path = path;
        }

        /**
         * Names the operation for hooks.
         *
         * @param name such as {@code radar.list_digests}
         * @return this builder
         */
        public Builder name(String name) {
            this.name = name;
            return this;
        }

        /**
         * Adds a query parameter; {@code null} is left out, a {@link Iterable} repeats the name.
         *
         * @param name the parameter
         * @param value its value
         * @return this builder
         */
        public Builder query(String name, Object value) {
            if (value instanceof Iterable<?> values) {
                for (Object v : values) {
                    query(name, v);
                }
            } else if (value != null) {
                query.add(Map.entry(name, String.valueOf(value)));
            }
            return this;
        }

        /**
         * Sets the JSON body.
         *
         * @param body a value Jackson writes
         * @return this builder
         */
        public Builder body(Object body) {
            this.body = body;
            return this;
        }

        /**
         * Records the scopes the operation needs.
         *
         * @param scopes the scopes
         * @return this builder
         */
        public Builder scopes(String... scopes) {
            this.scopes.addAll(List.of(scopes));
            return this;
        }

        /**
         * Retries the call like an idempotent method although its method is not.
         *
         * @param idempotent whether the operation is idempotent
         * @return this builder
         */
        public Builder idempotent(boolean idempotent) {
            this.idempotent = idempotent;
            return this;
        }

        /**
         * The operation.
         *
         * @return the operation
         */
        public Operation build() {
            return new Operation(this);
        }
    }
}
