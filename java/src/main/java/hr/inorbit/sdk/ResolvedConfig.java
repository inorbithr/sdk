package hr.inorbit.sdk;

import com.fasterxml.jackson.core.JsonProcessingException;
import com.fasterxml.jackson.databind.node.ObjectNode;

/**
 * A client's effective configuration and where each value came from (docs/config.md section
 * 2.6): the profile, the config file, each setting with its source, the credential chain, the
 * pipeline and what was ignored. Secrets are always {@code <redacted>}. The same document {@code
 * iohr sdk config} prints.
 */
public final class ResolvedConfig {

    private final ObjectNode doc;

    ResolvedConfig(ObjectNode doc) {
        this.doc = doc;
    }

    /**
     * The effective configuration as a JSON object, stable within a major version.
     *
     * @return a copy of the document
     */
    public ObjectNode describe() {
        return doc.deepCopy();
    }

    /**
     * {@link #describe()} as indented JSON text.
     *
     * @return the text
     */
    public String toJson() {
        try {
            return Json.MAPPER.writerWithDefaultPrettyPrinter().writeValueAsString(doc);
        } catch (JsonProcessingException e) {
            return doc.toString();
        }
    }

    /** The document; it holds no secret. */
    @Override
    public String toString() {
        return doc.toString();
    }
}
