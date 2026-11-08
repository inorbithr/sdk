use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::str::FromStr;

use etcetera::app_strategy::{AppStrategy as _, AppStrategyArgs, choose_native_strategy};
use serde::{Deserialize, Serialize};

const FILE: &str = "config.toml";

/// The profiles on this machine and which one is the default.
///
/// Stored as `config.toml` in the platform's config directory (XDG on Linux,
/// Application Support on macOS, `%APPDATA%` on Windows). It holds names, account ids
/// and settings, never a secret (SR-24).
///
/// The file is shared with the SDKs (`docs/config.md` section 4): they read the `[sdk]`
/// table and their own keys in `[profiles.<name>]`, and a profile may hold SDK keys only
/// (one written by hand, without `kind`). The command line owns `default`, the keys of
/// [`Profile`] and the `[ext]` table; [`save`](Self::save) changes only those and keeps
/// every other key, table and comment as it was read. The `[runner]` table (and a
/// profile's `[profiles.<name>.runner]`) is read, never written: a person edits it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Config {
    /// The profile used when no `--profile` or `IOHR_PROFILE` is given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<ProfileName>,
    /// Every profile the command line made (one with a `kind`), by name. Profiles that
    /// hold only SDK settings stay in the file and are not listed here.
    #[serde(default, deserialize_with = "command_line_profiles")]
    pub profiles: BTreeMap<ProfileName, Profile>,
    /// Where extensions come from and which extra keys may sign them.
    #[serde(default, skip_serializing_if = "ExtConfig::is_empty")]
    pub ext: ExtConfig,
    /// `iohr runner` settings (`docs/config.md` section 4.4); a profile's own
    /// `[profiles.<name>.runner]` overrides them key by key.
    #[serde(default, skip_serializing_if = "RunnerConfig::is_empty")]
    pub runner: RunnerConfig,
    /// `iohr project init` defaults (`[project]`); read, never written: a person edits it.
    #[serde(default, skip_serializing_if = "ProjectConfig::is_empty")]
    pub project: ProjectConfig,
    /// `iohr ask` defaults (`[ask]`); read, never written: a person edits it.
    #[serde(default, skip_serializing_if = "AskConfig::is_empty")]
    pub ask: AskConfig,
    /// `iohr sync` settings (`[sync]`); read, never written: a person edits it.
    #[serde(default, skip_serializing_if = "SyncConfig::is_empty")]
    pub sync: SyncConfig,
    /// `iohr memory` settings (`[memory]`); read, never written: a person edits it.
    #[serde(default, skip_serializing_if = "MemoryConfig::is_empty")]
    pub memory: MemoryConfig,
    /// `iohr` panel settings (`[panel]`); read, never written: a person edits it.
    #[serde(default, skip_serializing_if = "PanelConfig::is_empty")]
    pub panel: PanelConfig,
    /// The file as it was read, so a save keeps what the command line does not own.
    #[serde(skip)]
    source: Source,
}

/// The parsed file a [`Config`] came from. It takes no part in equality: two configs
/// with the same settings are equal whatever their comments.
#[derive(Clone, Default)]
struct Source(Option<toml_edit::DocumentMut>);

impl fmt::Debug for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(if self.0.is_some() {
            "Source(file)"
        } else {
            "Source(new)"
        })
    }
}

impl PartialEq for Source {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl Eq for Source {}

/// The keys of a profile table the command line owns; every other key is an SDK
/// setting or a newer command line's, and is kept.
const PROFILE_KEYS: [&str; 6] = [
    "kind",
    "account",
    "account_pinned",
    "storage",
    "issuer",
    "client_id",
];

/// Reads `[profiles]`, keeping only the tables the command line made: those with a
/// `kind`. A table without one holds SDK settings only (`docs/config.md` section 4.2).
fn command_line_profiles<'de, D>(d: D) -> Result<BTreeMap<ProfileName, Profile>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error as _;
    let tables = BTreeMap::<ProfileName, toml::Table>::deserialize(d)?;
    tables
        .into_iter()
        .filter(|(_, t)| t.contains_key("kind"))
        .map(|(name, t)| {
            let p = Profile::deserialize(toml::Value::Table(t))
                .map_err(|e| D::Error::custom(format!("profiles.{name}: {}", e.message())))?;
            Ok((name, p))
        })
        .collect()
}

/// The `[ext]` table: extension settings (`iohr config set ext.<key>`). Public keys
/// and a registry address; never a secret.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ExtConfig {
    /// The registry and path prefix extensions are pulled from, such as a company's
    /// mirror (`registry.acme.hr/inorbit/iohr-ext`). Unset: InOrbit's own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry: Option<String>,
    /// PEM public keys trusted to sign extensions besides InOrbit's release workflow:
    /// a mirror that re-signs, or an internal channel.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trusted_keys: Vec<String>,
}

