package app;

/** Passes the real streams and exits with the result of {@link Cli#run}. */
public final class Main {
    private Main() {}

    public static void main(String[] args) {
        System.exit(Cli.run(args, System.out, System.err, Cli.buildCommit()));
    }
}
