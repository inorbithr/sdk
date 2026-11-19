public final class Main {
    private Main() {}

    static String hello() {
        return "hello from @@NAME@@";
    }

    public static void main(String[] args) {
        System.out.println(hello());
    }
}