impl ExtConfig {
    /// Whether nothing is set.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.registry.is_none() && self.trusted_keys.is_empty()
    }
}

/// The `[project]` table: defaults for `iohr project init`. A `--template` flag wins over it;
/// the built-in default (`coordinated`) applies when it is unset. Checked where it is used.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ProjectConfig {
    /// The template a new project is made from when `--template` is not given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template: Option<String>,
    /// A folder your projects live in (`~` is your home). When set, `iohr project init` offers
    /// `ROOT/NAME` as a place for the new project; unset, there is no projects folder and the
    /// default is `./NAME` in the folder you are in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
    /// The model the agent of `iohr project init` builds with (`--agent-model`, or
    /// `IOHR_INIT_AGENT_MODEL`, wins over it). Unset: `sonnet` for Claude.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_model: Option<String>,
    /// The most turns that agent may take (`--agent-turns` wins over it). Unset: 150.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_turns: Option<u32>,
    /// The most seconds that agent may run (`--agent-timeout` wins over it). Unset: 1800.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_timeout: Option<u64>,
}

impl ProjectConfig {
    /// Whether nothing is set.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.template.is_none()
            && self.root.is_none()
            && self.agent_model.is_none()
            && self.agent_turns.is_none()
            && self.agent_timeout.is_none()
    }
}

/// The `[ask]` table: how `iohr ask` answers. `--via` and `--agent` win over it; `local` and
/// `claude` apply when a key is unset. Checked where it is used.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct AskConfig {
    /// Where the answer comes from: `local` (the developer's own agent; the default). `io` is
    /// reserved and not built yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via: Option<String>,
    /// With `via = "local"`, which agent to start: `claude` (the default) or `codex`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    /// The model the agent answers with: a Claude alias such as `haiku` (the default, the
    /// small fast tier) or a full model name. `--model` (or `IOHR_ASK_MODEL`) wins over it. For `codex` it is passed
    /// as `--model` only when set; Codex otherwise uses its own configured model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Let the agent edit files in this folder (Claude `--permission-mode acceptEdits`, Codex
    /// `--sandbox workspace-write`). Off by default; `--allow-edits` turns it on for one call.
    /// Shell commands are never allowed by iohr.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_edits: Option<bool>,
    /// How the thinking is drawn at a terminal: `show` (streamed, dim, in a quiet block; the
    /// default) or `collapse` (one line). `--thinking` wins over it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<String>,
    /// How often the footer of the `?` card is the full notice instead of one short hint:
    /// `sometimes` (the default: the first card, when the notice changes and about once a
    /// day), `always` or `never` (the first card and a changed notice still show it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hints: Option<String>,
    /// The key that opens the actions menu under a `?` card (RFC 0203.1): `down` (the
    /// default, on an empty line), `ctrl-g`, `alt-enter`, or `off`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub menu_key: Option<String>,
    /// What happens when Io wants to run a command or open a file no rule decides (PRD
    /// 0197): `manual` (or `prompt`: ask at the terminal; the default at one), `plan`, `auto`
    /// (the modes of `mode`), `rules` (follow the rules only; the default without a terminal)
    /// or `deny` (no commands offered). `--permissions` wins over it, and `mode` beats it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permissions: Option<String>,
    /// How Io works (PRD 0197): `manual` (ask before each command; the default at a terminal),
    /// `plan` (read and propose a plan, change nothing) or `auto` (run commands no rule
    /// decides without asking; deny rules and the secret list still win). `--mode` (or
    /// `IOHR_ASK_MODE`) wins over it, and it beats `permissions`. The zsh hook starts its
    /// shift+tab switch from it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    /// Folders `iohr ask` may always read besides the one it is asked in, the way Claude Code's
    /// `--add-dir` adds them: a leading `~/` is your home folder. Each must exist, be listable
    /// and hold no secrets, or the question is refused; secret files inside stay blocked.
    /// `--read` (alias `--add-dir`) and `IOHR_ASK_READ` (an OS path list) add to it, never
    /// replace it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read: Option<Vec<String>>,
}

impl AskConfig {
    /// Whether nothing is set.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.via.is_none()
            && self.agent.is_none()
            && self.model.is_none()
            && self.allow_edits.is_none()
            && self.thinking.is_none()
            && self.hints.is_none()
            && self.menu_key.is_none()
            && self.mode.is_none()
            && self.permissions.is_none()
            && self.read.is_none()
    }
}

/// The `[sync]` table: whether the project's `atlas/` documents may be pushed to Decisions
/// without the person running `iohr sync` (RFC 0205). Nothing leaves the machine otherwise.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SyncConfig {
    /// Push in the background. Off when unset: only `iohr sync` sends anything.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<bool>,
}

impl SyncConfig {
    /// Whether nothing is set.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.background.is_none()
    }

    /// Whether background sync is on (`background = true`).
    #[must_use]
    pub fn background_on(&self) -> bool {
        self.background == Some(true)
    }
}

