// Tokens from iohr's extension channel (the SDK's ADR 0012; PRD 0084 slice 1).
//
// iohr passes IOHR_EXT_TOKEN_SOCKET (a 0600 Unix socket). The program sends
// `POST /token {"scopes": [...]}` and gets `{"access_token", "expires_at"}` for scopes its
// manifest declares, and a refusal for any other. It is the only credential this project
// has: never read IOHR_TOKEN or run `iohr auth token`.

import Foundation

private let maxAnswer = 64 * 1024
private let waitSeconds = 10

/// What `token` throws when iohr refuses, or its channel cannot be reached.
public struct Refused: Error, CustomStringConvertible {
    public let description: String

    public init(_ message: String) {
        description = message
    }
}

private func sendAll(_ fd: Int32, _ bytes: [UInt8]) -> Bool {
    var sent = 0
    while sent < bytes.count {
        let n = bytes[sent...].withUnsafeBytes { send(fd, $0.baseAddress, $0.count, 0) }
        if n <= 0 {
            return false
        }
        sent += n
    }
    return true
}

/// Returns an access token for `scopes`; the `Refused` message says iohr's reason.
public func token(scopes: [String], environment env: [String: String]) throws -> String {
    let path = (env["IOHR_EXT_TOKEN_SOCKET"] ?? "").trimmingCharacters(in: .whitespaces)
    if path.isEmpty {
        throw Refused("IOHR_EXT_TOKEN_SOCKET is not set: run this with iohr ext dev")
    }
    var addr = sockaddr_un()
    addr.sun_family = sa_family_t(AF_UNIX)
    let capacity = MemoryLayout.size(ofValue: addr.sun_path)
    guard path.utf8.count < capacity else {
        throw Refused("cannot reach iohr's token channel: the socket path is too long")
    }
    withUnsafeMutableBytes(of: &addr.sun_path) { raw in
        for (i, b) in path.utf8.enumerated() {
            raw[i] = b
        }
    }
    let fd = socket(AF_UNIX, SOCK_STREAM, 0)
    if fd < 0 {
        throw Refused("cannot reach iohr's token channel: no socket")
    }
    defer { close(fd) }
    var limit = timeval(tv_sec: waitSeconds, tv_usec: 0)
    setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &limit, socklen_t(MemoryLayout<timeval>.size))
    setsockopt(fd, SOL_SOCKET, SO_SNDTIMEO, &limit, socklen_t(MemoryLayout<timeval>.size))
    let linked = withUnsafePointer(to: &addr) {
        $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
            connect(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
        }
    }
    if linked != 0 {
        throw Refused("cannot reach iohr's token channel: \(String(cString: strerror(errno)))")
    }
    let body = (try? JSONSerialization.data(withJSONObject: ["scopes": scopes])) ?? Data()
    let head = "POST /token HTTP/1.1\r\nhost: iohr\r\ncontent-type: application/json\r\n"
        + "content-length: \(body.count)\r\nconnection: close\r\n\r\n"
    if !sendAll(fd, Array(head.utf8) + Array(body)) {
        throw Refused("cannot reach iohr's token channel: the write failed")
    }
    var raw = [UInt8]()
    var chunk = [UInt8](repeating: 0, count: 4096)
    while raw.count < maxAnswer {
        let n = recv(fd, &chunk, chunk.count, 0)
        if n <= 0 {
            break
        }
        raw.append(contentsOf: chunk[0..<n])
    }
    let text = String(decoding: raw.prefix(maxAnswer), as: UTF8.self)
    var status = ""
    var payload = ""
    if let cut = text.range(of: "\r\n\r\n") {
        let fields = text[..<cut.lowerBound].split(separator: " ", omittingEmptySubsequences: true)
        status = fields.count > 1 ? String(fields[1]) : ""
        payload = String(text[cut.upperBound...])
    }
    let answer = (try? JSONSerialization.jsonObject(with: Data(payload.utf8))) as? [String: Any] ?? [:]
    if status != "200" {
        let why = (answer["error"] as? String ?? "") + ": " + (answer["message"] as? String ?? "")
        throw Refused("\(status.isEmpty ? "no answer" : status) \(why)")
    }
    guard let access = answer["access_token"] as? String, !access.isEmpty else {
        throw Refused("iohr's answer holds no token")
    }
    return access
}
