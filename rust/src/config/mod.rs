//! Configuration: where a client built with [`Client::load`](crate::Client::load)
//! finds its settings, and the description of what it found (`docs/config.md`).
//!
//! Each setting resolves on its own, in this order: code (the builder), the
//! environment (`INORBIT_*`), the config file the `iohr` command line shares, then the
//! default. [`ResolvedConfig::describe`] says which source every value came from.
//!
//! The functions here are pure apart from reading the files a configuration names, so
//! a test can resolve a configuration with [`LoadOptions`] instead of the process
//! environment.

mod no_proxy;
mod resolve;

use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use serde_json::{Map, Value};

pub(crate) use no_proxy::ProxyRules;
pub(crate) use resolve::{Inputs, ProblemsText, Resolved, program_found, resolve, show_duration};
pub use resolve::{Os, PIPELINE};

use crate::error::ConfigError;
use crate::secret::Secret;

/// What `load` reads instead of the process, for tests and tools: the environment, the
/// operating system's conventions, the home directory and the working directory
/// (`docs/config.md` section 9.2). Anything not set is the process's own.
///
/// ```
/// use inorbithr::config::{LoadOptions, Os};
///
/// let options = LoadOptions::new()
///     .env([("INORBIT_TOKEN", "t"), ("INORBIT_CONFIG_FILE", "off")])
///     .os(Os::Linux)
///     .home(None::<&str>);
/// # let _ = options;
/// ```
#[derive(Clone, Default)]
pub struct LoadOptions {
    env: Option<BTreeMap<String, String>>,
    os: Option<Os>,
    home: Option<Option<String>>,
    cwd: Option<String>,
}

/// Names the variables, never their values: the environment may hold secrets.
impl fmt::Debug for LoadOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LoadOptions")
            .field("env", &self.env.as_ref().map(|e| e.keys().collect::<Vec<_>>()))
            .field("os", &self.os)
            .field("home", &self.home)
            .field("cwd", &self.cwd)
            .finish()
    }
}

impl LoadOptions {
    /// Everything from the process.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The environment `load` sees, in place of the process's. An empty value is unset.
    #[must_use]
    pub fn env<K: Into<String>, V: Into<String>>(
        mut self,
        env: impl IntoIterator<Item = (K, V)>,
    ) -> Self {
        self.env = Some(
            env.into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
        );
        self
    }

    /// Whose conventions apply to the config file's location and to paths.
    #[must_use]
    pub fn os(mut self, os: Os) -> Self {
        self.os = Some(os);
        self
    }

    /// The home directory, or `None` for none (a sandbox without one).
    #[must_use]
    pub fn home(mut self, home: Option<impl Into<PathBuf>>) -> Self {
        self.home = Some(home.map(|h| h.into().to_string_lossy().into_owned()));
        self
    }

    /// The directory relative paths in the environment and in code resolve against.
    #[must_use]
    pub fn cwd(mut self, cwd: impl Into<PathBuf>) -> Self {
        self.cwd = Some(cwd.into().to_string_lossy().into_owned());
        self
    }

    pub(crate) fn env_map(&self) -> BTreeMap<String, String> {
        self.env
            .clone()
            .unwrap_or_else(|| std::env::vars().collect())
    }

    pub(crate) fn os_value(&self) -> Os {
        self.os.unwrap_or_else(Os::current)
    }

    pub(crate) fn home_dir(&self) -> Option<String> {
        match &self.home {
            Some(h) => h.clone(),
            None => home_from_process(),
        }
    }

    pub(crate) fn cwd_dir(&self) -> String {
        self.cwd.clone().unwrap_or_else(|| {
            std::env::current_dir()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|_| ".".to_owned())
        })
    }
}

/// The process's home directory, the way the command line finds it.
fn home_from_process() -> Option<String> {
    let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    if cfg!(windows) {
        var("USERPROFILE")
    } else {
        var("HOME")
    }
}

