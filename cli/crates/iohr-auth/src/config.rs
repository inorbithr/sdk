use std::collections::BTreeMap;
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};

use inorbithr::login::{self, ConfigError, Profile, ProfileName, set_str};
use serde::{Deserialize, Serialize};

/// The profiles on this machine, which one is the default, and the command line's own
/// tables.
///
/// The profile table (`default`, `[profiles.<name>]`, the file's comments and every key
/// the command line does not own) is [`inorbithr::login::Config`], the SDK's, and a
/// `Config` derefs to it, so `config.profiles` and `config.profile(name)` are the SDK's.
/// This type adds what only `iohr` has: the `[ext]`, `[runner]`, `[project]`, `[ask]`,
/// `[sync]`, `[memory]`, `[panel]` and `[doctor]` tables, and a profile's own
/// `[profiles.<name>.runner]`.
///
/// The file is shared with the SDKs (`docs/config.md` section 4) and never holds a secret
/// (SR-24). [`save`](Self::save) changes only `default`, the keys of [`Profile`] and `[ext]`
/// and keeps every other key, table and comment as it was read. The `[runner]` table (and a
/// profile's `[profiles.<name>.runner]`) is read, never written: a person edits it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Config {
    base: login::Config,
    /// Where extensions come from and which extra keys may sign them.
    pub ext: ExtConfig,
    /// `iohr runner` settings (`docs/config.md` section 4.4); a profile's own
    /// `[profiles.<name>.runner]` ([`profile_runner`](Self::profile_runner)) overrides
    /// them key by key.
    pub runner: RunnerConfig,
    /// `iohr project init` defaults (`[project]`); read, never written: a person edits it.
    pub project: ProjectConfig,
    /// `iohr ask` defaults (`[ask]`); read, never written: a person edits it.
    pub ask: AskConfig,
    /// `iohr sync` settings (`[sync]`); read, never written: a person edits it.
    pub sync: SyncConfig,
    /// `iohr memory` settings (`[memory]`); read, never written: a person edits it.
    pub memory: MemoryConfig,
    /// `iohr` panel settings (`[panel]`); read, never written: a person edits it.
    pub panel: PanelConfig,
    /// `iohr doctor` settings (`[doctor]`); read, never written: a person edits it.
    pub doctor: DoctorConfig,
    /// Each command-line profile's `[profiles.<name>.runner]`, read from the file.
    profile_runners: BTreeMap<ProfileName, RunnerConfig>,
}

impl Deref for Config {
    type Target = login::Config;
    fn deref(&self) -> &login::Config {
        &self.base
    }
}

impl DerefMut for Config {
    fn deref_mut(&mut self) -> &mut login::Config {
        &mut self.base
    }
}

/// The JSON the profile commands print: the SDK's `default` and `profiles` (each with its
/// own `runner` table) and the command line's tables, as one document.
impl Serialize for Config {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct ProfileView<'a> {
            #[serde(flatten)]
            profile: &'a Profile,
            #[serde(skip_serializing_if = "Option::is_none")]
            runner: Option<&'a RunnerConfig>,
        }
        #[derive(Serialize)]
        struct View<'a> {
            #[serde(skip_serializing_if = "Option::is_none")]
            default: Option<&'a ProfileName>,
            profiles: BTreeMap<&'a ProfileName, ProfileView<'a>>,
            #[serde(skip_serializing_if = "ExtConfig::is_empty")]
            ext: &'a ExtConfig,
            #[serde(skip_serializing_if = "RunnerConfig::is_empty")]
            runner: &'a RunnerConfig,
            #[serde(skip_serializing_if = "ProjectConfig::is_empty")]
            project: &'a ProjectConfig,
            #[serde(skip_serializing_if = "AskConfig::is_empty")]
            ask: &'a AskConfig,
            #[serde(skip_serializing_if = "SyncConfig::is_empty")]
            sync: &'a SyncConfig,
            #[serde(skip_serializing_if = "MemoryConfig::is_empty")]
            memory: &'a MemoryConfig,
            #[serde(skip_serializing_if = "PanelConfig::is_empty")]
            panel: &'a PanelConfig,
            #[serde(skip_serializing_if = "DoctorConfig::is_empty")]
            doctor: &'a DoctorConfig,
        }
        View {
            default: self.base.default.as_ref(),
            profiles: self
                .base
                .profiles
                .iter()
                .map(|(n, profile)| {
                    let runner = self.profile_runners.get(n);
                    (n, ProfileView { profile, runner })
                })
                .collect(),
            ext: &self.ext,
            runner: &self.runner,
            project: &self.project,
            ask: &self.ask,
            sync: &self.sync,
            memory: &self.memory,
            panel: &self.panel,
            doctor: &self.doctor,
        }
        .serialize(s)
    }
}

