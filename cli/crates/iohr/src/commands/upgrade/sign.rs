//! A stable macOS signature for the installed `iohr` (platform RFC 0206, slice 1).
//!
//! Releases are signed ad hoc by the linker, so their designated requirement is the exact
//! code hash. The keychain keeps that requirement in each item's access list when the
//! person clicks "Always Allow", and every release is a new program to it: after each
//! upgrade, every item `iohr` reads asks for the password again. With
//! `upgrade_sign_identity` (or [`ENV`]) set, the install signs the new binary with a
//! certificate from this Mac's keychain and the fixed identifier [`IDENTIFIER`] before it
//! checks and renames it, so the requirement (identifier and certificate) stays the same
//! from one release to the next.
//!
//! A binary that is already signed with a certificate is left as it is. Signing never
//! fails an install: on any failure the ad-hoc binary goes on and the caller says why,
//! once. `security` is asked for the signing identities only (`find-identity`): no
//! keychain item or access list is read or changed here.

use std::ffi::OsString;
use std::fmt;
use std::path::Path;
use std::time::{Duration, Instant};

/// The identifier every signed `iohr` carries: part of the designated requirement.
pub(crate) const IDENTIFIER: &str = "hr.inorbit.iohr";
/// The environment variable over `upgrade_sign_identity`, with the same values.
pub(crate) const ENV: &str = "IOHR_SIGN_IDENTITY";
/// How long `codesign` may take to sign (a key that asks for a password never answers
/// under launchd).
pub(crate) const SIGN_LIMIT: Duration = Duration::from_secs(30);
/// How long the look at the current signature and the identity list may take.
const LOOK_LIMIT: Duration = Duration::from_secs(10);
const CODESIGN: &str = "/usr/bin/codesign";
const SECURITY: &str = "/usr/bin/security";
/// The kinds of identity `auto` takes, in order (the order of the Mac app's `release.sh`).
const AUTO_KINDS: [&str; 2] = ["Developer ID Application:", "Apple Development:"];

/// What `upgrade_sign_identity` says.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) enum SignWith {
    /// Unset, empty or `-`: the release's own signature stays.
    #[default]
    Off,
    /// `auto`: the first valid Developer ID Application identity, else the first Apple
    /// Development one.
    Auto,
    /// Any other value: an identity as `codesign --sign` takes it (its name, or the SHA-1
    /// hash `security find-identity` shows).
    Named(String),
}

impl SignWith {
    /// The setting's value read: `-` and empty are off, `auto` picks, anything else names.
    pub(crate) fn parse(value: &str) -> Self {
        match value.trim() {
            "" | "-" => Self::Off,
            "auto" => Self::Auto,
            other => Self::Named(other.to_owned()),
        }
    }
}

impl fmt::Display for SignWith {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Off => f.write_str("off"),
            Self::Auto => f.write_str("auto"),
            Self::Named(n) => f.write_str(n),
        }
    }
}

/// A signing identity in the keychain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Identity {
    /// The certificate's SHA-1 hash, 40 hex digits: what `codesign` is given, so two
    /// certificates with one name are never ambiguous.
    pub(crate) hash: String,
    pub(crate) name: String,
}

/// The identity `auto` signs with, from the output of
/// `security find-identity -v -p codesigning`: the first Developer ID Application one,
/// else the first Apple Development one, else none.
pub(crate) fn pick(listing: &str) -> Option<Identity> {
    let all: Vec<Identity> = listing.lines().filter_map(identity_of).collect();
    AUTO_KINDS
        .iter()
        .find_map(|kind| all.iter().find(|i| i.name.starts_with(kind)).cloned())
}

/// One line of the listing: `  1) <40 hex> "<name>"`. Anything else is not an identity.
fn identity_of(line: &str) -> Option<Identity> {
    let (number, rest) = line.trim().split_once(") ")?;
    let (hash, quoted) = rest.trim_start().split_once(' ')?;
    let name = quoted.trim().strip_prefix('"')?.strip_suffix('"')?;
    let digits = !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit());
    let hex = hash.len() == 40 && hash.bytes().all(|b| b.is_ascii_hexdigit());
    (digits && hex && !name.is_empty()).then(|| Identity {
        hash: hash.to_owned(),
        name: name.to_owned(),
    })
}

/// Whether `codesign -dvv` says the code is signed with a certificate: an `Authority=`
/// line (`-dv` never prints one; `-dvv` does). Ad-hoc code (`Signature=adhoc`) and unsigned
/// code have none.
pub(crate) fn has_certificate(details: &str) -> bool {
    details
        .lines()
        .any(|l| l.trim_start().starts_with("Authority="))
}