/// The `[memory]` table: where `iohr memory` keeps entries by default, and whether it may
/// talk to the account's memory service. Nothing leaves the machine unless `sync = true` or
/// the person runs `iohr memory sync` (RFC 0205, ADR 0216).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct MemoryConfig {
    /// The default scope: `project` (the repository's `.inorbit/memory/`, committed), `user`
    /// (this person's notes on this machine) or `account` (the memory service, needs sync).
    /// `project` when unset; `--scope` overrides it per command.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    /// Sync with the account's memory service. Off when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync: Option<bool>,
    /// Days after which a memory's `checked:` date is too old for `iohr memory check`.
    /// 180 when unset; `--checked-days` overrides it per run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check_days: Option<u32>,
}

impl MemoryConfig {
    /// Whether nothing is set.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.scope.is_none() && self.sync.is_none() && self.check_days.is_none()
    }

    /// Whether sync is on (`sync = true`).
    #[must_use]
    pub fn sync_on(&self) -> bool {
        self.sync == Some(true)
    }
}

/// The `[panel]` table: the floating panel bare `iohr` puts over the top right corner of the
/// terminal (`docs/config.md` section 4.8).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct PanelConfig {
    /// When the panel reads the mouse: `menu` (only while the menu is open, so the terminal
    /// keeps selecting text with a drag; the default), `always` (a click on the pill opens
    /// the menu; the terminal selects text with Option or Shift held) or `off` (keys only).
    /// `IOHR_PANEL_MOUSE` overrides it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mouse: Option<String>,
}

impl PanelConfig {
    /// Whether nothing is set.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.mouse.is_none()
    }
}

/// The `[runner]` table: how `iohr runner` registers and runs on this machine. Read at
/// start and again on SIGHUP by `iohr runner run`; command-line flags win over it. Every
/// key is optional; an unset key falls back to the profile's table, then to the default.
/// Values are checked where they are used, so a bad one never stops other commands.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RunnerConfig {
    /// Turns held at once, 1 to 32; sent on every heartbeat.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capacity: Option<u32>,
    /// The turn classes this runner takes: `coordinate`, `build`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kinds: Option<Vec<String>>,
    /// Registered as always on (a server).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub always_on: Option<bool>,
    /// The runner's name; default the host name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The runner's id (`rnr_...`): pins which runner `run` and `gate-report` drive when this
    /// machine's key matches more than one. The `--runner` flag wins over it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// The kubectl context approved host actions run in (`kubectl --context`). With no
    /// context and no `kubeconfig`, the runner refuses every host action.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kube_context: Option<String>,
    /// The kubeconfig file approved host actions run with (`kubectl --kubeconfig`), an
    /// absolute path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kubeconfig: Option<String>,
    /// Whether `iohr runner run` upgrades itself to the newest complete release and
    /// restarts (default true). `false` leaves upgrades to a person.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_upgrade: Option<bool>,
    /// Minutes between looks for a newer release (5 to 10080; default 60, plus up to 10
    /// percent of jitter).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upgrade_interval_minutes: Option<u32>,
    /// How an upgraded runner restarts: `exec` (default; the same process becomes the new
    /// binary, so launchd or systemd see no stop) or `exit` (under launchd or systemd, exit
    /// with code 75 and let the supervisor start it; elsewhere it is `exec`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upgrade_restart: Option<String>,
    /// On macOS, the identity an upgrade signs the new binary with when the release is
    /// signed ad hoc, so the keychain's "Always Allow" holds from one release to the next:
    /// `auto` (the first Developer ID Application identity, else the first Apple
    /// Development one), a name or SHA-1 hash `codesign` takes, or `-` (off, the default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upgrade_sign_identity: Option<String>,
    /// The only user service units (`<name>.service`) an approved `systemd.user-restart`
    /// may restart. Unset or empty, the runner refuses every one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_units: Option<Vec<String>>,
    /// The core checkout (absolute path) an approved `build.sweep-targets` runs core's
    /// `target:sweep` or `target:guard` in. Unset, the runner refuses every sweep.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub core_checkout: Option<String>,
}

impl RunnerConfig {
    /// Whether nothing is set.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.capacity.is_none()
            && self.kinds.is_none()
            && self.always_on.is_none()
            && self.name.is_none()
            && self.id.is_none()
            && self.kube_context.is_none()
            && self.kubeconfig.is_none()
            && self.auto_upgrade.is_none()
            && self.upgrade_interval_minutes.is_none()
            && self.upgrade_restart.is_none()
            && self.upgrade_sign_identity.is_none()
            && self.user_units.is_none()
            && self.core_checkout.is_none()
    }
}