/// The configuration a client resolved to: [`describe`](Self::describe) gives the
/// document `docs/config.md` section 2.6 defines, the same one `iohr sdk config`
/// prints. Secrets in it are always `<redacted>`.
#[derive(Clone)]
pub struct ResolvedConfig {
    describe: Value,
}

impl fmt::Debug for ResolvedConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.describe.to_string())
    }
}

impl ResolvedConfig {
    pub(crate) fn new(describe: Value) -> Self {
        Self { describe }
    }

    /// The effective configuration as JSON: the profile and where it was chosen, the
    /// config file read, every setting with a value and its source, the credential and
    /// the sources tried, the pipeline by name, and what was read but not used.
    #[must_use]
    pub fn describe(&self) -> Value {
        self.describe.clone()
    }

    pub(crate) fn set_pipeline(&mut self, names: Vec<String>) {
        self.describe["pipeline"] = Value::from(names);
    }

    pub(crate) fn push_ignored(&mut self, entry: Value) {
        if let Some(a) = self.describe["ignored"].as_array_mut() {
            a.push(entry);
        }
    }

    /// The settings table of the description.
    pub(crate) fn settings(&self) -> &Map<String, Value> {
        static EMPTY: std::sync::OnceLock<Map<String, Value>> = std::sync::OnceLock::new();
        self.describe["settings"]
            .as_object()
            .unwrap_or_else(|| EMPTY.get_or_init(Map::new))
    }

    /// The profile's name, when one was chosen.
    pub(crate) fn profile(&self) -> Option<&str> {
        self.describe["profile"]["name"].as_str()
    }
}

/// Resolves the configuration for the public client (`profile` standing for `profile`
/// in code) or for a typed profile named `profile_type`, as
/// [`Client::load`](crate::Client::load) would, without building a client. This is
/// what `iohr sdk config` prints.
///
/// # Errors
///
/// [`ConfigError::Invalid`] with every problem `load` would report.
pub fn resolve_for(
    options: &LoadOptions,
    profile: Option<&str>,
    profile_type: Option<&str>,
) -> Result<ResolvedConfig, ConfigError> {
    let mut code = Map::new();
    if let Some(p) = profile {
        code.insert("profile".into(), Value::String(p.to_owned()));
    }
    run(options, code, profile_type).map(|r| ResolvedConfig::new(r.describe))
}

/// Runs the resolver over `options` and the code map.
pub(crate) fn run(
    options: &LoadOptions,
    code: Map<String, Value>,
    profile_type: Option<&str>,
) -> Result<Resolved, ConfigError> {
    let env = options.env_map();
    let path_env = env.clone();
    let cli_found = move |program: &str| program_found(program, &path_env);
    let read = |p: &str| std::fs::read(p).ok();
    let inputs = Inputs {
        env: &env,
        os: options.os_value(),
        home: options.home_dir(),
        cwd: options.cwd_dir(),
        code,
        profile_type: profile_type.map(str::to_owned),
        cli_found: &cli_found,
        read: &read,
    };
    resolve(&inputs).map_err(Into::into)
}

/// The config file `load` would read with `options` (`docs/config.md` section 4.1),
/// `code` standing for `config_file` in code; `None` reads no file (`off`, or no home
/// directory). A path named by code or `INORBIT_CONFIG_FILE` is returned whether or not
/// it exists.
#[must_use]
pub fn config_file_path(options: &LoadOptions, code: Option<&str>) -> Option<String> {
    resolve::config_path(
        options.os_value(),
        &options.env_map(),
        options.home_dir().as_deref(),
        code,
    )
    .map(|(p, _, _)| p)
}

/// Reads a duration as the environment and the config file write it: digits, then
/// `ms`, `s`, `m` or `h`, greater than zero (`docs/config.md` section 2.4).
#[must_use]
pub fn parse_duration(text: &str) -> Option<Duration> {
    resolve::parse_duration(text).map(Duration::from_millis)
}