/// What a program said: whether it exited 0, and its standard output and error together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Ran {
    pub(crate) ok: bool,
    pub(crate) said: String,
}

/// Runs the two programs signing needs; the tests answer for them.
pub(crate) trait Tools {
    /// Runs `program` with `args` and stops it after `limit`.
    ///
    /// # Errors
    /// It could not start, or it was stopped at the limit.
    fn run(&self, program: &str, args: &[OsString], limit: Duration) -> Result<Ran, String>;
}

/// This machine's `codesign` and `security`, by absolute path.
pub(crate) struct System;

impl Tools for System {
    fn run(&self, program: &str, args: &[OsString], limit: Duration) -> Result<Ran, String> {
        use std::io::Read;
        use std::process::Stdio;
        let mut child = std::process::Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("cannot run {program}: {e}"))?;
        let drain = |pipe: Option<Box<dyn Read + Send>>| {
            std::thread::spawn(move || {
                let mut bytes = Vec::new();
                if let Some(mut p) = pipe {
                    let _ = p.read_to_end(&mut bytes);
                }
                bytes
            })
        };
        let out = drain(
            child
                .stdout
                .take()
                .map(|p| Box::new(p) as Box<dyn Read + Send>),
        );
        let err = drain(
            child
                .stderr
                .take()
                .map(|p| Box::new(p) as Box<dyn Read + Send>),
        );
        let deadline = Instant::now() + limit;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() >= deadline => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!(
                        "{program} was stopped after {} seconds",
                        limit.as_secs()
                    ));
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(50)),
                Err(e) => return Err(format!("{program}: {e}")),
            }
        };
        let mut said = out.join().unwrap_or_default();
        said.extend(err.join().unwrap_or_default());
        Ok(Ran {
            ok: status.success(),
            said: String::from_utf8_lossy(&said).into_owned(),
        })
    }
}

/// What signing did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// Signing is off, or this is not macOS: nothing was run.
    Off,
    /// The release is signed with a certificate already and stays as it is.
    AlreadySigned,
    /// Signed with the identity of this name.
    Signed(String),
    /// Not signed, and why: the ad-hoc binary goes on.
    Kept(String),
}

/// Signs the binary at `path` when `with` asks and it is ad hoc (or unsigned): on macOS
/// with this machine's tools, elsewhere never.
pub(crate) fn here(path: &Path, with: &SignWith) -> Outcome {
    if cfg!(target_os = "macos") {
        sign(path, with, &System)
    } else {
        Outcome::Off
    }
}

/// Signs the binary at `path` when `with` asks and it is ad hoc (or unsigned), with
/// `codesign --force --sign <identity> --identifier hr.inorbit.iohr <path>`.
pub(crate) fn sign(path: &Path, with: &SignWith, tools: &dyn Tools) -> Outcome {
    if *with == SignWith::Off {
        return Outcome::Off;
    }
    let arg = |s: &str| OsString::from(s);
    // `codesign -dvv` exits 1 for code that is not signed at all: that is signed too.
    match tools.run(CODESIGN, &[arg("-dvv"), path.into()], LOOK_LIMIT) {
        Ok(r) if r.ok && has_certificate(&r.said) => return Outcome::AlreadySigned,
        Ok(_) => {}
        Err(e) => return Outcome::Kept(e),
    }
    let (identity, name) = match with {
        SignWith::Off => return Outcome::Off,
        SignWith::Named(n) => (n.clone(), n.clone()),
        SignWith::Auto => {
            let listing = match tools.run(
                SECURITY,
                &[
                    arg("find-identity"),
                    arg("-v"),
                    arg("-p"),
                    arg("codesigning"),
                ],
                LOOK_LIMIT,
            ) {
                Ok(r) if r.ok => r.said,
                Ok(r) => {
                    return Outcome::Kept(format!(
                        "security find-identity failed: {}",
                        last_line(&r.said)
                    ));
                }
                Err(e) => return Outcome::Kept(e),
            };
            match pick(&listing) {
                Some(i) => (i.hash, i.name),
                None => {
                    return Outcome::Kept(
                        "the keychain holds no Developer ID Application or Apple Development \
                         identity"
                            .into(),
                    );
                }
            }
        }
    };
    match tools.run(
        CODESIGN,
        &[
            arg("--force"),
            arg("--sign"),
            arg(&identity),
            arg("--identifier"),
            arg(IDENTIFIER),
            path.into(),
        ],
        SIGN_LIMIT,
    ) {
        Ok(r) if r.ok => Outcome::Signed(name),
        Ok(r) => Outcome::Kept(format!(
            "codesign with {name:?} failed: {}",
            last_line(&r.said)
        )),
        Err(e) => Outcome::Kept(format!("codesign with {name:?}: {e}")),
    }
}

