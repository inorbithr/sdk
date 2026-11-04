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
/// semantic version, the highest by semver precedence (not by listing order), and
/// complete. Release-please publishes the release before the CLI job uploads its
/// files, so a release counts only once `SHA256SUMS` and, when `triple` is given,
/// `iohr-<version>-<triple>.tar.gz` are attached. Until then the previous release
/// stays the newest and `iohr upgrade` does nothing instead of failing.
pub(crate) fn newest(listing: &[u8], triple: Option<&str>) -> Option<semver::Version> {
    let items: Vec<Value> = serde_json::from_slice(listing).ok()?;
    items
        .iter()
        .filter(|r| r.get("draft").and_then(Value::as_bool) != Some(true))
        .filter_map(|r| {
            let v = r.get("tag_name")?.as_str()?.strip_prefix("iohr/v")?;
            let v = semver::Version::parse(v).ok()?;
            let names: Vec<&str> = r
                .get("assets")?
                .as_array()?
                .iter()
                .filter_map(|a| a.get("name")?.as_str())
                .collect();
            let has = |n: &str| names.contains(&n);
            let archive = triple.map(|t| format!("iohr-{v}-{t}.tar.gz"));
            (has("SHA256SUMS") && archive.as_deref().is_none_or(has)).then_some(v)
        })
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

/// Where the releases are looked up and downloaded from, with the HTTP client that reads
/// them: what `iohr upgrade` uses, and what the runner's own upgrade calls.
pub(crate) struct Upstream {
    client: reqwest::Client,
    src: Sources,
}

impl Upstream {
    /// The defaults, or the `IOHR_UPGRADE_*` overrides of [`Sources::from_process`].
    pub(crate) fn from_process() -> Result<Self, Error> {
        let src = Sources::from_process();
        Ok(Self {
            client: http(&src.api)?,
            src,
        })
    }

    /// The newest complete release for a copy installed by `method`.
    ///
    /// # Errors
    /// The listing cannot be read, or it holds no complete release.
    pub(crate) async fn newest(&self, method: &Method) -> Result<semver::Version, Error> {
        let listing = get(
            &self.client,
            &format!("{}/releases?per_page=100", self.src.api),
            MAX_LISTING,
        )
        .await?;
        // Homebrew installs from the tap, so only the install script needs this host's archive.
        let triple = match method {
            Method::InstallScript { .. } => target(std::env::consts::OS, std::env::consts::ARCH),
            _ => None,
        };
        newest(&listing, triple.as_deref())
            .ok_or_else(|| Error::Failed("no complete iohr release found on GitHub".into()))
    }

    /// Installs `version` the way `method` says: [`Upstream::prepare`], then
    /// [`Staged::apply`]. APT and unknown copies (`real` is where this one is) are
    /// refused: they need a person, or root.
    ///
    /// # Errors
    /// Whatever stopped the install; the old binary is untouched.
    pub(crate) async fn install(
        &self,
        method: &Method,
        real: &Path,
        version: &semver::Version,
        verbose: bool,
    ) -> Result<(), Error> {
        self.prepare(method, real, version, verbose)
            .await
            .map_err(Error::from)?
            .apply(version)
    }

    /// Everything about an install that can fail for reasons of the network, the release
    /// or the machine, and needs no change to it: the download, the checksum, the
    /// attestation and the program in the archive (install script), or `brew` and the
    /// formula's version (Homebrew). The runner does this before it stops taking work.
    ///
    /// # Errors
    /// What stopped it; nothing was changed.
    pub(crate) async fn prepare(
        &self,
        method: &Method,
        real: &Path,
        version: &semver::Version,
        verbose: bool,
    ) -> Result<Staged, Prepare> {
        match method {
            Method::Homebrew => {
                brew_ready(version)?;
                Ok(Staged::Brew)
            }
            Method::InstallScript { dir } => {
                let program = fetch_program(&self.client, &self.src, version, verbose).await?;
                Ok(Staged::Replace {
                    dest: dir.join("iohr"),
                    program,
                })
            }
            Method::Apt => Err(Error::Failed(format!(
                "iohr {version} is out. This copy came from the APT package; run: {}",
                command_for(method)
            ))
            .into()),
            Method::Other => Err(Error::Failed(format!(
                "iohr {version} is out, but this copy ({}) is not one iohr installed. Install it again \
                 with Homebrew (`brew install {TAP}/iohr`), the install script \
                 (`curl -fsSL https://packages.inorbit.hr/install.sh | sh`), APT or winget.",
                real.display()
            ))
            .into()),
        }
    }
}

/// An install that is ready: what is left changes the machine.
pub(crate) enum Staged {
    /// `brew upgrade` of the tap's formula.
    Brew,
    /// The verified program, to be renamed over `dest`.
    Replace { dest: PathBuf, program: Vec<u8> },
}

impl Staged {
    /// Does the change: `brew upgrade`, or the file replacement. Blocks.
    ///
    /// # Errors
    /// What stopped it; the old binary is untouched.
    pub(crate) fn apply(self, version: &semver::Version) -> Result<(), Error> {
        match self {
            Self::Brew => brew(version),
            Self::Replace { dest, program } => replace(&dest, &program, version),
        }
    }
}

/// Why an install cannot be made ready.
#[derive(Debug)]
pub(crate) enum Prepare {
    /// The Homebrew formula is older than the release the lookup found (the tap lags the
    /// GitHub release); `brew upgrade` would install this version instead.
    TapBehind {
        formula: semver::Version,
    },
    Failed(Error),
}

impl From<Error> for Prepare {
    fn from(e: Error) -> Self {
        Self::Failed(e)
    }
}

impl From<Prepare> for Error {
    fn from(p: Prepare) -> Self {
        match p {
            Prepare::TapBehind { formula } => Self::Failed(format!(
                "the Homebrew tap {TAP} still lists iohr {formula}, not the release on GitHub \
                 yet; nothing was installed"
            )),
            Prepare::Failed(e) => e,
        }
    }
}

/// This process's executable as run and by real path, and how it was installed. Linux
/// names a file replaced while it runs `<path> (deleted)`; that suffix is dropped.
pub(crate) fn detect_here() -> Result<(PathBuf, PathBuf, Method), Error> {
    let exe = std::env::current_exe()
        .map_err(|e| Error::Failed(format!("cannot tell where iohr is: {e}")))?;
    let exe = exe
        .to_str()
        .and_then(|p| p.strip_suffix(" (deleted)"))
        .map_or(exe.clone(), PathBuf::from);
    let real = exe.canonicalize().unwrap_or_else(|_| exe.clone());
    let method = detect(&exe, &real, &Layout::from_process());
    Ok((exe, real, method))
}

/// The version the binary at `path` says it is (`iohr --version`).
///
/// # Errors
/// The program does not run or does not say `iohr <version>`.
pub(crate) fn installed_version(path: &Path) -> Result<semver::Version, Error> {
    let said = std::process::Command::new(path)
        .arg("--version")
        .output()
        .map_err(|e| Error::Failed(format!("{} does not run: {e}", path.display())))?;
    let said = String::from_utf8_lossy(&said.stdout);
    said.trim()
        .strip_prefix("iohr ")
        .and_then(|v| semver::Version::parse(v).ok())
        .ok_or_else(|| Error::Failed(format!("{} does not say its version", path.display())))
}

pub(crate) async fn run(g: &Global, check: bool, out: Out) -> Result<(), Error> {
    let (_, real, method) = detect_here()?;
    let up = Upstream::from_process()?;

    let current = semver::Version::parse(env!("CARGO_PKG_VERSION"))
        .map_err(|e| Error::Failed(format!("this build's version: {e}")))?;
    let latest = up.newest(&method).await?;
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

    up.install(&method, &real, &latest, g.verbose).await?;
    if matches!(method, Method::InstallScript { .. }) {
        Out::note(&format!("iohr is now {latest} (was {current})."));
    }
    Ok(())
}

/// `brew upgrade`, and the trust message when Homebrew refuses the tap.
fn brew(latest: &semver::Version) -> Result<(), Error> {
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

/// `brew` runs from this process's `PATH` (a launchd job has a short one), and the tap's
/// formula is not older than `version`. A formula that looks old gets one `brew update`
/// first, which `brew upgrade` would run itself.
fn brew_ready(version: &semver::Version) -> Result<(), Prepare> {
    let formula = format!("{TAP}/iohr");
    let run = |args: &[&str]| {
        std::process::Command::new("brew")
            .args(args)
            .output()
            .map_err(|e| {
                Error::Failed(format!(
                    "cannot run brew ({e}): it is not on this process's PATH; run it yourself: \
                     brew upgrade {formula}"
                ))
            })
    };
    let listed = || -> Result<Option<semver::Version>, Error> {
        let o = run(&["info", "--json=v2", &formula])?;
        Ok(formula_version(&String::from_utf8_lossy(&o.stdout)))
    };
    let Some(first) = listed()? else {
        // Unreadable: leave the decision to `brew upgrade`.
        return Ok(());
    };
    if first >= *version {
        return Ok(());
    }
    run(&["update", "--quiet"])?;
    match listed()? {
        Some(now) if now < *version => Err(Prepare::TapBehind { formula: now }),
        _ => Ok(()),
    }
}

/// The stable version in `brew info --json=v2` output.
pub(crate) fn formula_version(json: &str) -> Option<semver::Version> {
    let v: Value = serde_json::from_str(json).ok()?;
    let stable = v
        .get("formulae")?
        .get(0)?
        .get("versions")?
        .get("stable")?
        .as_str()?;
    semver::Version::parse(stable).ok()
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

/// Downloads release `version`, verifies its checksum and attestation and returns the
/// program from the archive. Changes nothing on this machine.
async fn fetch_program(
    client: &reqwest::Client,
    src: &Sources,
    version: &semver::Version,
    verbose: bool,
) -> Result<Vec<u8>, Error> {
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

    layer::program(&archive, &format!("{name}/iohr"))
        .map_err(|e| Error::Failed(format!("{e}; nothing was installed")))
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

    use super::{Layout, Method, brew_failure, bundles_of, detect, newest, sum_of, target};

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
        let sums = serde_json::json!([{"name": "SHA256SUMS"}]);
        let listing = serde_json::json!([
            {"tag_name": "go/v9.0.0", "assets": sums},
            {"tag_name": "iohr/v0.1.0-alpha.9", "assets": sums},
            {"tag_name": "iohr/v0.1.0-alpha.15", "assets": sums},
            {"tag_name": "iohr/v0.1.0-alpha.14", "assets": sums},
            {"tag_name": "iohr/v0.2.0", "draft": true, "assets": sums},
            {"tag_name": "iohr/vnot-a-version", "assets": sums}
        ])
        .to_string();
        assert_eq!(
            newest(listing.as_bytes(), None).unwrap().to_string(),
            "0.1.0-alpha.15"
        );
        assert!(newest(b"[]", None).is_none());
        assert!(newest(b"not json", None).is_none());
    }

    #[test]
    fn newest_skips_a_release_whose_files_are_not_uploaded_yet() {
        // 2026-10-28: alpha.27 was published with no files while release-cli waited,
        // and every hourly upgrade on the runners failed on it.
        let t = "x86_64-unknown-linux-musl";
        let listing = serde_json::json!([
            {"tag_name": "iohr/v0.1.0-alpha.26", "assets": [
                {"name": "SHA256SUMS"}, {"name": format!("iohr-0.1.0-alpha.26-{t}.tar.gz")}
            ]},
            {"tag_name": "iohr/v0.1.0-alpha.27", "assets": []},
            {"tag_name": "iohr/v0.1.0-alpha.28", "assets": [{"name": "SHA256SUMS"}]},
            {"tag_name": "iohr/v0.1.0-alpha.29"}
        ])
        .to_string();
        let l = listing.as_bytes();
        // The install script needs this host's archive; alpha.28 has only the sums.
        assert_eq!(newest(l, Some(t)).unwrap().to_string(), "0.1.0-alpha.26");
        // Homebrew needs only a complete release.
        assert_eq!(newest(l, None).unwrap().to_string(), "0.1.0-alpha.28");
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

    #[test]
    fn the_formula_version_is_read_from_brew_info() {
        use super::{Error, Prepare, formula_version};
        let ok = r#"{"formulae":[{"name":"iohr","versions":{"stable":"0.1.0-alpha.27","head":null}}],"casks":[]}"#;
        assert_eq!(
            formula_version(ok),
            Some(semver::Version::parse("0.1.0-alpha.27").unwrap())
        );
        for bad in [
            "",
            "{}",
            r#"{"formulae":[]}"#,
            r#"{"formulae":[{"versions":{}}]}"#,
        ] {
            assert_eq!(formula_version(bad), None, "{bad}");
        }
        let lag = Prepare::TapBehind {
            formula: semver::Version::parse("0.1.0-alpha.27").unwrap(),
        };
        let said = Error::from(lag).to_string();
        assert!(said.contains("still lists iohr 0.1.0-alpha.27"), "{said}");
    }

    #[cfg(unix)]
    #[test]
    fn replace_swaps_atomically_and_refuses_a_binary_that_says_another_version() {
        use super::replace;
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
