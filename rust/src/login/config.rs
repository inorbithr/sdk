//! The profile table `iohr login` writes (`docs/config.md` sections 4.2 and 5.6): the
//! profiles on this machine, which one is the default, and the owner-only file helpers
//! the credential store shares.
//!
//! The file also holds the SDK's own `[sdk]` table, SDK keys inside a profile, and the
//! command line's other tables (`[ext]`, `[runner]`, ...). This module reads and writes
//! only `default` and the profile keys it owns, and keeps everything else, comments
//! included, as it was read.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::config::{Os, config_path};

const FILE: &str = "config.toml";

/// The profiles on this machine and which one is the default.
///
/// Stored as `config.toml` in the platform's config directory (XDG on Linux,
/// Application Support on macOS, `%APPDATA%` on Windows). It holds names, account ids
/// and settings, never a secret.
///
/// [`save`](Self::save) changes only `default` and the keys of [`Profile`]; every other
/// key, table and comment stays as it was read.
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
    /// The file as it was read, so a save keeps what this module does not own.
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
        }
    }

    /// The kind of credential the profile holds.
    #[must_use]
    pub fn kind(&self) -> Kind {
        self.kind
    }

    /// The account the calls count against.
    #[must_use]
    pub fn account(&self) -> &str {
        &self.account
    }

    /// Whether a person chose the account (`account_pinned`).
    #[must_use]
    pub fn account_pinned(&self) -> bool {
        self.account_pinned
    }

    /// Where the profile's secret is kept.
    #[must_use]
    pub fn storage(&self) -> Storage {
        self.storage
    }

    /// For a person: the sign-in service the session refreshes with.
    #[must_use]
    pub fn issuer(&self) -> Option<&str> {
        self.issuer.as_deref()
    }

    /// For a person: the OAuth client the session belongs to.
    #[must_use]
    pub fn client_id(&self) -> Option<&str> {
        self.client_id.as_deref()
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

impl AsRef<str> for ProfileName {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl std::borrow::Borrow<str> for ProfileName {
    fn borrow(&self) -> &str {
        &self.0
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
    /// The platform's config directory for `iohr`, as `docs/config.md` section 4.1 says
    /// (`IOHR_CONFIG_DIR` first).
    ///
    /// # Errors
    ///
    /// [`ConfigError::NoHome`] when the home directory cannot be found.
    pub fn default_dir() -> Result<PathBuf, ConfigError> {
        let os = Os::current();
        let env: BTreeMap<String, String> = ["IOHR_CONFIG_DIR", "XDG_CONFIG_HOME", "APPDATA"]
            .into_iter()
            .filter_map(|k| std::env::var(k).ok().map(|v| (k.to_owned(), v)))
            .collect();
        let home = std::env::var(if os == Os::Windows {
            "USERPROFILE"
        } else {
            "HOME"
        })
        .ok();
        config_path(os, &env, home.as_deref(), None)
            .and_then(|(file, _, _)| Path::new(&file).parent().map(Path::to_path_buf))
            .ok_or(ConfigError::NoHome)
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

    /// The file's text with this config's settings: the read document with `default` and
    /// the command line's profile keys brought up to date, and everything else (the
    /// `[sdk]` table, SDK keys in a profile, other tables, unknown keys, comments) kept.
    #[must_use]
    pub fn to_toml(&self) -> String {
        self.to_toml_with(|_| {})
    }

    /// [`to_toml`](Self::to_toml) with `extra` applied to the document last, for a caller
    /// that owns more tables of the same file (`iohr`'s `[ext]`). Not part of the stable
    /// API: its argument is a `toml_edit` type.
    #[doc(hidden)]
    #[must_use]
    pub fn to_toml_with(&self, extra: impl FnOnce(&mut toml_edit::DocumentMut)) -> String {
        use toml_edit::{Item, Table};
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

        extra(&mut doc);
        doc.to_string()
    }

    /// Reads `config.toml` from `dir`; a missing file is an empty config.
    ///
    /// # Errors
    ///
    /// [`ConfigError::Io`] when the file exists but cannot be read, and
    /// [`ConfigError::Invalid`] when it is not a valid config.
    pub fn load(dir: &Path) -> Result<Self, ConfigError> {
        match Self::read_text(dir)? {
            Some(text) => Self::parse(&text, &dir.join(FILE)),
            None => Ok(<Self as Default>::default()),
        }
    }

    /// The text of `config.toml` in `dir`; `None` when there is no such file.
    ///
    /// # Errors
    ///
    /// [`ConfigError::Io`] when the file exists but cannot be read.
    pub fn read_text(dir: &Path) -> Result<Option<String>, ConfigError> {
        let path = dir.join(FILE);
        match fs::read_to_string(&path) {
            Ok(text) => Ok(Some(text)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
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
        Self::write_text(dir, &self.to_toml())
    }

    /// Writes `text` as `config.toml` in `dir`, as [`save`](Self::save) does.
    ///
    /// # Errors
    ///
    /// [`ConfigError::Io`] when the directory or the file cannot be written.
    pub fn write_text(dir: &Path, text: &str) -> Result<(), ConfigError> {
        let path = dir.join(FILE);
        write_private(dir, FILE, text.as_bytes()).map_err(|source| ConfigError::Io {
            action: "write",
            path,
            source,
        })
    }

    /// The default profile's name, if the file names one.
    #[must_use]
    pub fn default(&self) -> Option<&ProfileName> {
        self.default.as_ref()
    }

    /// The profile `name`, if there is one.
    #[must_use]
    pub fn profile(&self, name: impl AsRef<str>) -> Option<&Profile> {
        self.profiles.get(name.as_ref())
    }
}

/// Sets `key` to the string `v`, or removes it when `v` is `None`. An equal value is left
/// alone, so its comments and layout stay.
#[doc(hidden)]
pub fn set_str(t: &mut dyn toml_edit::TableLike, key: &str, v: Option<&str>) {
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

/// Writes `bytes` to `dir/name` through a new temporary file and a rename, with the
/// directory mode 0700 and the file mode 0600 on Unix. The temporary file has a random
/// name ([`temp_path`]) and is made by [`create_new_private`], so it is never one that
/// was there before (a link included) and never has a wider mode, even for a moment.
///
/// # Errors
/// The directory or the file could not be made, written or renamed; the temporary file
/// is removed again.
pub fn write_private(dir: &Path, name: &str, bytes: &[u8]) -> io::Result<()> {
    create_private_dir(dir)?;
    let tmp = temp_path(dir, name);
    let written = create_new_private(&tmp, 0o600)
        .and_then(|mut f| f.write_all(bytes).and_then(|()| f.sync_all()))
        .and_then(|()| fs::rename(&tmp, dir.join(name)));
    if written.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    written
}

/// A path in `dir` for a temporary file that becomes `name`: `.<name>.<random>.tmp`.
#[must_use]
pub fn temp_path(dir: &Path, name: &str) -> PathBuf {
    let mut r = [0u8; 8];
    // A failed draw still leaves a name `create_new_private` refuses if it is taken.
    let _ = getrandom::fill(&mut r);
    dir.join(format!(".{name}.{:016x}.tmp", u64::from_le_bytes(r)))
}

/// Creates the file `path` for writing, refusing one that is already there (a link
/// included, so nothing is written through it), with `mode` on Unix set as it is made.
///
/// # Errors
/// The file exists or could not be created.
pub fn create_new_private(path: &Path, mode: u32) -> io::Result<fs::File> {
    let mut o = fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, mode);
    #[cfg(not(unix))]
    let _ = mode;
    o.open(path)
}

/// Creates `dir` (and its parents) with mode 0700 on Unix; a `dir` that is already there
/// with a group or world bit is made 0700 too, when this user may change it.
///
/// # Errors
/// The directory could not be created.
pub fn create_private_dir(dir: &Path) -> io::Result<()> {
    let mut b = fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
    b.create(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = fs::metadata(dir)?.permissions().mode();
        if mode & 0o077 != 0 {
            match fs::set_permissions(dir, fs::Permissions::from_mode(0o700)) {
                // Another user's directory: its owner decides, and a caller that must
                // not use it checks the owner itself (`runner::private`).
                Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {}
                other => other?,
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Config, Kind, Profile, ProfileName, Storage};

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
            ..<Config as Default>::default()
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
    fn a_chosen_account_is_written_only_when_set_and_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = <Config as Default>::default();
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
        assert_eq!(
            Config::load(dir.path()).unwrap(),
            <Config as Default>::default()
        );
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
        assert!(c.profile("work").is_some());
        assert!(c.profile("ci").is_none());
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
        // A table this module does not own is kept whole.
        assert_eq!(t["ext"]["mirror_note"].as_str(), Some("kept"));
        assert_eq!(
            t["ext"]["registry"].as_str(),
            Some("registry.acme.hr/inorbit/iohr-ext")
        );
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
        c.save(dir.path()).unwrap();

        let (_, t) = reread(dir.path());
        assert!(t.get("default").is_none());
        let work = t["profiles"]["work"].as_table().unwrap();
        assert_eq!(work.len(), 1, "{work:?}");
        assert_eq!(work["timeout"].as_str(), Some("10s"));
        assert!(t["profiles"].get("ci").is_some());
        assert!(t["ext"].get("registry").is_some());
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
        <Config as Default>::default().save(&dir).unwrap();
        let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(&dir.join("config.toml")), 0o600);
    }

    /// A file or link already at a temporary name is never opened: the new file is
    /// made fresh, mode 0600 from the start, and what was there is left alone.
    #[cfg(unix)]
    #[test]
    fn a_private_write_never_writes_through_what_is_there() {
        use std::os::unix::fs::PermissionsExt as _;
        let d = tempfile::tempdir().unwrap();
        let elsewhere = d.path().join("elsewhere");
        std::fs::write(&elsewhere, "kept").unwrap();
        let link = d.path().join("link");
        std::os::unix::fs::symlink(&elsewhere, &link).unwrap();
        assert!(super::create_new_private(&link, 0o600).is_err());
        assert_eq!(std::fs::read_to_string(&elsewhere).unwrap(), "kept");

        let a = super::temp_path(d.path(), "x");
        assert_ne!(a, super::temp_path(d.path(), "x"));
        let f = super::create_new_private(&a, 0o600).unwrap();
        assert_eq!(f.metadata().unwrap().permissions().mode() & 0o777, 0o600);
        assert!(super::create_new_private(&a, 0o600).is_err());

        let dir = d.path().join("cfg");
        super::write_private(&dir, "token", b"one").unwrap();
        super::write_private(&dir, "token", b"two").unwrap();
        let p = dir.join("token");
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "two");
        assert_eq!(
            std::fs::metadata(&p).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let names: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["token"], "a temporary file was left behind");
    }

    /// A private directory that is already there with a wider mode is made 0700.
    #[cfg(unix)]
    #[test]
    fn an_existing_private_directory_is_tightened() {
        use std::os::unix::fs::PermissionsExt as _;
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("cfg");
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        super::create_private_dir(&dir).unwrap();
        assert_eq!(
            std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
}
