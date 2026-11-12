package app;

/** The entry point: reads HOST and PORT, starts the service and stops it cleanly on SIGINT or SIGTERM. */
public final class Main {
    private Main() {}

    public static void main(String[] args) {
        Obs.Logger log = new Obs.Logger(System.out);
        Service.Config cfg;
        try {
            cfg = Service.Config.load(System::getenv);
        } catch (IllegalArgumentException e) {
            log.error("bad configuration", "error", e.getMessage());
            System.exit(2);
            return;
        }
        Service svc = new Service(log);
        // The JVM runs shutdown hooks on SIGINT and SIGTERM: the hook is the clean shutdown.
        Runtime.getRuntime().addShutdownHook(new Thread(svc::stop, "shutdown"));
        try {
            svc.start(cfg);
        } catch (java.io.IOException e) {
            log.error("cannot listen", "error", e.getClass().getSimpleName());
            System.exit(1);
        }
    }
}
