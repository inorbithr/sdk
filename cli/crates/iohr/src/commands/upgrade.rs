//! `iohr upgrade`: bring this `iohr` to the newest release, the way it was installed.
//!
//! - Homebrew: runs `brew upgrade inorbithr/tap/iohr`. Homebrew refuses a tap it has not
//!   been told to trust; the error is answered with `brew trust inorbithr/tap`.
//! - The install script (`~/.local/bin`, or `IOHR_INSTALL_DIR`): downloads the newest
//!   archive, the same one `cli/install/install.sh` installs, and checks it as that
//!   script does (the archive's SHA-256 against the release's `SHA256SUMS`) and then
//!   more: the GitHub attestations of the archive are verified in process by
//!   [`crate::ext::trust::verify_iohr_release`], the verifier extensions use, so no
//!   `gh` is needed and the check never skips. The binary is replaced atomically.
//! - APT and everything else: prints what to run.
//!
//! `--check` only says whether a newer release exists and what would run.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;

use crate::cli::Global;
use crate::error::Error;
use crate::ext::layer;
use crate::ext::oci::Digest;
use crate::ext::trust;
use crate::output::Out;

/// Where releases are listed and attestations read (`GET /releases`, `/attestations/..`).
const API: &str = "https://api.github.com/repos/inorbithr/sdk";
/// Where release files download from.
const DOWNLOAD: &str = "https://github.com/inorbithr/sdk/releases/download";
const TAP: &str = "inorbithr/tap";
/// The most bytes read of a release listing, of `SHA256SUMS`, of the attestations and of
/// an archive.
const MAX_LISTING: usize = 8 * 1024 * 1024;
const MAX_SUMS: usize = 64 * 1024;
const MAX_ATTESTATIONS: usize = 8 * 1024 * 1024;
const MAX_ARCHIVE: usize = 200 * 1024 * 1024;

/// How this binary got here, which decides how it upgrades.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "method", rename_all = "snake_case")]
pub(crate) enum Method {
    /// Under a Homebrew prefix or Cellar.
    Homebrew,
    /// In the install script's directory.
    InstallScript { dir: PathBuf },
    /// The Debian package's `/usr/bin/iohr`.
    Apt,
    /// A place iohr does not manage (a build, winget, a copy).
    Other,
}

/// What the machine says about where things live; a value so tests need no environment.
#[derive(Debug, Clone, Default)]
pub(crate) struct Layout {
    /// `$HOME`.
    pub(crate) home: Option<PathBuf>,
    /// `IOHR_INSTALL_DIR`.
    pub(crate) install_dir: Option<PathBuf>,
    /// `HOMEBREW_PREFIX`.
    pub(crate) brew_prefix: Option<PathBuf>,
}

impl Layout {
    fn from_process() -> Self {
        let var = |k: &str| {
            std::env::var_os(k)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        };
        Self {
            home: var("HOME").or_else(|| var("USERPROFILE")),
            install_dir: var("IOHR_INSTALL_DIR"),
            brew_prefix: var("HOMEBREW_PREFIX"),
        }
    }
}

/// The install method of the binary at `exe` (as run) whose real path is `real`.
pub(crate) fn detect(exe: &Path, real: &Path, layout: &Layout) -> Method {
    let prefixes: Vec<PathBuf> = layout
        .brew_prefix
        .iter()
        .cloned()
        .chain(["/opt/homebrew", "/home/linuxbrew/.linuxbrew"].map(PathBuf::from))
        .collect();
    let in_cellar = |p: &Path| {
        let parts: Vec<_> = p
            .components()
            .map(std::path::Component::as_os_str)
            .collect();
        parts.windows(2).any(|w| w[0] == "Cellar" && w[1] == "iohr")
    };
    if in_cellar(real)
        || in_cellar(exe)
        || prefixes
            .iter()
            .any(|p| real.starts_with(p) || exe.starts_with(p))
    {
        return Method::Homebrew;
    }
    let dir = layout
        .install_dir
        .clone()
        .or_else(|| layout.home.as_ref().map(|h| h.join(".local").join("bin")));
    if let (Some(dir), Some(parent)) = (dir, exe.parent()) {
        // The directory's real path too: `~/.local/bin` may itself be a link.
        let same = |a: &Path, b: &Path| {
            a == b || matches!((a.canonicalize(), b.canonicalize()), (Ok(x), Ok(y)) if x == y)
        };
        if same(parent, &dir) && real.file_name() == exe.file_name() {
            return Method::InstallScript { dir };
        }
    }
    if real == Path::new("/usr/bin/iohr") {
        return Method::Apt;
    }
    Method::Other
}

