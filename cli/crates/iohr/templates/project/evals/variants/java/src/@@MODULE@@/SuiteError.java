package @@MODULE@@;

/** The dataset or the bar is wrong. The run stops and exits 2: nothing was graded. */
public final class SuiteError extends Exception {
    private static final long serialVersionUID = 1L;

    public SuiteError(String message) {
        super(message);
    }
}
