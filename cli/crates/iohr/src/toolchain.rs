//! Is the language toolchain on this machine behind the latest stable release? (RFC 0127,
//! `iohr project init`.)
//!
//! The installed version is the one `iohr project init` already reads for the agent's prompt
//! ([`project::toolchain_line`], parsed by [`project::installed_version`]); the latest stable
//! comes from each language's own upstream, checked on 2026-11-06:
//!
//! | language | source | what is read |
//! |---|---|---|
//! | Rust | <https://static.rust-lang.org/dist/channel-rust-stable.toml> | `version` under `[pkg.rust]` (`1.99.1 (58cf8f9c3 2026-10-13)`) |
//! | Go | <https://go.dev/dl/?mode=json> | `version` of the first entry with `stable: true` (`go1.27.3`) |
//! | Node | <https://nodejs.org/dist/index.json> | `version` of the first entry whose `lts` is a name, not `false` (`v26.13.0`) |
//! | Python | <https://www.python.org/api/v2/downloads/release/> | the highest published, non-pre-release `Python 3.x.y` |
//!
//! Python has no small machine-readable "latest" file on python.org; its release API lists
//! every release (about 180 KB), so the highest stable 3.x.y is taken from it. A third-party
//! mirror such as endoflife.date is not used.
//!
//! # State machine (one language)
//!
//! ```text
//!   Idle --start--> Running --version command fails or tool absent--> Absent (no line)
//!                      | version line read
//!                      +-- no version in the line --> Unchecked
//!                      +-- lookup: HTTP error, bad body, offline or over BUDGET --> Unchecked
//!                      +-- installed < latest --> Behind
//!                      +-- installed == latest --> Current
//!                      +-- installed > latest --> Ahead
//! ```
//!
//! Invariants: the lookup never fails `init` (every error is [`State::Unchecked`], which says
//! "could not check" and shows no latest version); it is bounded by [`BUDGET`] in total and
//! runs on its own thread, started when `init` starts, so the picker rarely waits; it is off
//! under `--json` and when `IOHR_TOOLCHAIN_UPSTREAM=off`; an upgrade command is run only
//! after a yes at a terminal, never by default.
//!
//! The HTTP client is the one `iohr upgrade` uses ([`crate::commands::upgrade::http_within`]):
//! https only (a loopback address for tests), the rustls provider reqwest brings, so there
//! is no provider to install first.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use semver::Version;
use serde::Deserialize;

use crate::ask;
use crate::commands::upgrade::{get, http_within};
use crate::project::{installed_version, parse_lenient, toolchain_line};

/// The most time one language's lookup takes, the whole of it (connect, read, parse).
pub(crate) const BUDGET: Duration = Duration::from_secs(3);

/// `IOHR_TOOLCHAIN_UPSTREAM`: `off` turns the lookup off; any other value is a base URL that
/// stands in for the four sources (`{base}/rust`, `/go`, `/node`, `/python`), for tests and a
/// mirror.
pub(crate) const UPSTREAM_VAR: &str = "IOHR_TOOLCHAIN_UPSTREAM";

/// The languages iohr knows an upstream for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Lang {
    Rust,
    Go,
    Node,
    Python,
}

impl Lang {
    pub(crate) const ALL: [Self; 4] = [Self::Rust, Self::Go, Self::Node, Self::Python];

    /// The language a template variant's version command belongs to, by its program
    /// (`rustc`, `go`, `node`, `python3`); `None` for one iohr has no upstream for.
    pub(crate) fn of_toolchain(command: &[String]) -> Option<Self> {
        let prog = command.first()?;
        let name = Path::new(prog).file_stem()?.to_str()?;
        match name {
            "rustc" => Some(Self::Rust),
            "go" => Some(Self::Go),
            "node" => Some(Self::Node),
            "python" | "python3" => Some(Self::Python),
            _ => None,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Rust => "Rust",
            Self::Go => "Go",
            Self::Node => "Node",
            Self::Python => "Python",
        }
    }