/// One way of calling the API.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Profile {
    /// What kind of credential the profile holds.
    pub kind: Kind,
    /// The account the calls count against.
    pub account: String,
    /// Whether a person chose `account` (`iohr login --account`, `iohr profile account`):
    /// a sign-in again keeps a chosen account. When not, it follows the account the person
    /// made their default on the server, else their own.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub account_pinned: bool,
    /// Where the profile's secret is kept.
    #[serde(default)]
    pub storage: Storage,
    /// For a person: the sign-in service the session refreshes with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issuer: Option<String>,
    /// For a person: the OAuth client the session belongs to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    /// This profile's `[profiles.<name>.runner]` table, over the file's `[runner]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runner: Option<RunnerConfig>,
}

impl Profile {
    /// A profile for `account` holding a credential of `kind` in `storage`.
    #[must_use]
    pub fn new(kind: Kind, account: impl Into<String>, storage: Storage) -> Self {
        Self {
            kind,
            account: account.into(),
            account_pinned: false,
            storage,
            issuer: None,
            client_id: None,
            runner: None,
        }
    }

    /// A signed-in person's profile, which refreshes with `issuer` as `client_id`.
    #[must_use]
    pub fn person(
        account: impl Into<String>,
        storage: Storage,
        issuer: impl Into<String>,
        client_id: impl Into<String>,
    ) -> Self {
        Self {
            kind: Kind::Person,
            account: account.into(),
            account_pinned: false,
            storage,
            issuer: Some(issuer.into()),
            client_id: Some(client_id.into()),
            runner: None,
        }
    }
}

/// What kind of credential a profile holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum Kind {
    /// An API token made in the console or with `iohr token create`.
    Token,
    /// A person signed in with `iohr login`.
    Person,
}

/// Where a profile's secret is kept.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum Storage {
    /// The operating system's credential store.
    #[default]
    Keyring,
    /// A file readable only by its owner, chosen with `--insecure-storage`.
    File,
}

/// A profile's name: 1 to 64 characters of lower-case letters, digits, `-` and `_`,
/// starting with a letter or digit.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ProfileName(String);

/// Why a string is not a profile name.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "a profile name has 1 to 64 lower-case letters, digits, '-' or '_', and starts with a letter or digit"
)]
pub struct ProfileNameError;

impl ProfileName {
    /// The name as a string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for ProfileName {
    type Err = ProfileNameError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let ok = (1..=64).contains(&s.len())
            && s.bytes()
                .next()
                .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
            && s.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_');
        if ok {
            Ok(Self(s.to_owned()))
        } else {
            Err(ProfileNameError)
        }
    }
}

impl TryFrom<String> for ProfileName {
    type Error = ProfileNameError;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

impl From<ProfileName> for String {
    fn from(n: ProfileName) -> Self {
        n.0
    }
}

impl fmt::Display for ProfileName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Why the config could not be read or written.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ConfigError {
    /// The home directory could not be found.
    #[error("cannot find the home directory to keep the config in; set IOHR_CONFIG_DIR")]
    NoHome,
    /// Reading or writing the file failed.
    #[error("cannot {action} {path}: {source}")]
    Io {
        /// What was being done.
        action: &'static str,
        /// The file.
        path: PathBuf,
        /// The underlying error.
        source: io::Error,
    },
    /// The file is not a valid config.
    #[error("{path} is not a valid iohr config: {message}")]
    Invalid {
        /// The file.
        path: PathBuf,
        /// What is wrong, from the TOML parser.
        message: String,
    },
}

impl Config {
    /// The platform's config directory for `iohr`.
    ///
    /// # Errors
    ///
    /// [`ConfigError::NoHome`] when the home directory cannot be found.
    pub fn default_dir() -> Result<PathBuf, ConfigError> {
        let args = AppStrategyArgs {
            top_level_domain: "hr".into(),
            author: "InOrbit".into(),
            app_name: "iohr".into(),
        };
        choose_native_strategy(args)
            .map(|s| s.config_dir())
            .map_err(|_| ConfigError::NoHome)
    }

    /// Parses a config from TOML text.
    ///
    /// # Errors
    ///
    /// [`ConfigError::Invalid`] naming `path` when the text is not a valid config.
    pub fn parse(text: &str, path: &Path) -> Result<Self, ConfigError> {
        let invalid = |message: String| ConfigError::Invalid {
            path: path.to_owned(),
            message,
        };
        let mut config: Self = toml::from_str(text).map_err(|e| invalid(e.message().to_owned()))?;
        let doc = text
            .parse::<toml_edit::DocumentMut>()
            .map_err(|e| invalid(e.message().to_owned()))?;
        config.source = Source(Some(doc));
        Ok(config)
    }

