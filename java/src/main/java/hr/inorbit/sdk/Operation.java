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
    private final boolean idempotencyKey;
    private final String template;
    private final String rpc;
    private final Map<String, Object> fields;

    private Operation(Builder b) {
        this.name = b.name;
        this.method = b.method;
        this.path = b.path;
        this.query = List.copyOf(b.query);
        this.body = b.body;
        this.scopes = List.copyOf(b.scopes);
        this.idempotent = b.idempotent;
        this.idempotencyKey = b.idempotencyKey;
        this.template = b.template;
        this.rpc = b.rpc;
        this.fields = java.util.Collections.unmodifiableMap(new java.util.LinkedHashMap<>(b.fields));
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

    /**
     * Whether the operation takes {@code Idempotency-Key} (docs/config.md section 7.5): the call
     * then sends one key on every attempt and may be retried like a read.
     *
     * @return whether it takes the key
     */
    public boolean takesIdempotencyKey() {
        return idempotencyKey;
    }

    /**
     * The path template ({@code /v1/webhooks/endpoints/{endpoint_id}}), which names an attempt's
     * span without the identifiers; {@code null} when the generator gave none.
     *
     * @return the template, or {@code null}
     */
    public String template() {
        return template;
    }

    /**
     * The RPC's full name a {@code /v1/ws} call frame names ({@code
     * iohr.events.v1.EventsService/StreamEvents}), or {@code null} when the operation has none.
     *
     * @return the name, or {@code null}
     */
    public String rpc() {
        return rpc;
    }

    /**
     * The path and query parameters as typed values by wire name, the body of a {@code /v1/ws}
     * call frame; unset ones are absent.
     *
     * @return the fields, in the order given
     */
    public Map<String, Object> fields() {
        return fields;
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
        private boolean idempotencyKey;
        private String template;
        private String rpc;
        private final Map<String, Object> fields = new java.util.LinkedHashMap<>();

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
         * Marks an operation that takes {@code Idempotency-Key}: the call gets one key, sent on
         * every attempt, and is retried like a read.
         *
         * @param takesKey whether the operation takes the key
         * @return this builder
         */
        public Builder idempotencyKey(boolean takesKey) {
            this.idempotencyKey = takesKey;
            return this;
        }

        /**
         * The path template, for span names ({@code /v1/radar/digests/{digest_id}}).
         *
         * @param template the template
         * @return this builder
         */
        public Builder template(String template) {
            this.template = template;
            return this;
        }

        /**
         * The RPC's full name, for a stream opened over {@code /v1/ws}.
         *
         * @param rpc the name ({@code iohr.events.v1.EventsService/StreamEvents})
         * @return this builder
         */
        public Builder rpc(String rpc) {
            this.rpc = rpc;
            return this;
        }

        /**
         * One path or query parameter as its typed value, for the body of a {@code /v1/ws} call
         * frame; {@code null} leaves it out. The query and the path are given separately.
         *
         * @param name the wire name ({@code a.b} for a nested field)
         * @param value the value
         * @return this builder
         */
        public Builder field(String name, Object value) {
            if (value != null) {
                fields.put(name, value);
            }
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