/// What the command line reads beyond the SDK's profile table.
#[derive(Default, Deserialize)]
struct Extras {
    #[serde(default)]
    ext: ExtConfig,
    #[serde(default)]
    runner: RunnerConfig,
    #[serde(default)]
    project: ProjectConfig,
    #[serde(default)]
    ask: AskConfig,
    #[serde(default)]
    sync: SyncConfig,
    #[serde(default)]
    memory: MemoryConfig,
    #[serde(default)]
    panel: PanelConfig,
    #[serde(default)]
    doctor: DoctorConfig,
    #[serde(default)]
    profiles: BTreeMap<ProfileName, ProfileExtras>,
}

#[derive(Default, Deserialize)]
struct ProfileExtras {
    runner: Option<RunnerConfig>,
}

impl Config {
    /// The platform's config directory for `iohr`, as `docs/config.md` section 4.1 says
    /// (`IOHR_CONFIG_DIR` first).
    ///
    /// # Errors
    ///
    /// [`ConfigError::NoHome`] when the home directory cannot be found.
    pub fn default_dir() -> Result<PathBuf, ConfigError> {
        login::Config::default_dir()
    }

    /// Parses a config from TOML text.
    ///
    /// # Errors
    ///
    /// [`ConfigError::Invalid`] naming `path` when the text is not a valid config.
    pub fn parse(text: &str, path: &Path) -> Result<Self, ConfigError> {
        let base = login::Config::parse(text, path)?;
        let extras: Extras = toml::from_str(text).map_err(|e| ConfigError::Invalid {
            path: path.to_owned(),
            message: e.message().to_owned(),
        })?;
        let profile_runners = extras
            .profiles
            .into_iter()
            .filter(|(n, _)| base.profiles.contains_key(n))
            .filter_map(|(n, p)| p.runner.map(|r| (n, r)))
            .collect();
        Ok(Self {
            base,
            ext: extras.ext,
            runner: extras.runner,
            project: extras.project,
            ask: extras.ask,
            sync: extras.sync,
            memory: extras.memory,
            panel: extras.panel,
            doctor: extras.doctor,
            profile_runners,
        })
    }

    /// This profile's `[profiles.<name>.runner]` table, over the file's `[runner]`.
    #[must_use]
    pub fn profile_runner(&self, name: &ProfileName) -> Option<&RunnerConfig> {
        self.profile_runners.get(name)
    }

    /// The file's text with this config's settings: the SDK's
    /// [`to_toml`](login::Config::to_toml) (`default` and the command line's profile keys)
    /// and `[ext]` brought up to date, and everything else kept.
    #[must_use]
    pub fn to_toml(&self) -> String {
        self.base.to_toml_with(|doc| write_ext(doc, &self.ext))
    }

    /// Reads `config.toml` from `dir`; a missing file is an empty config.
    ///
    /// # Errors
    ///
    /// [`ConfigError::Io`] when the file exists but cannot be read, and
    /// [`ConfigError::Invalid`] when it is not a valid config.
    pub fn load(dir: &Path) -> Result<Self, ConfigError> {
        match login::Config::read_text(dir)? {
            Some(text) => Self::parse(&text, &dir.join("config.toml")),
            None => Ok(Self::default()),
        }
    }

    /// Writes `config.toml` into `dir`, creating the directory owner-only. The file is
    /// replaced in one step, so a crash never leaves half a config.
    ///
    /// # Errors
    ///
    /// [`ConfigError::Io`] when the directory or the file cannot be written.
    pub fn save(&self, dir: &Path) -> Result<(), ConfigError> {
        login::Config::write_text(dir, &self.to_toml())
    }
}

