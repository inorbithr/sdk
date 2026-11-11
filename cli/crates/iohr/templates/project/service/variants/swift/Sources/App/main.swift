// Reads HOST and PORT from the environment, serves, and stops cleanly on SIGINT or SIGTERM.
import Dispatch
import Foundation
import Service

let log = Logger { line in
    FileHandle.standardOutput.write(Data((line + "\n").utf8))
}

let cfg: Config
do {
    cfg = try Config.load(ProcessInfo.processInfo.environment)
} catch {
    log.error("bad configuration", [("error", .string("\(error)"))])
    exit(2)
}

let svc = Service(log: log)
let port: UInt16
do {
    port = try svc.start(cfg)
} catch {
    log.error("cannot listen")
    exit(1)
}
log.info("listening", [("addr", .string("\(cfg.host):\(port)"))])

// Signals are handled on a global queue: ready goes false, the listener closes, and the
// process exits; a hard stop follows after 10 seconds.
nonisolated func watch(_ sig: Int32, _ service: Service, _ logger: Logger) -> DispatchSourceSignal {
    signal(sig, SIG_IGN)
    let source = DispatchSource.makeSignalSource(signal: sig, queue: .global())
    source.setEventHandler {
        logger.info("shutting down")
        DispatchQueue.global().asyncAfter(deadline: .now() + 10) { exit(1) }
        service.stop()
        exit(0)
    }
    source.resume()
    return source
}

let sources = [watch(SIGINT, svc, log), watch(SIGTERM, svc, log)]
withExtendedLifetime(sources) { dispatchMain() }