    /// The file's text with this config's settings: the read document with `default`,
    /// the command line's profile keys and `[ext]` brought up to date, and everything
    /// else (the `[sdk]` table, SDK keys in a profile, unknown keys, comments) kept.
    #[must_use]
    pub fn to_toml(&self) -> String {
        use toml_edit::{Item, Table, value};
        let mut doc = self.source.0.clone().unwrap_or_default();
        match &self.default {
            Some(d) => set_str(doc.as_table_mut(), "default", Some(d.as_str())),
            None => {
                doc.remove("default");
            }
        }

        // Profiles: the command line's keys of each of its profiles. A profile it no
        // longer has loses those keys, and its table goes only when nothing is left.
        if !doc.contains_key("profiles") {
            let mut t = Table::new();
            t.set_implicit(true);
            doc.insert("profiles", Item::Table(t));
        }
        if let Some(all) = doc.get_mut("profiles").and_then(Item::as_table_like_mut) {
            let made_here: Vec<String> = all
                .iter()
                .filter(|(_, item)| item.as_table_like().is_some_and(|t| t.contains_key("kind")))
                .map(|(k, _)| k.to_owned())
                .collect();
            for name in made_here {
                let ours = name
                    .parse::<ProfileName>()
                    .is_ok_and(|n| self.profiles.contains_key(&n));
                if ours {
                    continue;
                }
                let empty = all
                    .get_mut(&name)
                    .and_then(Item::as_table_like_mut)
                    .is_none_or(|t| {
                        for k in PROFILE_KEYS {
                            t.remove(k);
                        }
                        t.is_empty()
                    });
                if empty {
                    all.remove(&name);
                }
            }
            for (name, p) in &self.profiles {
                if !all.contains_key(name.as_str()) {
                    all.insert(name.as_str(), Item::Table(Table::new()));
                }
                let Some(t) = all.get_mut(name.as_str()).and_then(Item::as_table_like_mut) else {
                    // Not a table: the typed parse would have refused the file.
                    continue;
                };
                set_str(t, "kind", Some(kind_str(p.kind)));
                set_str(t, "account", Some(&p.account));
                set_flag(t, "account_pinned", p.account_pinned);
                set_str(t, "storage", Some(storage_str(p.storage)));
                set_str(t, "issuer", p.issuer.as_deref());
                set_str(t, "client_id", p.client_id.as_deref());
            }
        }
        if doc
            .get("profiles")
            .and_then(Item::as_table_like)
            .is_some_and(toml_edit::TableLike::is_empty)
        {
            doc.remove("profiles");
        }

        // `[ext]`: its two keys; unknown ones stay.
        if !self.ext.is_empty() && !doc.contains_key("ext") {
            doc.insert("ext", Item::Table(Table::new()));
        }
        if let Some(ext) = doc.get_mut("ext").and_then(Item::as_table_like_mut) {
            set_str(ext, "registry", self.ext.registry.as_deref());
            if self.ext.trusted_keys.is_empty() {
                ext.remove("trusted_keys");
            } else {
                let keys: toml_edit::Array = self.ext.trusted_keys.iter().collect();
                let same = ext
                    .get("trusted_keys")
                    .and_then(Item::as_array)
                    .is_some_and(|a| {
                        a.iter().map(toml_edit::Value::as_str).eq(self
                            .ext
                            .trusted_keys
                            .iter()
                            .map(|k| Some(k.as_str())))
                    });
                if !same {
                    match ext.get_mut("trusted_keys") {
                        Some(item) => *item = value(keys),
                        None => {
                            ext.insert("trusted_keys", value(keys));
                        }
                    }
                }
            }
            if ext.is_empty() {
                doc.remove("ext");
            }
        }
        doc.to_string()
    }

    /// Reads `config.toml` from `dir`; a missing file is an empty config.
    ///
    /// # Errors
    ///
    /// [`ConfigError::Io`] when the file exists but cannot be read, and
    /// [`ConfigError::Invalid`] when it is not a valid config.
    pub fn load(dir: &Path) -> Result<Self, ConfigError> {
        let path = dir.join(FILE);
        match fs::read_to_string(&path) {
            Ok(text) => Self::parse(&text, &path),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(source) => Err(ConfigError::Io {
                action: "read",
                path,
                source,
            }),
        }
    }

    /// Writes `config.toml` into `dir`, creating the directory owner-only. The file is
    /// replaced in one step, so a crash never leaves half a config.
    ///
    /// # Errors
    ///
    /// [`ConfigError::Io`] when the directory or the file cannot be written.
    pub fn save(&self, dir: &Path) -> Result<(), ConfigError> {
        let path = dir.join(FILE);
        let text = self.to_toml();
        write_private(dir, FILE, text.as_bytes()).map_err(|source| ConfigError::Io {
            action: "write",
            path,
            source,
        })
    }