/// The last line a tool printed, short enough for a note.
fn last_line(said: &str) -> String {
    said.lines()
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("no message")
        .chars()
        .take(200)
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use std::cell::RefCell;
    use std::ffi::OsString;
    use std::path::Path;
    use std::time::Duration;

    use super::{Identity, Outcome, Ran, SignWith, Tools, has_certificate, pick, sign};

    /// The listing's shape, as `security find-identity -v -p codesigning` prints it.
    const BOTH: &str =
        "  1) 0123456789ABCDEF0123456789ABCDEF01234567 \"Apple Development: Someone (ABC123)\"
  2) 89ABCDEF0123456789ABCDEF0123456789ABCDEF \"Developer ID Application: Someone (TEAM)\"
     2 valid identities found
";
    const DEVELOPMENT: &str =
        "  1) 0123456789ABCDEF0123456789ABCDEF01234567 \"Apple Development: Someone (ABC123)\"
  2) FEDCBA9876543210FEDCBA9876543210FEDCBA98 \"Apple Distribution: Someone (TEAM)\"
     2 valid identities found
";
    const AD_HOC: &str = "Executable=/tmp/.iohr.upgrade-1\nIdentifier=iohr-1234\nFormat=Mach-O thin (arm64)\nCodeDirectory v=20400 size=1 flags=0x20002(adhoc,linker-signed) hashes=1+0 location=embedded\nSignature=adhoc\nTeamIdentifier=not set\n";
    const DEVELOPER_ID: &str = "Executable=/tmp/.iohr.upgrade-1\nIdentifier=hr.inorbit.iohr\nAuthority=Developer ID Application: Someone (TEAM)\nAuthority=Developer ID Certification Authority\nAuthority=Apple Root CA\nTeamIdentifier=TEAM\n";

    /// Answers each call with the next canned result and records what was asked.
    struct Fake {
        answers: RefCell<Vec<Result<Ran, String>>>,
        calls: RefCell<Vec<(String, Vec<String>, Duration)>>,
    }

    impl Fake {
        fn new(answers: Vec<Result<Ran, String>>) -> Self {
            Self {
                answers: RefCell::new(answers.into_iter().rev().collect()),
                calls: RefCell::new(Vec::new()),
            }
        }
        fn programs(&self) -> Vec<String> {
            self.calls.borrow().iter().map(|c| c.0.clone()).collect()
        }
    }

    impl Tools for Fake {
        fn run(&self, program: &str, args: &[OsString], limit: Duration) -> Result<Ran, String> {
            self.calls.borrow_mut().push((
                program.to_owned(),
                args.iter()
                    .map(|a| a.to_string_lossy().into_owned())
                    .collect(),
                limit,
            ));
            self.answers
                .borrow_mut()
                .pop()
                .expect("a call nobody expected")
        }
    }

    #[allow(clippy::unnecessary_wraps, reason = "the fake answers with results")]
    fn ran(ok: bool, said: &str) -> Result<Ran, String> {
        Ok(Ran {
            ok,
            said: said.to_owned(),
        })
    }

    #[test]
    fn auto_takes_developer_id_first_then_apple_development() {
        assert_eq!(
            pick(BOTH),
            Some(Identity {
                hash: "89ABCDEF0123456789ABCDEF0123456789ABCDEF".into(),
                name: "Developer ID Application: Someone (TEAM)".into(),
            })
        );
        assert_eq!(
            pick(DEVELOPMENT).map(|i| i.name),
            Some("Apple Development: Someone (ABC123)".into())
        );
        assert_eq!(pick("     0 valid identities found\n"), None);
        assert_eq!(pick(""), None);
        // A line that is not `n) <40 hex> "<name>"` is no identity.
        assert_eq!(
            pick(
                "  1) 0123 \"Apple Development: Short (X)\"\n  x) 0123456789ABCDEF0123456789ABCDEF01234567 \"Apple Development: Y (Z)\"\n"
            ),
            None
        );
    }

    #[test]
    fn the_setting_reads_off_auto_or_a_name() {
        assert_eq!(SignWith::parse(""), SignWith::Off);
        assert_eq!(SignWith::parse(" - "), SignWith::Off);
        assert_eq!(SignWith::parse("auto"), SignWith::Auto);
        assert_eq!(
            SignWith::parse("Apple Development: Someone (ABC123)"),
            SignWith::Named("Apple Development: Someone (ABC123)".into())
        );
        assert_eq!(SignWith::default(), SignWith::Off);
        assert_eq!(SignWith::Off.to_string(), "off");
    }

    #[test]
    fn a_certificate_shows_as_an_authority_line() {
        assert!(has_certificate(DEVELOPER_ID));
        assert!(!has_certificate(AD_HOC));
        assert!(!has_certificate(
            "/tmp/x: code object is not signed at all\n"
        ));
    }

    #[test]
    fn off_runs_nothing() {
        let tools = Fake::new(vec![]);
        assert_eq!(
            sign(Path::new("/tmp/x"), &SignWith::Off, &tools),
            Outcome::Off
        );
        assert!(tools.calls.borrow().is_empty());
    }

    #[test]
    fn a_release_signed_with_a_certificate_is_left_alone() {
        let tools = Fake::new(vec![ran(true, DEVELOPER_ID)]);
        assert_eq!(
            sign(Path::new("/tmp/x"), &SignWith::Auto, &tools),
            Outcome::AlreadySigned
        );
        assert_eq!(tools.programs(), ["/usr/bin/codesign"]);
    }

    #[test]
    fn auto_signs_an_ad_hoc_binary_by_hash_with_the_fixed_identifier() {
        let tools = Fake::new(vec![ran(true, AD_HOC), ran(true, BOTH), ran(true, "")]);
        let path = Path::new("/opt/bin/.iohr.upgrade-7");
        assert_eq!(
            sign(path, &SignWith::Auto, &tools),
            Outcome::Signed("Developer ID Application: Someone (TEAM)".into())
        );
        let calls = tools.calls.borrow();
        assert_eq!(calls[0].1, ["-dvv", "/opt/bin/.iohr.upgrade-7"]);
        assert_eq!(calls[1].0, "/usr/bin/security");
        assert_eq!(calls[1].1, ["find-identity", "-v", "-p", "codesigning"]);
        assert_eq!(calls[2].0, "/usr/bin/codesign");
        assert_eq!(
            calls[2].1,
            [
                "--force",
                "--sign",
                "89ABCDEF0123456789ABCDEF0123456789ABCDEF",
                "--identifier",
                "hr.inorbit.iohr",
                "/opt/bin/.iohr.upgrade-7"
            ]
        );
        assert_eq!(calls[2].2, Duration::from_secs(30));
    }

    #[test]
    fn an_unsigned_binary_is_signed_too() {
        let tools = Fake::new(vec![
            ran(false, "/tmp/x: code object is not signed at all"),
            ran(true, ""),
        ]);
        assert_eq!(
            sign(Path::new("/tmp/x"), &SignWith::Named("Me".into()), &tools),
            Outcome::Signed("Me".into())
        );
    }

    #[test]
    fn a_named_identity_skips_the_lookup() {
        let tools = Fake::new(vec![ran(true, AD_HOC), ran(true, "")]);
        let with = SignWith::Named("Apple Development: Someone (ABC123)".into());
        assert_eq!(
            sign(Path::new("/tmp/x"), &with, &tools),
            Outcome::Signed("Apple Development: Someone (ABC123)".into())
        );
        assert_eq!(tools.programs(), ["/usr/bin/codesign", "/usr/bin/codesign"]);
        assert_eq!(
            tools.calls.borrow()[1].1[2],
            "Apple Development: Someone (ABC123)"
        );
    }

    #[test]
    fn every_failure_keeps_the_ad_hoc_binary_and_says_why() {
        let none = Fake::new(vec![
            ran(true, AD_HOC),
            ran(true, "     0 valid identities found\n"),
        ]);
        assert!(matches!(
            sign(Path::new("/tmp/x"), &SignWith::Auto, &none),
            Outcome::Kept(why) if why.contains("no Developer ID Application or Apple Development")
        ));
        let refused = Fake::new(vec![
            ran(true, AD_HOC),
            ran(
                false,
                "Me: no identity found\n/tmp/x: errSecInternalComponent\n",
            ),
        ]);
        assert!(matches!(
            sign(Path::new("/tmp/x"), &SignWith::Named("Me".into()), &refused),
            Outcome::Kept(why) if why.ends_with("errSecInternalComponent")
        ));
        let slow = Fake::new(vec![
            ran(true, AD_HOC),
            Err("/usr/bin/codesign was stopped after 30 seconds".into()),
        ]);
        assert!(matches!(
            sign(Path::new("/tmp/x"), &SignWith::Named("Me".into()), &slow),
            Outcome::Kept(why) if why.contains("stopped after 30 seconds")
        ));
        let missing = Fake::new(vec![Err("cannot run /usr/bin/codesign: not found".into())]);
        assert!(matches!(
            sign(Path::new("/tmp/x"), &SignWith::Auto, &missing),
            Outcome::Kept(_)
        ));
    }
}