/// `[ext]`: its two keys; unknown ones stay.
fn write_ext(doc: &mut toml_edit::DocumentMut, ext_config: &ExtConfig) {
    use toml_edit::{Item, Table, value};
    if !ext_config.is_empty() && !doc.contains_key("ext") {
        doc.insert("ext", Item::Table(Table::new()));
    }
    if let Some(ext) = doc.get_mut("ext").and_then(Item::as_table_like_mut) {
        set_str(ext, "registry", ext_config.registry.as_deref());
        if ext_config.trusted_keys.is_empty() {
            ext.remove("trusted_keys");
        } else {
            let keys: toml_edit::Array = ext_config.trusted_keys.iter().collect();
            let same = ext
                .get("trusted_keys")
                .and_then(Item::as_array)
                .is_some_and(|a| {
                    a.iter()
                        .map(toml_edit::Value::as_str)
                        .eq(ext_config.trusted_keys.iter().map(|k| Some(k.as_str())))
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

/// The `[doctor]` table: whether `iohr doctor` and the `?` card may repair a local problem
/// themselves. Off when unset: a fix is offered, never run, until the person says yes here.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct DoctorConfig {
    /// Run the safe fixes (the ones that only restart something already down) without asking.
    /// Every other fix stays an offer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_fix: Option<bool>,
}

impl DoctorConfig {
    /// Whether nothing is set.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.auto_fix.is_none()
    }

    /// Whether the safe fixes run on their own (`auto_fix = true`).
    #[must_use]
    pub fn auto_fix_on(&self) -> bool {
        self.auto_fix == Some(true)
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
    /// The chat box at the bottom of the panel's terminal (RFC 0212): `on` (shown, the
    /// default), `hidden` (loaded, but a corner chip until Option+h brings it back) or `off`
    /// (never drawn, `?` stays the shell's). `IOHR_PANEL_CHAT` overrides it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat: Option<String>,
}

impl PanelConfig {
    /// Whether nothing is set.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.mouse.is_none() && self.chat.is_none()
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
    /// The volume build turns write to (an absolute path). While it has less than
    /// `build_volume_min_free_gb` free, the runner takes no new build turn. Unset, there
    /// is no such pause.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build_volume: Option<String>,
    /// The free space, in GB, `build_volume` must keep for new build turns (default 100).
    /// Needs `build_volume`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build_volume_min_free_gb: Option<u64>,
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
    /// Which sign-in Claude turns may run on: `api` (default; an Anthropic API key the
    /// runner keeps in the credential store), `plan` (your own Claude plan only, which
    /// is for personal use only; the stored key is not used) or `any` (either). Set
    /// here only: the account's policy can narrow a runner, never widen it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claude_auth: Option<String>,
    /// The same for Codex turns: `any` (default), `plan` or `api`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex_auth: Option<String>,
    /// Whether git may use this machine's own `gh` sign-in for a job when the platform
    /// says its GitHub App is not installed for the account (or mints no runner tokens).
    /// Unset or false, such a job is refused instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_host_git_credentials: Option<bool>,
    /// Days a job checkout, a mirror or a shared build directory may sit unused before
    /// the runner removes it (`iohr runner prune`, and the runner while it is idle).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_keep_days: Option<u32>,
    /// The most the checkouts, mirrors and build directories together may take, in GB;
    /// over it, the oldest unused go first. Unset, no cap.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_max_gb: Option<u64>,
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
            && self.build_volume.is_none()
            && self.build_volume_min_free_gb.is_none()
            && self.auto_upgrade.is_none()
            && self.upgrade_interval_minutes.is_none()
            && self.upgrade_restart.is_none()
            && self.upgrade_sign_identity.is_none()
            && self.user_units.is_none()
            && self.core_checkout.is_none()
            && self.claude_auth.is_none()
            && self.codex_auth.is_none()
            && self.allow_host_git_credentials.is_none()
            && self.workspace_keep_days.is_none()
            && self.workspace_max_gb.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::{Config, RunnerConfig};
    use inorbithr::login::{Kind, Profile, ProfileName, Storage};

    fn name(s: &str) -> ProfileName {
        s.parse().unwrap()
    }

    /// `[ext]` is the command line's: a save writes it, keeps its unknown keys, the SDK's
    /// tables and every comment, and drops the table only when nothing is left.
    #[test]
    fn ext_is_written_and_the_rest_of_the_file_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let text =
            "# keep\n[sdk]\nlog = \"warn\"\n\n[ext]\nregistry = \"r.example/x\"\nnote = \"kept\"\n";
        std::fs::write(dir.path().join("config.toml"), text).unwrap();
        let mut c = Config::load(dir.path()).unwrap();
        assert_eq!(c.ext.registry.as_deref(), Some("r.example/x"));
        c.save(dir.path()).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("config.toml")).unwrap(),
            text
        );
        c.ext.trusted_keys = vec!["-----BEGIN PUBLIC KEY-----".into()];
        c.ext.registry = None;
        c.profiles.insert(
            name("new"),
            Profile::new(Kind::Token, "acc_2", Storage::File),
        );
        c.save(dir.path()).unwrap();
        let saved = std::fs::read_to_string(dir.path().join("config.toml")).unwrap();
        let t = saved.parse::<toml::Table>().unwrap();
        assert!(saved.contains("# keep"), "{saved}");
        assert_eq!(t["sdk"]["log"].as_str(), Some("warn"));
        assert!(t["ext"].get("registry").is_none());
        assert_eq!(t["ext"]["note"].as_str(), Some("kept"));
        assert_eq!(t["ext"]["trusted_keys"].as_array().map(Vec::len), Some(1));
        assert_eq!(t["profiles"]["new"]["kind"].as_str(), Some("token"));
        assert_eq!(Config::load(dir.path()).unwrap(), c);
        // Cleared again: its own keys go, the unknown one stays.
        c.ext = super::ExtConfig::default();
        c.save(dir.path()).unwrap();
        let saved = std::fs::read_to_string(dir.path().join("config.toml")).unwrap();
        assert_eq!(
            saved.parse::<toml::Table>().unwrap()["ext"]["note"].as_str(),
            Some("kept")
        );
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
        assert!(c.profile(name("work")).is_some());
        assert_eq!(
            c.profile_runner(&name("work")),
            Some(&RunnerConfig {
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
}
