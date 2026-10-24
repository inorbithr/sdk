package example;

import hr.inorbit.sdk.generated.ConnectionsListMonitorsParams;
import hr.inorbit.sdk.generated.ListMonitorsResponse;
import hr.inorbit.sdk.generated.Monitor;
import hr.inorbit.sdk.generated.Public;

/**
 * A paged list. {@code allListMonitors} yields every item and follows {@code next_page_token}
 * for you; the second half reads one page by hand with {@code pageSize}. Run with INORBIT_ORG_ID
 * and a credential with {@code connections:read}.
 */
public final class ListPagination {

    private ListPagination() {}

    /**
     * Prints every monitor, then one page.
     *
     * @param args unused
     */
    public static void main(String[] args) {
        Public api = Public.fromEnv();
        String org = System.getenv().getOrDefault("INORBIT_ORG_ID", "");

        int n = 0;
        for (Monitor m : api.connections().allListMonitors(
                org, ConnectionsListMonitorsParams.builder().pageSize(2).build())) {
            n++;
            System.out.println(m.monitorId() + " " + m.name());
        }
        System.out.println(n + " monitors");

        ListMonitorsResponse page = api.connections()
                .listMonitors(org, ConnectionsListMonitorsParams.builder().pageSize(1).build())
                .value();
        System.out.println("one page: " + page.monitors().size() + " monitor, more after \""
                + page.nextPageToken() + "\"");
    }
}
