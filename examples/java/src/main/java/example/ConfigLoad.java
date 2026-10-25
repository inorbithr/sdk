package example;

import hr.inorbit.sdk.Client;
import hr.inorbit.sdk.errors.ConfigException;
import hr.inorbit.sdk.generated.Me;
import hr.inorbit.sdk.generated.Public;
import java.time.Duration;

/**
 * Configuration by {@code load}: code, the environment, the iohr config file and the iohr
 * login. {@code config().describe()} says where each value came from, without printing a secret.
 */
public final class ConfigLoad {

    private ConfigLoad() {}

    /**
     * Prints where the credential came from, then the caller.
     *
     * @param args unused
     */
    public static void main(String[] args) {
        Client client;
        try {
            client = Client.builder().timeout(Duration.ofSeconds(10)).load();
        } catch (ConfigException e) {
            System.err.println(e.getMessage()); // every problem, with its setting
            System.exit(1);
            return;
        }
        System.out.println("credential from " + client.config().describe().path("credential").path("source").asText());
        Me me = new Public(client).me().value();
        System.out.println(me.subject() + ", a " + me.kind());
    }
}