/// The proxy a request to `url` goes through with `options`' environment and the
/// `proxy` and `no_proxy` given in code: its URL, or `None` for a direct connection
/// (`docs/config.md` section 6.2). The config file is not read.
///
/// # Errors
///
/// [`ConfigError::Invalid`] when a proxy URL or a `no_proxy` entry cannot be used.
pub fn proxy_for(
    url: &str,
    options: &LoadOptions,
    code_proxy: Option<&str>,
    code_no_proxy: Option<&[&str]>,
) -> Result<Option<String>, ConfigError> {
    let env = options.env_map();
    let var = |k: &str| env.get(k).map(String::as_str).filter(|v| !v.is_empty());
    let (proxy, explicit, proxy_source) = if let Some(p) = code_proxy {
        (Some(p.to_owned()), true, "code".to_owned())
    } else if let Some(p) = var("INORBIT_PROXY") {
        (Some(p.to_owned()), true, "env INORBIT_PROXY".to_owned())
    } else if let Some(p) = var("https_proxy") {
        (Some(p.to_owned()), false, "env https_proxy".to_owned())
    } else if let Some(p) = var("HTTPS_PROXY") {
        (Some(p.to_owned()), false, "env HTTPS_PROXY".to_owned())
    } else {
        (None, false, String::new())
    };
    let (no_proxy, no_proxy_source): (Vec<String>, String) = if let Some(n) = code_no_proxy {
        (n.iter().map(|s| (*s).to_owned()).collect(), "code".into())
    } else {
        ["INORBIT_NO_PROXY", "no_proxy", "NO_PROXY"]
            .into_iter()
            .find_map(|k| var(k).map(|v| (k, v)))
            .map(|(k, v)| {
                (
                    v.split(',')
                        .map(str::trim)
                        .filter(|x| !x.is_empty())
                        .map(str::to_owned)
                        .collect(),
                    format!("env {k}"),
                )
            })
            .unwrap_or_default()
    };
    let proxy = proxy.filter(|p| p != "off");
    let rules = ProxyRules::new(proxy.as_deref(), explicit, &no_proxy).map_err(
        |(which, message)| ConfigError::Invalid {
            problems: vec![crate::error::Problem {
                setting: which.to_owned(),
                source: if which == "proxy" {
                    proxy_source.clone()
                } else {
                    no_proxy_source.clone()
                },
                message,
            }],
        },
    )?;
    let url = url::Url::parse(url).map_err(|e| ConfigError::InvalidUrl {
        what: "url",
        reason: e.to_string(),
    })?;
    Ok(rules.proxy_for(&url).map(|u| u.to_string().trim_end_matches('/').to_owned()))
}

/// The typed settings a client runs with, from the description and the secrets.
#[derive(Clone)]
pub(crate) struct Settings {
    pub(crate) base_url: String,
    pub(crate) token_url: String,
    pub(crate) connect_timeout: Duration,
    pub(crate) timeout: Duration,
    pub(crate) total_timeout: Duration,
    pub(crate) stream_idle_timeout: Duration,
    pub(crate) max_retries: u32,
    pub(crate) retry_base_delay: Duration,
    pub(crate) retry_max_delay: Duration,
    pub(crate) retry_after_max: Duration,
    pub(crate) retry_budget: bool,
    pub(crate) streams: crate::stream::Streams,
    pub(crate) proxy: ProxyRules,
    pub(crate) ca_bundle: Option<String>,
    pub(crate) system_trust: bool,
    pub(crate) client_cert: Option<String>,
    pub(crate) client_key: Option<String>,
    pub(crate) client_key_password: Option<Secret<String>>,
    pub(crate) pinned_keys: Vec<String>,
    pub(crate) log: crate::middleware::LogLevel,
    pub(crate) log_headers: bool,
    pub(crate) log_allow_headers: Vec<String>,
    pub(crate) tracing: bool,
    pub(crate) metrics: bool,
    pub(crate) rate_limit: crate::ratelimit::RateLimitMode,
    pub(crate) user_agent_suffix: Option<String>,
}

