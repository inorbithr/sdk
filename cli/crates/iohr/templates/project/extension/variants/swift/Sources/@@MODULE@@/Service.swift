// @@ID@@: the demo project's private macOS service.
//
// Every minute it writes $STATE/heartbeat.json (the health InOrbit reads) and asks iohr's
// token channel for a token with the one scope its manifest declares. It also serves a tiny
// panel on 127.0.0.1 that InOrbit Desktop can show as a pane: the page reads /api/heartbeat.
//
// Run it with `swift run` under `iohr ext dev`'s STATE and CACHE, or once with IOHR_EXT_ONCE=1
// as the self-test does. Foundation only.

import Foundation

/// The extension id, from the manifest.
public let serviceId = "@@ID@@"

/// The program's version.
public let serviceVersion = "0.1.0"

/// The one scope the manifest declares.
public let serviceScopes = ["identity:read"]

private func say(_ text: String) {
    print(text)
    fflush(stdout)
}

private func complain(_ text: String) {
    FileHandle.standardError.write(Data((text + "\n").utf8))
}

/// Writes `value` to $STATE/name atomically with mode 0600.
func writeJSON(_ state: String, _ name: String, _ value: [String: Any]) throws {
    let data = try JSONSerialization.data(withJSONObject: value, options: [.sortedKeys, .withoutEscapingSlashes])
    let tmp = "\(state)/.\(name).tmp"
    guard FileManager.default.createFile(atPath: tmp, contents: data, attributes: [.posixPermissions: 0o600]) else {
        throw Refused("cannot write \(tmp)")
    }
    if rename(tmp, "\(state)/\(name)") != 0 {
        throw Refused("cannot rename \(tmp): \(String(cString: strerror(errno)))")
    }
}

func now() -> String {
    ISO8601DateFormatter().string(from: Date())
}

/// One heartbeat: asks for a token (kept in memory only, never printed), writes
/// $STATE/heartbeat.json and returns what it wrote.
public func heartbeat(environment env: [String: String], state: String, beats: Int) throws -> [String: Any] {
    var tok = "granted"
    do {
        _ = try token(scopes: serviceScopes, environment: env)
    } catch let e as Refused {
        complain("token refused: \(e)")
        tok = "refused"
    }
    let at = now()
    let beat: [String: Any] = [
        "id": serviceId,
        "version": serviceVersion,
        "state": "ok",
        "at": at,
        "beats": beats,
        "token": tok,
        "dev": env["IOHR_EXT_DEV"] == "1",
    ]
    try writeJSON(state, "heartbeat.json", beat)
    say("heartbeat \(beats) at \(at) (token \(tok))")
    return beat
}

/// The latest heartbeat, shared between the loop and the panel's thread.
public final class Latest: @unchecked Sendable {
    private let lock = NSLock()
    private var value: [String: Any] = [:]

    public init() {}

    public func set(_ new: [String: Any]) {
        lock.lock()
        value = new
        lock.unlock()
    }

    func json() -> Data {
        lock.lock()
        defer { lock.unlock() }
        return (try? JSONSerialization.data(withJSONObject: value, options: [.sortedKeys, .withoutEscapingSlashes])) ?? Data("{}".utf8)
    }
}

/// Where the panel page is: PANEL_FILE, or panel/index.html beside the program, or in the
/// folder it runs from.
func panelFile(_ env: [String: String]) -> String {
    if let named = env["PANEL_FILE"], !named.isEmpty {
        return named
    }
    let beside = URL(fileURLWithPath: CommandLine.arguments[0]).resolvingSymlinksInPath()
        .deletingLastPathComponent().appendingPathComponent("panel/index.html").path
    return FileManager.default.fileExists(atPath: beside) ? beside : "panel/index.html"
}

/// A panel on 127.0.0.1 only. `stop()` ends it.
public final class Panel: @unchecked Sendable {
    public let url: String
    private let fd: Int32
    private let running = NSLock()
    private var stopped = false

    init(fd: Int32, port: UInt16) {
        self.fd = fd
        url = "http://127.0.0.1:\(port)/"
    }

    var isStopped: Bool {
        running.lock()
        defer { running.unlock() }
        return stopped
    }

    public func stop() {
        running.lock()
        let first = !stopped
        stopped = true
        running.unlock()
        if first {
            // The accept loop polls, sees the flag and closes the socket.
            Thread.sleep(forTimeInterval: 0.3)
        }
    }

    fileprivate func listen(env: [String: String], latest: Latest) {
        while !isStopped {
            var ready = pollfd(fd: fd, events: Int16(POLLIN), revents: 0)
            if poll(&ready, 1, 100) <= 0 {
                continue
            }
            let client = accept(fd, nil, nil)
            if client < 0 {
                continue
            }
            answer(client, env: env, latest: latest)
            close(client)
        }
        close(fd)
    }
}

private func respond(_ client: Int32, _ status: String, _ type: String, _ body: Data) {
    let head = "HTTP/1.1 \(status)\r\ncontent-type: \(type)\r\ncontent-length: \(body.count)\r\nconnection: close\r\n\r\n"
    let bytes = Array(head.utf8) + Array(body)
    var sent = 0
    while sent < bytes.count {
        let n = bytes[sent...].withUnsafeBytes { send(client, $0.baseAddress, $0.count, 0) }
        if n <= 0 {
            return
        }
        sent += n
    }
}

