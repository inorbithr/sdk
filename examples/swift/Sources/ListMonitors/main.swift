// Every monitor of an account, page after page, then the latest runs of the first one.
// Reads INORBIT_TOKEN, or INORBIT_KEY_ID, INORBIT_KEY_SECRET and INORBIT_SCOPES
// (monitors:read), and the account id from INORBIT_ORG_ID.
import Foundation
import InOrbit

let api = try Client<Public>.fromEnv()
let org = ProcessInfo.processInfo.environment["INORBIT_ORG_ID"] ?? "<org_id>"
do {
    var first: String?
    for try await monitor in api.connections.allListMonitors(orgId: org) {
        print(monitor.monitorId, monitor.name)
        first = first ?? monitor.monitorId
    }
    if let first {
        let runs = try await api.connections.listMonitorRuns(orgId: org, monitorId: first).value
        print("latest runs of \(first): \(runs.runs.count)")
    }
} catch let e as APIError {
    print("\(e.code): \(e.problem) (request id \(e.raw.requestId))")
}
