// A paged list. allListMonitors yields every item and follows nextPageToken for you; the
// second half reads one page by hand with pageSize. Reads INORBIT_ORG_ID and a credential
// with connections:read.
import Foundation
import InOrbit

func listPagination() async throws {
    let api = try Client<Public>.fromEnv()
    let org = ProcessInfo.processInfo.environment["INORBIT_ORG_ID"] ?? ""

    var n = 0
    for try await monitor in api.connections.allListMonitors(orgId: org, ConnectionsListMonitorsParams(pageSize: 2)) {
        n += 1
        print(monitor.monitorId, monitor.name)
    }
    print("\(n) monitors")

    let page = try await api.connections.listMonitors(orgId: org, ConnectionsListMonitorsParams(pageSize: 1)).value
    print("one page: \(page.monitors.count) monitor, more after \"\(page.nextPageToken)\"")
}
