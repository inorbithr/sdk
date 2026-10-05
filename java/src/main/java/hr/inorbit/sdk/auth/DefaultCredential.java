package hr.inorbit.sdk.auth;

import hr.inorbit.sdk.Client;
import hr.inorbit.sdk.LoadOptions;
import java.util.Objects;

/**
 * The credential chain of docs/config.md section 5.1 as one provider: code, the environment, the
 * reserved workload slot, the config file, then the {@code iohr} login, decided once. {@link
 * Client#load()} uses the same chain; this type is for a program that wants the credential alone.
 */
public final class DefaultCredential implements TokenProvider {

    private final TokenProvider chosen;

    private DefaultCredential(TokenProvider chosen) {
        this.chosen = Objects.requireNonNull(chosen, "chosen");
    }

    /**
     * The credential the chain finds in the process's environment and config file.
     *
     * @return the credential
     * @throws hr.inorbit.sdk.errors.ConfigException listing every source tried when none has one
     */
    public static DefaultCredential load() {
        return new DefaultCredential(Client.load().credential());
    }

    /**
     * The credential the chain finds in {@code options}.
     *
     * @param options the environment, OS and directories to read
     * @return the credential
     * @throws hr.inorbit.sdk.errors.ConfigException listing every source tried when none has one
     */
    public static DefaultCredential load(LoadOptions options) {
        return new DefaultCredential(Client.builder().load(options).credential());
    }

    @Override
    public Token token() {
        return chosen.token();
    }

    @Override
    public void invalidate() {
        chosen.invalidate();
    }

    /** The chosen provider, never a token. */
    @Override
    public String toString() {
        return "DefaultCredential(" + chosen + ")";
    }
}