    /// What the hint calls the installed tool.
    pub(crate) fn tool(self) -> &'static str {
        match self {
            Self::Rust => "rustc",
            Self::Go => "go",
            Self::Node => "node",
            Self::Python => "python",
        }
    }

    /// The command that prints the installed version, and so the program to find on PATH.
    fn version_command(self) -> Vec<String> {
        let words: &[&str] = match self {
            Self::Rust => &["rustc", "--version"],
            Self::Go => &["go", "version"],
            Self::Node => &["node", "--version"],
            Self::Python => &["python3", "--version"],
        };
        words.iter().map(|w| (*w).to_owned()).collect()
    }

    fn program(self) -> &'static str {
        match self {
            Self::Rust => "rustc",
            Self::Go => "go",
            Self::Node => "node",
            Self::Python => "python3",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::Go => "go",
            Self::Node => "node",
            Self::Python => "python",
        }
    }

    fn source(self) -> &'static str {
        match self {
            Self::Rust => "https://static.rust-lang.org/dist/channel-rust-stable.toml",
            Self::Go => "https://go.dev/dl/?mode=json",
            Self::Node => "https://nodejs.org/dist/index.json",
            Self::Python => "https://www.python.org/api/v2/downloads/release/",
        }
    }

    /// The largest body read: the Rust manifest is about 1 MB, the others are smaller.
    fn max_bytes(self) -> usize {
        match self {
            Self::Rust => 8 << 20,
            _ => 4 << 20,
        }
    }

    /// Where to get the language by hand.
    fn download(self) -> &'static str {
        match self {
            Self::Rust => "https://rust-lang.org/tools/install",
            Self::Go => "https://go.dev/dl/",
            Self::Node => "https://nodejs.org/en/download",
            Self::Python => "https://www.python.org/downloads/",
        }
    }
}

// ---- the upstream formats -------------------------------------------------------------

/// `[pkg.rust]` `version = "1.99.1 (58cf8f9c3 2026-10-13)"` of the stable channel manifest.
pub(crate) fn parse_rust(body: &str) -> Option<Version> {
    let mut in_rust = false;
    for line in body.lines().map(str::trim) {
        if line.starts_with('[') {
            if in_rust {
                // The section ended without a version.
                return None;
            }
            in_rust = line == "[pkg.rust]";
        } else if in_rust
            && let Some(rest) = line.strip_prefix("version")
            && let Some(value) = rest.trim_start().strip_prefix('=')
        {
            let quoted = value.trim().strip_prefix('"')?;
            return parse_lenient(quoted.split(['"', ' ']).next()?);
        }
    }
    None
}

#[derive(Deserialize)]
struct GoRelease {
    version: String,
    #[serde(default)]
    stable: bool,
}

/// The first `stable: true` entry of `?mode=json` (newest first): `go1.27.3`.
pub(crate) fn parse_go(body: &str) -> Option<Version> {
    let list: Vec<GoRelease> = serde_json::from_str(body).ok()?;
    let first = list.into_iter().find(|r| r.stable)?;
    parse_lenient(first.version.strip_prefix("go")?)
}

#[derive(Deserialize)]
struct NodeRelease {
    version: String,
    /// `false`, or the name of the LTS line (`"Krypton"`).
    #[serde(default)]
    lts: serde_json::Value,
}

/// The first entry of `index.json` (newest first) whose `lts` is a name: `v26.13.0`.
pub(crate) fn parse_node(body: &str) -> Option<Version> {
    let list: Vec<NodeRelease> = serde_json::from_str(body).ok()?;
    let first = list.into_iter().find(|r| r.lts.is_string())?;
    parse_lenient(first.version.strip_prefix('v')?)
}

#[derive(Deserialize)]
struct PyRelease {
    name: String,
    #[serde(default)]
    is_published: bool,
    #[serde(default)]
    pre_release: bool,
}

/// The highest published, non-pre-release `Python 3.x.y` of python.org's release list.
pub(crate) fn parse_python(body: &str) -> Option<Version> {
    let list: Vec<PyRelease> = serde_json::from_str(body).ok()?;
    list.iter()
        .filter(|r| r.is_published && !r.pre_release)
        .filter_map(|r| {
            let v = r.name.strip_prefix("Python 3.")?;
            // `3.15.2` strictly: digits and two dots, so `3.15.0rc3` and the install
            // manager's own releases never count.
            if v.split('.').count() != 2 || !v.chars().all(|c| c.is_ascii_digit() || c == '.') {
                return None;
            }
            parse_lenient(&format!("3.{v}"))
        })
        .max()
}