/// The release archive's target for an OS and CPU, as `install.sh` names it.
pub(crate) fn target(os: &str, arch: &str) -> Option<String> {
    let arch = match arch {
        "x86_64" => "x86_64",
        "aarch64" => "aarch64",
        _ => return None,
    };
    let os = match os {
        "linux" => "unknown-linux-musl",
        "macos" => "apple-darwin",
        _ => return None,
    };
    Some(format!("{arch}-{os}"))
}

/// The newest `iohr/v*` release in a GitHub release listing: not a draft, a valid
/// semantic version, the highest by semver precedence (not by listing order).
pub(crate) fn newest(listing: &[u8]) -> Option<semver::Version> {
    let items: Vec<Value> = serde_json::from_slice(listing).ok()?;
    items
        .iter()
        .filter(|r| r.get("draft").and_then(Value::as_bool) != Some(true))
        .filter_map(|r| r.get("tag_name")?.as_str()?.strip_prefix("iohr/v"))
        .filter_map(|v| semver::Version::parse(v).ok())
        .max()
}

/// The checksum `SHA256SUMS` lists for `file` (`<hex>  <file>` or `<hex> *<file>`).
pub(crate) fn sum_of(sums: &str, file: &str) -> Option<String> {
    sums.lines().find_map(|l| {
        let (hex, name) = l.split_once(char::is_whitespace)?;
        let name = name.trim_start().trim_start_matches('*');
        (name == file).then(|| hex.to_owned())
    })
}

/// What `--check` reports.
#[derive(Debug, Serialize)]
pub(crate) struct Report {
    current: String,
    latest: String,
    update_available: bool,
    #[serde(flatten)]
    install: Method,
    /// What `iohr upgrade` would run, or what to run yourself.
    command: String,
}

fn command_for(method: &Method) -> String {
    match method {
        Method::Homebrew => format!("brew upgrade {TAP}/iohr"),
        Method::InstallScript { dir } => format!(
            "download, verify and replace {}",
            dir.join("iohr").display()
        ),
        Method::Apt => "sudo apt-get update && sudo apt-get install --only-upgrade iohr".into(),
        Method::Other => {
            "none: iohr does not know how this copy was installed (see the install docs)".into()
        }
    }
}

