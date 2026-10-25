import Foundation

/// A 64-bit integer as it travels: a decimal string on the wire, an `Int64` in Swift. The
/// generated models decode and encode through it and expose plain `Int64`s; it is the one
/// place the conversion happens.
public struct WireInt64: Codable, Sendable, Equatable {
    /// The value.
    public let value: Int64

    /// The wire form of `value`.
    public init(_ value: Int64) {
        self.value = value
    }

    public init(from decoder: any Decoder) throws {
        let c = try decoder.singleValueContainer()
        if let s = try? c.decode(String.self) {
            guard let v = Int64(s) else {
                throw DecodingError.dataCorruptedError(
                    in: c, debugDescription: "not a decimal 64-bit integer")
            }
            value = v
        } else {
            // A document that sends the number as a JSON number is read too.
            value = try c.decode(Int64.self)
        }
    }

    public func encode(to encoder: any Encoder) throws {
        var c = encoder.singleValueContainer()
        try c.encode(String(value))
    }
}

/// RFC 3339 timestamps as the API sends them.
public enum Timestamps {
    /// The instant `text` names, or `nil` for the empty string, which is how the API sends
    /// an unset timestamp (design.md section 12, rule N5), and for text that is not one.
    public static func parse(_ text: String) -> Date? {
        if text.isEmpty { return nil }
        let fractional = ISO8601DateFormatter()
        fractional.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        if let d = fractional.date(from: text) { return d }
        let plain = ISO8601DateFormatter()
        plain.formatOptions = [.withInternetDateTime]
        return plain.date(from: text)
    }

    /// `date` as the API writes it (UTC, seconds).
    public static func format(_ date: Date) -> String {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime]
        return f.string(from: date)
    }
}