fn parse(lang: Lang, body: &str) -> Option<Version> {
    match lang {
        Lang::Rust => parse_rust(body),
        Lang::Go => parse_go(body),
        Lang::Node => parse_node(body),
        Lang::Python => parse_python(body),
    }
}

// ---- the lookup -----------------------------------------------------------------------

/// Where the latest versions are read from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Upstream {
    base: Option<String>,
}

impl Upstream {
    pub(crate) fn official() -> Self {
        Self { base: None }
    }

    pub(crate) fn at(base: &str) -> Self {
        Self {
            base: Some(base.trim_end_matches('/').to_owned()),
        }
    }

    /// `None` when `IOHR_TOOLCHAIN_UPSTREAM=off`.
    pub(crate) fn from_process() -> Option<Self> {
        match std::env::var(UPSTREAM_VAR).ok().as_deref().map(str::trim) {
            Some("off") => None,
            Some(b) if !b.is_empty() => Some(Self::at(b)),
            _ => Some(Self::official()),
        }
    }

    fn url(&self, lang: Lang) -> String {
        match &self.base {
            Some(b) => format!("{b}/{}", lang.key()),
            None => lang.source().to_owned(),
        }
    }

    /// The latest stable of `lang`. The error is one short line, without the URL.
    ///
    /// # Errors
    /// The network, a status other than 200, a body over the limit or one that does not
    /// parse.
    pub(crate) async fn latest(&self, lang: Lang) -> Result<Version, String> {
        let url = self.url(lang);
        let client = http_within(&url, BUDGET, None, BUDGET).map_err(|_| "no HTTP client")?;
        let body = get(&client, &url, lang.max_bytes())
            .await
            .map_err(|_| format!("{} did not answer", lang.label()))?;
        let text = String::from_utf8_lossy(&body);
        parse(lang, &text).ok_or_else(|| format!("{}'s answer had no version", lang.label()))
    }
}

/// How the installed toolchain stands against the latest stable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum State {
    Behind {
        installed: Version,
        latest: Version,
    },
    Current {
        installed: Version,
        latest: Version,
    },
    Ahead {
        installed: Version,
        latest: Version,
    },
    /// The lookup or the installed version could not be read: no latest is shown.
    Unchecked {
        installed: Option<Version>,
    },
}

/// Compares what is installed with the latest stable; a failed lookup is [`State::Unchecked`].
pub(crate) fn judge(installed: Option<Version>, latest: Result<Version, String>) -> State {
    match (installed, latest) {
        (Some(i), Ok(l)) => match i.cmp(&l) {
            std::cmp::Ordering::Less => State::Behind {
                installed: i,
                latest: l,
            },
            std::cmp::Ordering::Equal => State::Current {
                installed: i,
                latest: l,
            },
            std::cmp::Ordering::Greater => State::Ahead {
                installed: i,
                latest: l,
            },
        },
        (installed, _) => State::Unchecked { installed },
    }
}

impl State {
    /// The picker's hint after what the menu already says: `rustc 1.90.0, 1.93.0 is out`.
    pub(crate) fn hint(&self, lang: Lang) -> String {
        let t = lang.tool();
        match self {
            Self::Behind { installed, latest } => format!("{t} {installed}, {latest} is out"),
            Self::Current { installed, .. } => format!("{t} {installed}, up to date"),
            Self::Ahead { installed, latest } => {
                format!("{t} {installed}, newer than stable {latest}")
            }
            Self::Unchecked { installed: Some(i) } => {
                format!("{t} {i}, could not check the latest")
            }
            Self::Unchecked { installed: None } => "could not check the latest".to_owned(),
        }
    }

