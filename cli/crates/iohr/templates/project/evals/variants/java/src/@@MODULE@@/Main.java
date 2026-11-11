package @@MODULE@@;

/** @@NAME@@: an offline eval suite with a pass bar. JDK only, no network. */
public final class Main {
    private Main() {}

    public static void main(String[] args) {
        System.exit(Suite.run(args, System.out, System.err, null));
    }
}
