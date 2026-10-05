package hr.inorbit.sdk.middleware;

import hr.inorbit.sdk.errors.ConfigException;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import java.util.Objects;
import java.util.Set;

/**
 * A client's ordered list of middlewares, edited by name when the client is built
 * (docs/config.md section 7.3). Every method returns the pipeline, so edits chain: {@code p ->
 * p.addPerRetry(probe).remove("rate_limit")}. A name that exists, or one that does not, is a
 * {@link ConfigException}; {@code retry}, {@code auth} and {@code timeout} can be replaced but not
 * removed.
 */
public final class Pipeline {

    private static final Set<String> PROTECTED = Set.of("retry", "auth", "timeout");

    private final List<Map.Entry<String, Middleware>> entries;

    /**
     * A pipeline of {@code middlewares}, outermost first. The client builds its own; this is for
     * tests of a middleware.
     *
     * @param middlewares the middlewares
     */
    public Pipeline(List<Middleware> middlewares) {
        this.entries = new ArrayList<>();
        for (Middleware m : middlewares) {
            entries.add(Map.entry(fresh(m), m));
        }
    }

    /**
     * Every middleware's name, outermost first.
     *
     * @return the names
     */
    public List<String> names() {
        return entries.stream().map(Map.Entry::getKey).toList();
    }

    /**
     * The middlewares, outermost first.
     *
     * @return the middlewares
     */
    public List<Middleware> middlewares() {
        return entries.stream().map(Map.Entry::getValue).toList();
    }

    private int index(String name) {
        for (int i = 0; i < entries.size(); i++) {
            if (entries.get(i).getKey().equals(name)) {
                return i;
            }
        }
        throw new ConfigException("the pipeline has no middleware named \"" + name + "\"");
    }

    private String fresh(Middleware m) {
        Objects.requireNonNull(m, "middleware");
        String name = m.name();
        if (name == null
                || name.isEmpty()
                || entries.stream().anyMatch(e -> e.getKey().equals(name))) {
            throw new ConfigException("the pipeline already has a middleware named \"" + name + "\"");
        }
        return name;
    }

    /**
     * Adds {@code m} just before {@code retry}, after earlier additions: it runs once per call.
     *
     * @param m the middleware
     * @return this pipeline
     */
    public Pipeline addPerCall(Middleware m) {
        String name = fresh(m);
        entries.add(index("retry"), Map.entry(name, m));
        return this;
    }

    /**
     * Adds {@code m} just before {@code timeout}, after earlier additions: it runs once per attempt.
     *
     * @param m the middleware
     * @return this pipeline
     */
    public Pipeline addPerRetry(Middleware m) {
        String name = fresh(m);
        entries.add(index("timeout"), Map.entry(name, m));
        return this;
    }

    /**
     * Adds {@code m} just before the middleware named {@code name}.
     *
     * @param name a middleware in the pipeline
     * @param m the middleware to add
     * @return this pipeline
     */
    public Pipeline insertBefore(String name, Middleware m) {
        String fresh = fresh(m);
        entries.add(index(name), Map.entry(fresh, m));
        return this;
    }

    /**
     * Adds {@code m} just after the middleware named {@code name}.
     *
     * @param name a middleware in the pipeline
     * @param m the middleware to add
     * @return this pipeline
     */
    public Pipeline insertAfter(String name, Middleware m) {
        String fresh = fresh(m);
        entries.add(index(name) + 1, Map.entry(fresh, m));
        return this;
    }

    /**
     * Puts {@code m} in the place of {@code name}; it keeps that name.
     *
     * @param name a middleware in the pipeline
     * @param m the middleware to put there
     * @return this pipeline
     */
    public Pipeline replace(String name, Middleware m) {
        entries.set(index(name), Map.entry(name, Objects.requireNonNull(m, "middleware")));
        return this;
    }

    /**
     * Drops the middleware named {@code name}; {@code retry}, {@code auth} and {@code timeout}
     * stay (switch them off with a setting, such as {@code maxRetries(0)}).
     *
     * @param name a middleware in the pipeline
     * @return this pipeline
     */
    public Pipeline remove(String name) {
        if (PROTECTED.contains(name)) {
            throw new ConfigException(name + " cannot be removed; replace it, or switch it off with a setting "
                    + "(max_retries = 0 for retry)");
        }
        entries.remove(index(name));
        return this;
    }

    @Override
    public String toString() {
        return "Pipeline" + names();
    }
}
