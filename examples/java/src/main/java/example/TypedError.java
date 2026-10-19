package example;

import hr.inorbit.sdk.errors.ApiException;
import hr.inorbit.sdk.errors.Code;
import hr.inorbit.sdk.generated.Public;

/**
 * Typed error handling. A refused call throws {@link ApiException} with the HTTP status, a
 * stable code to branch on, the problem text and the request id to quote to support.
 */
public final class TypedError {

    private TypedError() {}

    /**
     * Calls the API and reports a refusal.
     *
     * @param args unused
     */
    public static void main(String[] args) {
        try {
            Public.fromEnv().me();
        } catch (ApiException e) {
            if (!e.code().equals(Code.FORBIDDEN)) {
                throw e;
            }
            System.out.println("refused: status " + e.status() + ", code " + e.code().slug()
                    + ", request " + e.raw().requestId());
        }
    }
}