    /// The profile `name`, if there is one.
    #[must_use]
    pub fn profile(&self, name: &ProfileName) -> Option<&Profile> {
        self.profiles.get(name)
    }
}

/// Sets `key` to the string `v`, or removes it when `v` is `None`. An equal value is left
/// alone, so its comments and layout stay.
fn set_str(t: &mut dyn toml_edit::TableLike, key: &str, v: Option<&str>) {
    match v {
        None => {
            t.remove(key);
        }
        Some(v) => {
            let old = t.get(key);
            if old.and_then(toml_edit::Item::as_str) == Some(v) {
                return;
            }
            let mut new = toml_edit::Value::from(v);
            // Keep a comment written after the old value.
            if let Some(decor) = old
                .and_then(toml_edit::Item::as_value)
                .map(toml_edit::Value::decor)
            {
                *new.decor_mut() = decor.clone();
            }
            // Assigning in place keeps the key and the comments above it.
            match t.get_mut(key) {
                Some(item) => *item = toml_edit::Item::Value(new),
                None => {
                    t.insert(key, toml_edit::Item::Value(new));
                }
            }
        }
    }
}

/// Sets `key` to `true`, or removes it when `on` is false (an absent key means false).
fn set_flag(t: &mut dyn toml_edit::TableLike, key: &str, on: bool) {
    if !on {
        t.remove(key);
    } else if t.get(key).and_then(toml_edit::Item::as_bool) != Some(true) {
        t.insert(key, toml_edit::value(true));
    }
}

fn kind_str(k: Kind) -> &'static str {
    match k {
        Kind::Token => "token",
        Kind::Person => "person",
    }
}

fn storage_str(s: Storage) -> &'static str {
    match s {
        Storage::Keyring => "keyring",
        Storage::File => "file",
    }
}