impl Settings {
    /// Reads the typed settings from a resolved description. A value missing from it
    /// takes the catalogue's default.
    pub(crate) fn from_resolved(
        config: &ResolvedConfig,
        secrets: &BTreeMap<String, Secret<String>>,
    ) -> Result<Self, ConfigError> {
        let s = config.settings();
        let value = |k: &str| s.get(k).map(|e| &e["value"]);
        let text = |k: &str| value(k).and_then(Value::as_str).map(str::to_owned);
        let source = |k: &str| {
            s.get(k)
                .and_then(|e| e["source"].as_str())
                .unwrap_or_default()
                .to_owned()
        };
        let dur = |k: &str, d: u64| {
            text(k)
                .and_then(|t| resolve::parse_duration(&t))
                .map_or(Duration::from_millis(d), Duration::from_millis)
        };
        let flag = |k: &str, d: bool| value(k).and_then(Value::as_bool).unwrap_or(d);
        let list = |k: &str| -> Vec<String> {
            value(k)
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default()
        };
        let proxy_text = match text("proxy").as_deref() {
            None | Some("off") => None,
            Some(shown) => Some(
                secrets
                    .get("proxy")
                    .map_or_else(|| shown.to_owned(), |p| p.expose().clone()),
            ),
        };
        let proxy_source = source("proxy");
        let explicit = proxy_source == "code"
            || proxy_source.starts_with("env INORBIT_")
            || proxy_source.starts_with("file ");
        let no_proxy = list("no_proxy");
        let proxy = ProxyRules::new(proxy_text.as_deref(), explicit, &no_proxy).map_err(
            |(which, message)| ConfigError::Invalid {
                problems: vec![crate::error::Problem {
                    setting: which.to_owned(),
                    source: source(which),
                    message,
                }],
            },
        )?;
        let otel = cfg!(feature = "otel");
        Ok(Self {
            base_url: text("base_url").unwrap_or_else(|| crate::DEFAULT_BASE_URL.to_owned()),
            token_url: text("token_url").unwrap_or_else(|| crate::DEFAULT_TOKEN_URL.to_owned()),
            connect_timeout: dur("connect_timeout", 10_000),
            timeout: dur("timeout", 30_000),
            total_timeout: dur("total_timeout", 120_000),
            stream_idle_timeout: dur("stream_idle_timeout", 45_000),
            max_retries: value("max_retries")
                .and_then(Value::as_u64)
                .and_then(|n| u32::try_from(n).ok())
                .unwrap_or(2),
            retry_base_delay: dur("retry_base_delay", 500),
            retry_max_delay: dur("retry_max_delay", 8_000),
            retry_after_max: dur("retry_after_max", 60_000),
            retry_budget: flag("retry_budget", true),
            streams: if text("streams").as_deref() == Some("socket") {
                crate::stream::Streams::Socket
            } else {
                crate::stream::Streams::Sse
            },
            proxy,
            ca_bundle: text("ca_bundle"),
            system_trust: flag("system_trust", true),
            client_cert: text("client_cert"),
            client_key: text("client_key"),
            client_key_password: secrets.get("client_key_password").cloned(),
            pinned_keys: list("pinned_keys"),
            log: text("log")
                .and_then(|l| l.parse().ok())
                .unwrap_or_default(),
            log_headers: flag("log_headers", false),
            log_allow_headers: list("log_allow_headers"),
            tracing: flag("tracing", otel),
            metrics: value("metrics")
                .and_then(Value::as_bool)
                .unwrap_or_else(|| flag("tracing", otel)),
            rate_limit: text("rate_limit")
                .and_then(|r| r.parse().ok())
                .unwrap_or_default(),
            user_agent_suffix: text("user_agent_suffix"),
        })
    }
}
