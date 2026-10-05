package hr.inorbit.sdk;

/**
 * The per-client retry quota (docs/config.md section 7.4): a token bucket after AWS's standard
 * retry mode. A retry draws its cost; a call that succeeds after retries returns the cost of its
 * last retry, and one that succeeds on the first attempt adds 1, up to the capacity.
 */
final class RetryBudget {

    /** The bucket's capacity. */
    static final int CAPACITY = 500;

    /** What a retry after a 429, or a 503 with {@code Retry-After}, costs. */
    static final int COST_THROTTLED = 5;

    /** What any other retry costs. */
    static final int COST_OTHER = 10;

    private final int capacity;
    private final boolean enabled;
    private int level;

    RetryBudget(int capacity, boolean enabled) {
        this.capacity = capacity;
        this.enabled = enabled;
        this.level = capacity;
    }

    /** Takes {@code cost} for a retry; false when the bucket cannot pay. */
    synchronized boolean draw(int cost) {
        if (!enabled) {
            return true;
        }
        if (level < cost) {
            return false;
        }
        level -= cost;
        return true;
    }

    /** Puts {@code amount} back, up to the capacity. */
    synchronized void refund(int amount) {
        if (enabled) {
            level = Math.min(capacity, level + amount);
        }
    }

    synchronized int level() {
        return level;
    }
}