    /// The sentence for the report and the plan.
    pub(crate) fn said(&self, lang: Lang) -> String {
        let l = lang.label();
        match self {
            Self::Behind { installed, latest } => {
                format!("{l} {installed} is behind the latest stable, {latest}")
            }
            Self::Current { installed, latest } => {
                format!("{l} {installed} is the latest stable ({latest})")
            }
            Self::Ahead { installed, latest } => {
                format!("{l} {installed} is newer than the latest stable ({latest})")
            }
            Self::Unchecked { installed } => format!(
                "could not check whether {l}{} is the latest stable",
                installed
                    .as_ref()
                    .map(|i| format!(" {i}"))
                    .unwrap_or_default()
            ),
        }
    }
}

/// Where a language's lookup is.
enum Phase {
    Running,
    /// The tool is not installed.
    Absent,
    Found(State),
}

/// One language's slot: running, then done.
struct Slot {
    done: Mutex<Phase>,
    cv: Condvar,
    started: Instant,
}

/// The lookups of one `init`, one thread per language, each started once.
pub(crate) struct Lookup {
    up: Upstream,
    slots: Mutex<HashMap<Lang, Arc<Slot>>>,
}

static LOOKUP: OnceLock<Lookup> = OnceLock::new();

/// Starts the lookups for `langs` (the installed ones answer; the others have no line).
/// Without a call, or with `IOHR_TOOLCHAIN_UPSTREAM=off`, [`state`] answers `None`.
pub(crate) fn start(langs: &[Lang]) {
    let Some(up) = Upstream::from_process() else {
        return;
    };
    let lookup = LOOKUP.get_or_init(|| Lookup {
        up,
        slots: Mutex::new(HashMap::new()),
    });
    for l in langs {
        lookup.begin(*l);
    }
}

/// The state of `lang`'s toolchain, waiting for its lookup (at most [`BUDGET`] after it began).
/// `None`: the lookup is off, or the tool is not installed.
pub(crate) fn state(lang: Lang) -> Option<State> {
    LOOKUP.get()?.begin(lang).wait()
}

impl Lookup {
    fn begin(&self, lang: Lang) -> Arc<Slot> {
        let mut slots = self
            .slots
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(s) = slots.get(&lang) {
            return Arc::clone(s);
        }
        let slot = Arc::new(Slot {
            done: Mutex::new(Phase::Running),
            cv: Condvar::new(),
            started: Instant::now(),
        });
        slots.insert(lang, Arc::clone(&slot));
        let (up, put) = (self.up.clone(), Arc::clone(&slot));
        let spawned = std::thread::Builder::new()
            .name(format!("toolchain-{}", lang.key()))
            .spawn(move || {
                let found = probe(lang, &up);
                *put.done
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) =
                    found.map_or(Phase::Absent, Phase::Found);
                put.cv.notify_all();
            });
        if spawned.is_err() {
            *slot
                .done
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) =
                Phase::Found(State::Unchecked { installed: None });
        }
        slot
    }
}

impl Slot {
    fn wait(&self) -> Option<State> {
        // A little over the lookup's own bound, so a thread that ends on its budget is read.
        let limit = BUDGET + Duration::from_millis(500);
        let left = limit.saturating_sub(self.started.elapsed());
        let guard = self
            .done
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (guard, _) = self
            .cv
            .wait_timeout_while(guard, left, |d| matches!(d, Phase::Running))
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match &*guard {
            Phase::Found(found) => Some(found.clone()),
            Phase::Absent => None,
            Phase::Running => Some(State::Unchecked { installed: None }),
        }
    }
}

/// Reads the installed version, then the latest stable, within [`BUDGET`].
fn probe(lang: Lang, up: &Upstream) -> Option<State> {
    let line = toolchain_line(&lang.version_command())?;
    Some(probe_line(lang, &line, up))
}

/// [`probe`] for a version line already read.
pub(crate) fn probe_line(lang: Lang, line: &str, up: &Upstream) -> State {
    let installed = installed_version(line);
    if installed.is_none() {
        return State::Unchecked { installed };
    }
    let latest = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| "no runtime".to_owned())
        .and_then(|rt| {
            rt.block_on(async {
                tokio::time::timeout(BUDGET, up.latest(lang))
                    .await
                    .unwrap_or_else(|_| Err(format!("{} took too long", lang.label())))
            })
        });
    judge(installed, latest)
}

// ---- the upgrade ----------------------------------------------------------------------

