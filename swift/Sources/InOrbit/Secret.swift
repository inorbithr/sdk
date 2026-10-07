/// A credential: a key secret or an access token. It prints as `<redacted>` in every
/// formatting path (`print`, string interpolation, `debugPrint`, `dump`), and the value is
/// read only where it is sent [SR-10, SR-12].
public struct Secret: Sendable, Equatable, CustomStringConvertible, CustomDebugStringConvertible,
    CustomReflectable
{
    private let value: String

    /// A secret holding `value`.
    public init(_ value: String) {
        self.value = value
    }

    /// The value, for the one place that sends it.
    public func reveal() -> String {
        value
    }

    /// Whether the value is empty.
    public var isEmpty: Bool {
        value.isEmpty
    }

    public var description: String {
        "<redacted>"
    }

    public var debugDescription: String {
        "Secret(<redacted>)"
    }

    public var customMirror: Mirror {
        Mirror(self, children: ["value": "<redacted>"])
    }
}
