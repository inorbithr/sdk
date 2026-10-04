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
/// every other key, table and comment as it was read.
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
const PROFILE_KEYS: [&str; 5] = ["kind", "account", "storage", "issuer", "client_id"];

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

/// One way of calling the API.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Profile {
    /// What kind of credential the profile holds.
    pub kind: Kind,
    /// The account the calls count against.
    pub account: String,
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
            storage,
            issuer: None,
            client_id: None,
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

pub(crate) fn create_private_dir(dir: &Path) -> io::Result<()> {
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