/// How to bring a toolchain to the latest stable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Upgrade {
    /// What the person sees and may run.
    pub(crate) shown: String,
    /// The program and arguments iohr runs after a yes; `None` when it is for the person
    /// (a download, or a manager that is a shell function).
    pub(crate) run: Option<Vec<String>>,
}

impl Upgrade {
    fn run(words: &[&str]) -> Self {
        Self {
            shown: words.join(" "),
            run: Some(words.iter().map(|w| (*w).to_owned()).collect()),
        }
    }

    fn shown(text: impl Into<String>) -> Self {
        Self {
            shown: text.into(),
            run: None,
        }
    }
}

/// The upgrade command for the toolchain at `binary`, by what installed it: Homebrew
/// (`brew upgrade go`), rustup (`rustup update stable`), mise (`mise upgrade node`), fnm; asdf,
/// pyenv and nvm are shown, not run (they need a version, a pin or a shell function); else the
/// download page. `has` says whether a program is on PATH.
pub(crate) fn upgrade_for(
    lang: Lang,
    binary: Option<&Path>,
    latest: &Version,
    has: impl Fn(&str) -> bool,
) -> Upgrade {
    let raw = binary.map(|b| b.to_string_lossy().replace('\\', "/").to_lowercase());
    // A shim points at the manager, so the resolved path is read too, but only to find
    // Homebrew (its Cellar), never to change what the path as found says.
    let real = binary
        .and_then(|b| std::fs::canonicalize(b).ok())
        .map(|b| b.to_string_lossy().replace('\\', "/").to_lowercase());
    let any = |needles: &[&str]| {
        [raw.as_deref(), real.as_deref()]
            .into_iter()
            .flatten()
            .any(|p| needles.iter().any(|n| p.contains(n)))
    };
    if any(&["/cellar/", "/homebrew/", "/linuxbrew/"]) && has("brew") {
        let formula = match lang {
            Lang::Rust => "rust",
            Lang::Go => "go",
            Lang::Node => "node",
            Lang::Python => "python",
        };
        return Upgrade::run(&["brew", "upgrade", formula]);
    }
    if lang == Lang::Rust && has("rustup") {
        return Upgrade::run(&["rustup", "update", "stable"]);
    }
    if any(&["/mise/"]) && has("mise") {
        return Upgrade::run(&["mise", "upgrade", lang.tool()]);
    }
    if any(&["/.asdf/"]) {
        let plugin = match lang {
            Lang::Rust => "rust",
            Lang::Go => "golang",
            Lang::Node => "nodejs",
            Lang::Python => "python",
        };
        return Upgrade::shown(format!("asdf install {plugin} latest"));
    }
    if lang == Lang::Python && any(&["/.pyenv/"]) {
        return Upgrade::shown(format!("pyenv install {latest}"));
    }
    if lang == Lang::Node && any(&["/.nvm/"]) {
        return Upgrade::shown("nvm install --lts");
    }
    if lang == Lang::Node && any(&["/fnm"]) && has("fnm") {
        return Upgrade::run(&["fnm", "install", "--lts"]);
    }
    Upgrade::shown(format!("download {latest} from {}", lang.download()))
}

/// The upgrade for `lang` as this machine has it installed.
pub(crate) fn upgrade_here(lang: Lang, latest: &Version) -> Upgrade {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let binary = ask::find_on_path(lang.program(), &path);
    upgrade_for(lang, binary.as_deref(), latest, |p| {
        ask::find_on_path(p, &path).is_some()
    })
}

/// Runs an upgrade that has a command, with the terminal as its own (the manager's progress
/// shows). `Err` is one line.
///
/// # Errors
/// The command did not start, or exited with a failure.
pub(crate) fn run_upgrade(up: &Upgrade) -> Result<(), String> {
    let Some((prog, args)) = up.run.as_ref().and_then(|w| w.split_first()) else {
        return Err("this upgrade is for you to run".to_owned());
    };
    match std::process::Command::new(prog).args(args).status() {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(format!("`{}` exited with {s}", up.shown)),
        Err(e) => Err(format!("`{}` did not start: {e}", up.shown)),
    }
}

#[cfg(test)]
mod tests;
