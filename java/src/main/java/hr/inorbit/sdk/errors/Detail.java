package hr.inorbit.sdk.errors;

import com.fasterxml.jackson.annotation.JsonValue;
import com.fasterxml.jackson.core.JsonParser;
import com.fasterxml.jackson.databind.DeserializationContext;
import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.annotation.JsonDeserialize;
import com.fasterxml.jackson.databind.deser.std.StdDeserializer;
import java.io.IOException;
import java.util.LinkedHashMap;
import java.util.Map;

/**
 * One entry of an error's {@code details}. A type this version does not know is kept as it came, in
 * {@link Unknown}.
 */
@JsonDeserialize(using = Detail.Reader.class)
public sealed interface Detail permits Detail.Field, Detail.Info, Detail.Retry, Detail.Unknown {

    /**
     * A field of the request was wrong.
     *
     * @param field the field
     * @param description what is wrong with it
     */
    record Field(String field, String description) implements Detail {}

    /**
     * Why the request failed, as a machine-readable reason.
     *
     * @param reason the reason
     * @param domain the domain that gave it
     * @param metadata more about it
     */
    record Info(String reason, String domain, Map<String, String> metadata) implements Detail {
        /**
         * An info detail; the metadata is copied.
         *
         * @param reason the reason
         * @param domain the domain that gave it
         * @param metadata more about it
         */
        public Info {
            metadata = Map.copyOf(metadata);
        }
    }

    /**
     * When to try again.
     *
     * @param afterSeconds seconds to wait
     */
    record Retry(long afterSeconds) implements Detail {}

    /**
     * A detail type this version does not know, as it came.
     *
     * @param value the detail
     */
    record Unknown(@JsonValue JsonNode value) implements Detail {}

    /**
     * Reads a detail from its JSON.
     *
     * @param node the detail
     * @return the typed detail
     */
    static Detail of(JsonNode node) {
        String type = node.path("type").asText("");
        return switch (type) {
            case "field" ->
                new Field(
                        node.path("field").asText(""), node.path("description").asText(""));
            case "info" -> {
                Map<String, String> metadata = new LinkedHashMap<>();
                node.path("metadata")
                        .properties()
                        .forEach(e -> metadata.put(e.getKey(), e.getValue().asText()));
                yield new Info(
                        node.path("reason").asText(""), node.path("domain").asText(""), metadata);
            }
            case "retry" -> new Retry(node.path("after_seconds").asLong(0));
            default -> new Unknown(node);
        };
    }

    /** Jackson's way in: every detail goes through {@link Detail#of}. */
    final class Reader extends StdDeserializer<Detail> {

        private static final long serialVersionUID = 1L;

        /** The reader Jackson instantiates. */
        public Reader() {
            super(Detail.class);
        }

        @Override
        public Detail deserialize(JsonParser p, DeserializationContext ctxt) throws IOException {
            return of(p.readValueAsTree());
        }
    }
}
