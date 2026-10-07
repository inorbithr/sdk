/// A profile: the credential of one account. A generated surface defines one type per
/// profile, adopting the marker protocol of each operation its cut holds, so a call the
/// profile may not make does not compile (design.md section 12).
public protocol Profile: Sendable {
    /// The profile's name (`acme-ci`).
    static var name: String { get }
    /// What its environment variables carry (`ACME_CI` in `INORBIT_ACME_CI_TOKEN`); empty for
    /// the public profile, which reads the bare `INORBIT_*`.
    static var env: String { get }
}

/// The public profile: every operation of the public document, `spec/openapi.json`. The
/// package's own surface adds its markers.
public enum Public: Profile {
    public static let name = "public"
    public static let env = ""
}
