/** One test, with no framework to install: it exits 1 when it fails. */
public final class MainTest {
    public static void main(String[] args) {
        String got = Main.hello();
        if (!got.equals("hello from @@NAME@@")) {
            System.err.println("hello() = " + got);
            System.exit(1);
        }
        System.out.println("ok: hello");
    }
}