/// Writes `bytes` to `dir/name` through a temporary file and a rename, with the
/// directory mode 0700 and the file mode 0600 on Unix.
pub(crate) fn write_private(dir: &Path, name: &str, bytes: &[u8]) -> io::Result<()> {
    create_private_dir(dir)?;
    let tmp = dir.join(format!(".{name}.tmp"));
    {
        let mut f = private_file(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, dir.join(name))
}

/// Creates `dir` (and its parents) with mode 0700 on Unix.
///
/// # Errors
/// The directory could not be created.
pub fn create_private_dir(dir: &Path) -> io::Result<()> {
    let mut b = fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
    b.create(dir)
}

fn private_file(path: &Path) -> io::Result<fs::File> {
    let mut o = fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    o.open(path)
}

#[cfg(test)]
mod tests {
    use super::{Config, Kind, Profile, ProfileName, RunnerConfig, Storage};

    fn name(s: &str) -> ProfileName {
        s.parse().unwrap()
    }

    #[test]
    fn profile_names() {
        for ok in ["personal", "acme-ci", "a", "x_1", "0team"] {
            assert!(ok.parse::<ProfileName>().is_ok(), "{ok}");
        }
        for bad in ["", "-x", "Upper", "a b", "a/b", "é", &"a".repeat(65)] {
            assert!(bad.parse::<ProfileName>().is_err(), "{bad}");
        }
    }

    #[test]
    fn round_trips_through_the_file_and_holds_no_secret_field() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = Config {
            default: Some(name("work")),
            ..Config::default()
        };
        c.profiles.insert(
            name("work"),
            Profile::new(Kind::Token, "acc_1", Storage::Keyring),
        );
        c.save(dir.path()).unwrap();
        let text = std::fs::read_to_string(dir.path().join("config.toml")).unwrap();
        assert!(text.contains("account = \"acc_1\""));
        assert!(!text.contains("token =") && !text.contains("secret"));
        assert_eq!(Config::load(dir.path()).unwrap(), c);
    }

    #[test]
    fn the_ask_read_folders_are_read_and_a_table_with_only_them_is_not_empty() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("config.toml"),
            "[ask]\nread = [\"~/notes\", \"/srv/docs\"]\n",
        )
        .unwrap();
        let c = Config::load(dir.path()).unwrap();
        assert_eq!(
            c.ask.read.as_deref(),
            Some(&["~/notes".to_owned(), "/srv/docs".to_owned()][..])
        );
        assert!(!c.ask.is_empty());
        assert!(Config::default().ask.read.is_none());
        assert!(Config::default().ask.is_empty());
    }

    #[test]
    fn the_project_table_is_read_and_a_save_keeps_it() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("config.toml"),
            "[project]   # my default\ntemplate = \"coordinated\"\nroot = \"~/code\"\nagent_model = \"opus\"\nagent_turns = 90\nagent_timeout = 900\n",
        )
        .unwrap();
        let mut c = Config::load(dir.path()).unwrap();
        assert_eq!(c.project.template.as_deref(), Some("coordinated"));
        assert_eq!(c.project.root.as_deref(), Some("~/code"));
        assert_eq!(c.project.agent_model.as_deref(), Some("opus"));
        assert_eq!(
            (c.project.agent_turns, c.project.agent_timeout),
            (Some(90), Some(900))
        );
        c.default = Some(name("work"));
        c.save(dir.path()).unwrap();
        let saved = std::fs::read_to_string(dir.path().join("config.toml")).unwrap();
        assert!(saved.contains("[project]   # my default"), "{saved}");
        assert!(saved.contains("template = \"coordinated\""), "{saved}");
        assert!(saved.contains("root = \"~/code\""), "{saved}");
        assert!(saved.contains("agent_turns = 90"), "{saved}");
    }

    #[test]
    fn the_runner_tables_are_read_and_a_save_keeps_them() {
        let text = r#"default = "work"

[runner]                 # this machine
capacity = 4
kinds = ["coordinate", "build"]
always_on = true
auto_upgrade = false
upgrade_interval_minutes = 30
upgrade_restart = "exit"
upgrade_sign_identity = "auto"

[profiles.work]
kind = "token"
account = "acc_1"
storage = "keyring"

[profiles.work.runner]   # this profile only
capacity = 8
"#;
        let path = std::path::Path::new("config.toml");
        let mut c = Config::parse(text, path).unwrap();
        assert_eq!(c.runner.capacity, Some(4));
        assert_eq!(
            c.runner.kinds.as_deref(),
            Some(&["coordinate".to_owned(), "build".to_owned()][..])
        );
        assert_eq!(c.runner.always_on, Some(true));
        assert_eq!(c.runner.name, None);
        assert_eq!(c.runner.auto_upgrade, Some(false));
        assert_eq!(c.runner.upgrade_interval_minutes, Some(30));
        assert_eq!(c.runner.upgrade_restart.as_deref(), Some("exit"));
        assert_eq!(c.runner.upgrade_sign_identity.as_deref(), Some("auto"));
        let work = c.profile(&name("work")).unwrap();
        assert_eq!(
            work.runner,
            Some(RunnerConfig {
                capacity: Some(8),
                ..RunnerConfig::default()
            })
        );
        // The command line changes what it owns; the runner tables stay as written.
        c.default = None;
        let saved = c.to_toml();
        assert!(
            saved.contains("[runner]                 # this machine"),
            "{saved}"
        );
        assert!(
            saved.contains("[profiles.work.runner]   # this profile only"),
            "{saved}"
        );
        assert!(saved.contains("capacity = 8"), "{saved}");
        assert!(saved.contains("upgrade_interval_minutes = 30"), "{saved}");
        assert!(
            saved.contains("upgrade_sign_identity = \"auto\""),
            "{saved}"
        );
        assert!(!saved.contains("default ="), "{saved}");
        assert_eq!(Config::parse(&saved, path).unwrap(), c);
    }

    #[test]
    fn a_chosen_account_is_written_only_when_set_and_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = Config::default();
        c.profiles.insert(
            name("me"),
            Profile::person(
                "acc_1",
                Storage::Keyring,
                "https://auth.inorbit.hr",
                "iohr-cli",
            ),
        );
        c.save(dir.path()).unwrap();
        let text = std::fs::read_to_string(dir.path().join("config.toml")).unwrap();
        assert!(!text.contains("account_pinned"), "{text}");
        assert!(!Config::load(dir.path()).unwrap().profiles[&name("me")].account_pinned);

        c.profiles.get_mut(&name("me")).unwrap().account_pinned = true;
        c.save(dir.path()).unwrap();
        let text = std::fs::read_to_string(dir.path().join("config.toml")).unwrap();
        assert!(text.contains("account_pinned = true"), "{text}");
        assert_eq!(Config::load(dir.path()).unwrap(), c);

        // Unpinned again: the key goes, the rest stays.
        c.profiles.get_mut(&name("me")).unwrap().account_pinned = false;
        c.save(dir.path()).unwrap();
        let text = std::fs::read_to_string(dir.path().join("config.toml")).unwrap();
        assert!(
            !text.contains("account_pinned") && text.contains("account = \"acc_1\""),
            "{text}"
        );
    }

    #[test]
    fn a_missing_file_is_an_empty_config_and_a_bad_one_names_the_file() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(Config::load(dir.path()).unwrap(), Config::default());
        std::fs::write(dir.path().join("config.toml"), "default = 3").unwrap();
        let e = Config::load(dir.path()).unwrap_err().to_string();
        assert!(e.contains("config.toml"), "{e}");
    }

    /// A file shared with the SDKs (`docs/config.md` section 4.2): comments, the `[sdk]`
    /// table, SDK keys in a command-line profile, a profile with SDK keys only, and keys
    /// nobody knows yet.
    const SHARED: &str = r#"# Written by hand; keep this comment.
default = "work"                    # set by iohr login

[sdk]                               # every profile
log = "warn"
proxy = "http://proxy.corp.example:3128"
future_key = { nested = true }

[profiles.work]                     # written by iohr login
kind = "person"
account = "acc_8d2e" # the team
storage = "keyring"
issuer = "https://auth.inorbit.hr"
client_id = "iohr-cli"
timeout = "10s"                     # an SDK key in a command-line profile

[profiles.ci]                       # SDK only
key_id = "ak_7f3c"
key_secret_file = "/run/secrets/inorbit-ci"
scopes = ["identity:read", "radar:read"]

