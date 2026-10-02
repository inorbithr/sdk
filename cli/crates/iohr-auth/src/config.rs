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
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Config {
    /// The profile used when no `--profile` or `IOHR_PROFILE` is given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<ProfileName>,
    /// Every profile, by name.
    #[serde(default)]
    pub profiles: BTreeMap<ProfileName, Profile>,
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
}

impl Profile {
    /// A profile for `account` holding a credential of `kind` in `storage`.
    #[must_use]
    pub fn new(kind: Kind, account: impl Into<String>, storage: Storage) -> Self {
        Self {
            kind,
            account: account.into(),
            storage,
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
        toml::from_str(text).map_err(|e| ConfigError::Invalid {
            path: path.to_owned(),
            message: e.message().to_owned(),
        })
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
        let text = toml::to_string_pretty(self).map_err(|e| ConfigError::Invalid {
            path: path.clone(),
            message: e.to_string(),
        })?;
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