fn http(base: &str) -> Result<reqwest::Client, Error> {
    // https only, except a loopback address (a test double).
    let local = url::Url::parse(base)
        .ok()
        .and_then(|u| {
            u.host_str()
                .map(|h| matches!(h, "127.0.0.1" | "localhost" | "[::1]"))
        })
        .unwrap_or(false);
    reqwest::Client::builder()
        .user_agent(format!("iohr/{}", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(600))
        .https_only(!local)
        .build()
        .map_err(|e| Error::Failed(format!("cannot start the HTTP client: {e}")))
}

/// A body of at most `max` bytes.
async fn get(c: &reqwest::Client, url: &str, max: usize) -> Result<Vec<u8>, Error> {
    let mut resp = c
        .get(url)
        .send()
        .await
        .map_err(|e| Error::Failed(format!("{url}: {}", e.without_url())))?;
    if !resp.status().is_success() {
        return Err(Error::Failed(format!("{url}: {}", resp.status())));
    }
    let mut body = Vec::new();
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| Error::Failed(format!("{url}: {}", e.without_url())))?
    {
        if body.len() + chunk.len() > max {
            return Err(Error::Failed(format!("{url}: larger than {max} bytes")));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

struct Sources {
    api: String,
    download: String,
}

impl Sources {
    /// `IOHR_UPGRADE_API` and `IOHR_UPGRADE_DOWNLOAD` point the lookup elsewhere (tests,
    /// a mirror). The archive is still checked against the release workflow's
    /// attestations, so a mirror is not trusted.
    fn from_process() -> Self {
        let var = |k: &str, d: &str| {
            std::env::var(k)
                .ok()
                .filter(|v| !v.trim().is_empty())
                .unwrap_or_else(|| d.to_owned())
                .trim_end_matches('/')
                .to_owned()
        };
        Self {
            api: var("IOHR_UPGRADE_API", API),
            download: var("IOHR_UPGRADE_DOWNLOAD", DOWNLOAD),
        }
    }
}

pub(crate) async fn run(g: &Global, check: bool, out: Out) -> Result<(), Error> {
    let exe = std::env::current_exe()
        .map_err(|e| Error::Failed(format!("cannot tell where iohr is: {e}")))?;
    let real = exe.canonicalize().unwrap_or_else(|_| exe.clone());
    let method = detect(&exe, &real, &Layout::from_process());
    let src = Sources::from_process();
    let client = http(&src.api)?;

    let current = semver::Version::parse(env!("CARGO_PKG_VERSION"))
        .map_err(|e| Error::Failed(format!("this build's version: {e}")))?;
    let listing = get(
        &client,
        &format!("{}/releases?per_page=100", src.api),
        MAX_LISTING,
    )
    .await?;
    let latest =
        newest(&listing).ok_or_else(|| Error::Failed("no iohr release found on GitHub".into()))?;
    let update = latest > current;

    if check {
        let report = Report {
            current: current.to_string(),
            latest: latest.to_string(),
            update_available: update,
            command: command_for(&method),
            install: method,
        };
        if out.json {
            Out::print_json(&report);
        } else {
            Out::pairs(&[
                ("current", report.current.clone()),
                ("latest", report.latest.clone()),
                (
                    "update",
                    if update { "available" } else { "none" }.to_owned(),
                ),
                ("command", report.command.clone()),
            ]);
        }
        return Ok(());
    }
    if !update {
        Out::note(&format!("iohr {current} is the newest release."));
        if out.json {
            Out::print_json(&serde_json::json!({
                "current": current.to_string(), "latest": latest.to_string(),
                "upgraded": false
            }));
        }
        return Ok(());
    }

    match &method {
        Method::Homebrew => brew(g, &latest),
        Method::InstallScript { dir } => {
            install_script(&client, &src, &latest, &dir.join("iohr"), g.verbose).await?;
            Out::note(&format!("iohr is now {latest} (was {current})."));
            Ok(())
        }
        Method::Apt => Err(Error::Failed(format!(
            "iohr {latest} is out. This copy came from the APT package; run: {}",
            command_for(&method)
        ))),
        Method::Other => Err(Error::Failed(format!(
            "iohr {latest} is out, but this copy ({}) is not one iohr installed. Install it again \
             with Homebrew (`brew install {TAP}/iohr`), the install script \
             (`curl -fsSL https://packages.inorbit.hr/install.sh | sh`), APT or winget.",
            real.display()
        ))),
    }
}

/// `brew upgrade`, and the trust message when Homebrew refuses the tap.
fn brew(_g: &Global, latest: &semver::Version) -> Result<(), Error> {
    Out::note(&format!(
        "Running: brew upgrade {TAP}/iohr (iohr {latest} is out)"
    ));
    let o = std::process::Command::new("brew")
        .args(["upgrade", &format!("{TAP}/iohr")])
        .output()
        .map_err(|e| {
            Error::Failed(format!(
                "cannot run brew ({e}); run it yourself: brew upgrade {TAP}/iohr"
            ))
        })?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
    Out::note(text.trim_end());
    if o.status.success() {
        return Ok(());
    }
    Err(Error::Failed(brew_failure(&text)))
}

/// What to tell the person when `brew upgrade` failed with `text`.
pub(crate) fn brew_failure(text: &str) -> String {
    if text.contains("untrusted tap") || text.contains("brew trust") {
        format!(
            "Homebrew does not trust the {TAP} tap yet. Run once: brew trust {TAP}  then run iohr upgrade again."
        )
    } else {
        format!("brew upgrade {TAP}/iohr failed; its output is above.")
    }
}

/// Downloads, verifies and installs release `version` over `dest`.
async fn install_script(
    client: &reqwest::Client,
    src: &Sources,
    version: &semver::Version,
    dest: &Path,
    verbose: bool,
) -> Result<(), Error> {
    let triple = target(std::env::consts::OS, std::env::consts::ARCH).ok_or_else(|| {
        Error::Failed(format!(
            "there is no iohr build for {} on {}",
            std::env::consts::ARCH,
            std::env::consts::OS
        ))
    })?;
    let name = format!("iohr-{version}-{triple}");
    let file = format!("{name}.tar.gz");
    let base = format!("{}/iohr/v{version}", src.download);
    if verbose {
        Out::note(&format!("downloading {base}/{file}"));
    }
    let archive = get(client, &format!("{base}/{file}"), MAX_ARCHIVE).await?;
    let sums = get(client, &format!("{base}/SHA256SUMS"), MAX_SUMS).await?;
    let want = sum_of(&String::from_utf8_lossy(&sums), &file)
        .ok_or_else(|| Error::Failed(format!("{file} is not in SHA256SUMS")))?;
    let digest = Digest::of(&archive);
    if digest.hex() != want {
        return Err(Error::Failed(format!(
            "checksum mismatch for {file} (expected {want}, got {}); nothing was installed",
            digest.hex()
        )));
    }
    Out::note(&format!("checksum ok ({})", digest.hex()));

    let att = get(
        client,
        &format!("{}/attestations/{}", src.api, digest.as_str()),
        MAX_ATTESTATIONS,
    )
    .await?;
    let bundles = bundles_of(&att);
    let verified = trust::verify_iohr_release(&digest, &bundles)
        .map_err(|e| Error::Failed(format!("{e}; nothing was installed")))?;
    Out::note(&format!(
        "attestation ok (built by {})",
        verified
            .provenance
            .as_deref()
            .unwrap_or("the release workflow")
    ));

    let program = layer::program(&archive, &format!("{name}/iohr"))
        .map_err(|e| Error::Failed(format!("{e}; nothing was installed")))?;
    replace(dest, &program, version)
}

/// The bundles in an attestations API answer.
pub(crate) fn bundles_of(answer: &[u8]) -> Vec<Vec<u8>> {
    serde_json::from_slice::<Value>(answer)
        .ok()
        .and_then(|v| v.get("attestations")?.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|a| serde_json::to_vec(a.get("bundle")?).ok())
        .collect()
}

/// Writes `program` beside `dest`, checks that it runs and says `version`, then renames
/// it over `dest` (atomic on one filesystem). On any failure `dest` is untouched and the
/// temporary file is removed.
pub(crate) fn replace(dest: &Path, program: &[u8], version: &semver::Version) -> Result<(), Error> {
    let dir = dest
        .parent()
        .ok_or_else(|| Error::Failed("the install path has no directory".into()))?;
    let tmp = dir.join(format!(".iohr.upgrade-{}", std::process::id()));
    let result = (|| -> Result<(), Error> {
        let fail = |what: &str, e: &dyn std::fmt::Display| {
            Error::Failed(format!(
                "{what} {}: {e}. Is the directory writable? Nothing was changed.",
                tmp.display()
            ))
        };
        std::fs::write(&tmp, program).map_err(|e| fail("cannot write", &e))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))
                .map_err(|e| fail("cannot make executable", &e))?;
        }
        let said = std::process::Command::new(&tmp)
            .arg("--version")
            .output()
            .map_err(|e| fail("the new binary does not run:", &e))?;
        let said = String::from_utf8_lossy(&said.stdout);
        if said.trim() != format!("iohr {version}") {
            return Err(Error::Failed(format!(
                "the new binary says {:?}, not \"iohr {version}\"; nothing was installed",
                said.trim().chars().take(64).collect::<String>()
            )));
        }
        std::fs::rename(&tmp, dest).map_err(|e| fail("cannot replace the binary with", &e))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use std::path::{Path, PathBuf};

    use super::{
        Layout, Method, brew_failure, bundles_of, detect, newest, replace, sum_of, target,
    };

    fn layout() -> Layout {
        Layout {
            home: Some("/home/ann".into()),
            install_dir: None,
            brew_prefix: None,
        }
    }

    fn d(exe: &str, real: &str, l: &Layout) -> Method {
        detect(Path::new(exe), Path::new(real), l)
    }

    #[test]
    fn homebrew_by_prefix_cellar_or_env() {
        let l = layout();
        assert_eq!(
            d(
                "/opt/homebrew/bin/iohr",
                "/opt/homebrew/Cellar/iohr/0.1.0/bin/iohr",
                &l
            ),
            Method::Homebrew
        );
        // Intel Mac: /usr/local/bin is a link into the Cellar.
        assert_eq!(
            d(
                "/usr/local/bin/iohr",
                "/usr/local/Cellar/iohr/0.1.0/bin/iohr",
                &l
            ),
            Method::Homebrew
        );
        assert_eq!(
            d(
                "/home/linuxbrew/.linuxbrew/bin/iohr",
                "/home/linuxbrew/.linuxbrew/Cellar/iohr/1/bin/iohr",
                &l
            ),
            Method::Homebrew
        );
        let custom = Layout {
            brew_prefix: Some("/srv/brew".into()),
            ..layout()
        };
        assert_eq!(
            d("/srv/brew/bin/iohr", "/srv/brew/bin/iohr", &custom),
            Method::Homebrew
        );
        // A Cellar of another formula is not ours.
        assert_eq!(
            d(
                "/usr/local/bin/iohr",
                "/usr/local/Cellar/other/1/bin/iohr",
                &l
            ),
            Method::Other
        );
    }

    #[test]
    fn install_script_directory_default_and_override() {
        let l = layout();
        assert_eq!(
            d("/home/ann/.local/bin/iohr", "/home/ann/.local/bin/iohr", &l),
            Method::InstallScript {
                dir: PathBuf::from("/home/ann/.local/bin")
            }
        );
        let custom = Layout {
            install_dir: Some("/tmp/tools".into()),
            ..layout()
        };
        assert_eq!(
            d("/tmp/tools/iohr", "/tmp/tools/iohr", &custom),
            Method::InstallScript {
                dir: PathBuf::from("/tmp/tools")
            }
        );
        // With an override, the default directory no longer counts.
        assert_eq!(
            d(
                "/home/ann/.local/bin/iohr",
                "/home/ann/.local/bin/iohr",
                &custom
            ),
            Method::Other
        );
    }

    #[test]
    fn apt_and_the_rest() {
        let l = layout();
        assert_eq!(d("/usr/bin/iohr", "/usr/bin/iohr", &l), Method::Apt);
        assert_eq!(
            d("/work/target/debug/iohr", "/work/target/debug/iohr", &l),
            Method::Other
        );
        assert_eq!(
            d(
                "/home/ann/.local/bin/iohr",
                "/home/ann/.local/bin/iohr",
                &Layout::default()
            ),
            Method::Other
        );
    }

    #[test]
    fn newest_is_by_version_not_by_order_and_skips_drafts_and_other_packages() {
        let listing = serde_json::json!([
            {"tag_name": "go/v9.0.0"},
            {"tag_name": "iohr/v0.1.0-alpha.9"},
            {"tag_name": "iohr/v0.1.0-alpha.15"},
            {"tag_name": "iohr/v0.1.0-alpha.14"},
            {"tag_name": "iohr/v0.2.0", "draft": true},
            {"tag_name": "iohr/vnot-a-version"}
        ])
        .to_string();
        assert_eq!(
            newest(listing.as_bytes()).unwrap().to_string(),
            "0.1.0-alpha.15"
        );
        assert!(newest(b"[]").is_none());
        assert!(newest(b"not json").is_none());
    }

    #[test]
    fn targets_match_the_installer() {
        assert_eq!(target("macos", "aarch64").unwrap(), "aarch64-apple-darwin");
        assert_eq!(target("macos", "x86_64").unwrap(), "x86_64-apple-darwin");
        assert_eq!(
            target("linux", "aarch64").unwrap(),
            "aarch64-unknown-linux-musl"
        );
        assert_eq!(
            target("linux", "x86_64").unwrap(),
            "x86_64-unknown-linux-musl"
        );
        assert!(target("windows", "x86_64").is_none());
        assert!(target("linux", "riscv64").is_none());
    }

    #[test]
    fn checksums_are_read_like_the_installer_does() {
        let sums = "aa  iohr-1-x.tar.gz\nbb *iohr-1-y.tar.gz\n";
        assert_eq!(sum_of(sums, "iohr-1-x.tar.gz").unwrap(), "aa");
        assert_eq!(sum_of(sums, "iohr-1-y.tar.gz").unwrap(), "bb");
        assert!(sum_of(sums, "iohr-1-z.tar.gz").is_none());
    }

    #[test]
    fn the_untrusted_tap_error_says_brew_trust() {
        let e =
            "Error: Refusing to load formula inorbithr/tap/iohr from untrusted tap inorbithr/tap.";
        assert!(brew_failure(e).contains("brew trust inorbithr/tap"));
        assert!(!brew_failure("Error: no network").contains("brew trust"));
    }

    #[test]
    fn bundles_come_out_of_the_attestations_answer() {
        let a = serde_json::json!({"attestations": [{"bundle": {"a": 1}}, {"nobundle": 1}]});
        assert_eq!(bundles_of(a.to_string().as_bytes()).len(), 1);
        assert_eq!(bundles_of(b"{}"), Vec::<Vec<u8>>::new());
    }

    #[cfg(unix)]
    #[test]
    fn replace_swaps_atomically_and_refuses_a_binary_that_says_another_version() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("iohr");
        std::fs::write(&dest, "old").unwrap();
        let v = semver::Version::parse("1.2.3").unwrap();
        let wrong = b"#!/bin/sh\necho 'iohr 9.9.9'\n";
        assert!(replace(&dest, wrong, &v).is_err());
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "old");
        let names: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
        assert_eq!(names.len(), 1, "the temporary file was removed");
        let right = b"#!/bin/sh\necho 'iohr 1.2.3'\n";
        replace(&dest, right, &v).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), right);
        assert_eq!(
            std::fs::metadata(&dest).unwrap().permissions().mode() & 0o777,
            0o755
        );
    }
}