[ext]
registry = "registry.acme.hr/inorbit/iohr-ext"
mirror_note = "kept"
"#;

    fn shared(dir: &std::path::Path) -> Config {
        std::fs::write(dir.join("config.toml"), SHARED).unwrap();
        Config::load(dir).unwrap()
    }

    fn reread(dir: &std::path::Path) -> (String, toml::Table) {
        let text = std::fs::read_to_string(dir.join("config.toml")).unwrap();
        let table = text.parse::<toml::Table>().unwrap();
        (text, table)
    }

    #[test]
    fn a_profile_with_sdk_keys_only_is_not_a_command_line_profile() {
        let dir = tempfile::tempdir().unwrap();
        let c = shared(dir.path());
        assert_eq!(c.profiles.len(), 1);
        assert!(c.profile(&name("work")).is_some());
        assert!(c.profile(&name("ci")).is_none());
        assert_eq!(c.default, Some(name("work")));
    }

    #[test]
    fn saving_unchanged_keeps_the_file_byte_for_byte() {
        let dir = tempfile::tempdir().unwrap();
        shared(dir.path()).save(dir.path()).unwrap();
        assert_eq!(reread(dir.path()).0, SHARED);
    }

    #[test]
    fn changes_keep_sdk_keys_unknown_keys_and_comments() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = shared(dir.path());
        // What login, profile use, profile account and config set do.
        c.profiles.insert(
            name("new"),
            Profile::new(Kind::Token, "acc_2", Storage::File),
        );
        c.default = Some(name("new"));
        if let Some(w) = c.profiles.get_mut(&name("work")) {
            w.account = "acc_9".into();
        }
        c.ext.trusted_keys = vec!["-----BEGIN PUBLIC KEY-----".into()];
        c.save(dir.path()).unwrap();

        let (text, t) = reread(dir.path());
        for kept in [
            "# Written by hand; keep this comment.",
            "# set by iohr login",
            "# every profile",
            "# the team",
            "# an SDK key in a command-line profile",
            "# SDK only",
        ] {
            assert!(text.contains(kept), "{kept} lost:\n{text}");
        }
        assert_eq!(t["default"].as_str(), Some("new"));
        assert_eq!(t["sdk"]["log"].as_str(), Some("warn"));
        assert_eq!(t["sdk"]["future_key"]["nested"].as_bool(), Some(true));
        assert_eq!(t["profiles"]["work"]["account"].as_str(), Some("acc_9"));
        assert_eq!(t["profiles"]["work"]["timeout"].as_str(), Some("10s"));
        assert_eq!(t["profiles"]["ci"]["key_id"].as_str(), Some("ak_7f3c"));
        assert_eq!(
            t["profiles"]["ci"]["scopes"].as_array().map(Vec::len),
            Some(2)
        );
        assert_eq!(t["profiles"]["new"]["kind"].as_str(), Some("token"));
        assert_eq!(t["profiles"]["new"]["storage"].as_str(), Some("file"));
        assert_eq!(t["ext"]["mirror_note"].as_str(), Some("kept"));
        assert_eq!(t["ext"]["trusted_keys"].as_array().map(Vec::len), Some(1));
        // And it reads back as the same config.
        assert_eq!(Config::load(dir.path()).unwrap(), c);
    }

    #[test]
    fn forgetting_a_profile_keeps_its_sdk_keys() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = shared(dir.path());
        // What logout does.
        c.profiles.remove(&name("work"));
        c.default = None;
        c.ext.registry = None;
        c.save(dir.path()).unwrap();

        let (_, t) = reread(dir.path());
        assert!(t.get("default").is_none());
        let work = t["profiles"]["work"].as_table().unwrap();
        assert_eq!(work.len(), 1, "{work:?}");
        assert_eq!(work["timeout"].as_str(), Some("10s"));
        assert!(t["profiles"].get("ci").is_some());
        assert!(t["ext"].get("registry").is_none());
        assert_eq!(t["ext"]["mirror_note"].as_str(), Some("kept"));
        assert!(Config::load(dir.path()).unwrap().profiles.is_empty());
    }

    #[test]
    fn a_profile_with_only_command_line_keys_goes_entirely() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("config.toml"),
            "default = \"a\"\n\n[profiles.a]\nkind = \"token\"\naccount = \"acc_1\"\n",
        )
        .unwrap();
        let mut c = Config::load(dir.path()).unwrap();
        c.profiles.clear();
        c.default = None;
        c.save(dir.path()).unwrap();
        assert_eq!(reread(dir.path()).0.trim(), "");
    }

    #[cfg(unix)]
    #[test]
    fn the_file_and_directory_are_owner_only() {
        use std::os::unix::fs::PermissionsExt as _;
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("iohr");
        Config::default().save(&dir).unwrap();
        let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(&dir.join("config.toml")), 0o600);
    }
}