private func answer(_ client: Int32, env: [String: String], latest: Latest) {
    var limit = timeval(tv_sec: 5, tv_usec: 0)
    setsockopt(client, SOL_SOCKET, SO_RCVTIMEO, &limit, socklen_t(MemoryLayout<timeval>.size))
    var raw = [UInt8]()
    var chunk = [UInt8](repeating: 0, count: 2048)
    while raw.count < 16 * 1024 {
        let n = recv(client, &chunk, chunk.count, 0)
        if n <= 0 {
            break
        }
        raw.append(contentsOf: chunk[0..<n])
        if String(decoding: raw, as: UTF8.self).contains("\r\n\r\n") {
            break
        }
    }
    let line = String(decoding: raw, as: UTF8.self).split(separator: "\r\n").first ?? ""
    let parts = line.split(separator: " ")
    let method = parts.first.map(String.init) ?? ""
    let path = parts.count > 1 ? String(parts[1]).split(separator: "?", omittingEmptySubsequences: false).first.map(String.init) ?? "" : ""
    if method == "GET", path == "/api/heartbeat" {
        respond(client, "200 OK", "application/json", latest.json())
    } else if method == "GET", path == "/" {
        let page = FileManager.default.contents(atPath: panelFile(env)) ?? Data("<p>panel/index.html is missing</p>".utf8)
        respond(client, "200 OK", "text/html; charset=utf-8", page)
    } else {
        respond(client, "404 Not Found", "text/plain", Data())
    }
}

/// Listens on 127.0.0.1 only; the port is PANEL_PORT or one the system picks, and it is
/// written to $STATE/panel.json so the app knows where the panel is.
public func servePanel(environment env: [String: String], state: String, latest: Latest) throws -> Panel {
    var wanted: UInt16 = 0
    if let raw = env["PANEL_PORT"], !raw.isEmpty {
        guard let port = UInt16(raw) else {
            throw Refused("PANEL_PORT is not a number: \(raw)")
        }
        wanted = port
    }
    let fd = socket(AF_INET, SOCK_STREAM, 0)
    if fd < 0 {
        throw Refused("no socket for the panel")
    }
    var on: Int32 = 1
    setsockopt(fd, SOL_SOCKET, SO_REUSEADDR, &on, socklen_t(MemoryLayout<Int32>.size))
    var addr = sockaddr_in()
    addr.sin_family = sa_family_t(AF_INET)
    addr.sin_port = wanted.bigEndian
    addr.sin_addr.s_addr = UInt32(0x7F00_0001).bigEndian
    let bound = withUnsafePointer(to: &addr) {
        $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
            bind(fd, $0, socklen_t(MemoryLayout<sockaddr_in>.size))
        }
    }
    if bound != 0 || listen(fd, 16) != 0 {
        let why = String(cString: strerror(errno))
        close(fd)
        throw Refused("the panel cannot listen: \(why)")
    }
    var actual = sockaddr_in()
    var size = socklen_t(MemoryLayout<sockaddr_in>.size)
    _ = withUnsafeMutablePointer(to: &actual) {
        $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { getsockname(fd, $0, &size) }
    }
    let panel = Panel(fd: fd, port: UInt16(bigEndian: actual.sin_port))
    let thread = Thread { panel.listen(env: env, latest: latest) }
    thread.start()
    try writeJSON(state, "panel.json", ["id": serviceId, "url": panel.url])
    say("panel at \(panel.url)")
    return panel
}

private let stopSignal = DispatchSemaphore(value: 0)

/// Runs the service until SIGINT or SIGTERM (or once, with IOHR_EXT_ONCE) and returns the exit code.
public func runService(environment env: [String: String]) -> Int32 {
    let state = env["STATE"] ?? ""
    if state.isEmpty || (env["CACHE"] ?? "").isEmpty {
        complain("STATE and CACHE are not set: run this with iohr ext dev")
        return 2
    }
    let every = Int(env["HEARTBEAT_EVERY"] ?? "").flatMap { $0 > 0 ? $0 : nil } ?? 60
    let once = !(env["IOHR_EXT_ONCE"] ?? "").isEmpty
    say("\(serviceId) \(serviceVersion) starting; state in \(state)")

    let latest = Latest()
    var panel: Panel?
    var sources = [DispatchSourceSignal]()
    defer { panel?.stop() }
    do {
        if !once {
            panel = try servePanel(environment: env, state: state, latest: latest)
            for sig in [SIGTERM, SIGINT] {
                signal(sig, SIG_IGN)
                let source = DispatchSource.makeSignalSource(signal: sig, queue: .global())
                source.setEventHandler { stopSignal.signal() }
                source.resume()
                sources.append(source)
            }
        }
        var beats = 1
        while true {
            latest.set(try heartbeat(environment: env, state: state, beats: beats))
            if once {
                return 0
            }
            if stopSignal.wait(timeout: .now() + .seconds(every)) == .success {
                say("stopping")
                return 0
            }
            beats += 1
        }
    } catch {
        complain("\(error)")
        return 1
    }
}
