//! Configuration resolution (`docs/config.md` sections 2 to 5): what a client built
//! with `load` sees, as `describe()` gives it. Pure: the environment, the OS, the home
//! directory and the code options are inputs, so the conformance vectors
//! (`conformance/vectors/config`) run against it, and `iohr sdk config` prints it.
//!
//! This resolves and validates; it never contacts a host. Secret values are kept apart
//! from the description (which only ever says `<redacted>`), for the client to use.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use base64::Engine as _;
use serde_json::{Map, Value, json};

use crate::secret::Secret;

/// The built-in pipeline, outermost first (`docs/config.md` section 7.2).
pub const PIPELINE: [&str; 12] = [
    "request_id",
    "user_agent",
    "idempotency_key",
    "call_tracing",
    "deadline",
    "retry",
    "auth",
    "rate_limit",
    "attempt_tracing",
    "logging",
    "hooks",
    "timeout",
];

/// The largest config file read (section 4.1).
const MAX_FILE: u64 = 1024 * 1024;

const REDACTED: &str = "<redacted>";

/// The keys of a profile table the command line owns (section 4.2).
const CLI_KEYS: [&str; 5] = ["kind", "account", "storage", "issuer", "client_id"];

const SOURCES: [&str; 4] = ["env", "workload", "file", "cli"];

/// The operating system whose conventions apply to the config file's location and to
/// paths (`docs/config.md` section 4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Os {
    /// Linux and other Unix systems.
    Linux,
    /// macOS.
    Macos,
    /// Windows.
    Windows,
}

impl Os {
    /// The OS this program runs on.
    #[must_use]
    pub fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::Macos
        } else if cfg!(windows) {
            Self::Windows
        } else {
            Self::Linux
        }
    }

    fn sep(self) -> char {
        if self == Self::Windows { '\\' } else { '/' }
    }

    /// Absolute by the OS's rule, or on the host this runs on (a vector for Linux run
    /// on Windows hands in `C:\...` paths).
    fn is_absolute(self, p: &str) -> bool {
        if std::path::Path::new(p).is_absolute() {
            return true;
        }
        match self {
            Self::Windows => {
                let b = p.as_bytes();
                p.starts_with("\\\\")
                    || p.starts_with('\\')
                    || p.starts_with('/')
                    || (b.len() >= 3
                        && b[0].is_ascii_alphabetic()
                        && b[1] == b':'
                        && (b[2] == b'\\' || b[2] == b'/'))
            }
            _ => p.starts_with('/'),
        }
    }

    fn join(self, dir: &str, rest: &str) -> String {
        let sep = self.sep();
        let dir = dir.trim_end_matches(['/', '\\']);
        format!("{dir}{sep}{rest}")
    }

    fn parent(self, path: &str) -> String {
        match path.rfind(['/', '\\']) {
            Some(0) => path[..1].to_owned(),
            Some(i) => path[..i].to_owned(),
            None => ".".to_owned(),
        }
        .replace(
            if self == Self::Windows { '/' } else { '\\' },
            &self.sep().to_string(),
        )
    }
}

/// What resolution reads.
pub(crate) struct Inputs<'a> {
    /// The environment; an empty value is unset.
    pub(crate) env: &'a BTreeMap<String, String>,
    pub(crate) os: Os,
    /// The home directory, when there is one.
    pub(crate) home: Option<String>,
    /// The working directory, for relative paths in the environment.
    pub(crate) cwd: String,
    /// Options set in code, by catalogue name (durations as strings, lists as arrays;
    /// `http_client: "custom"` stands for a caller-supplied client).
    pub(crate) code: Map<String, Value>,
    /// Resolve for a typed (generated) profile of this name.
    pub(crate) profile_type: Option<String>,
    /// Whether the `cli` source's program can be run, given `cli_path` (or `iohr`).
    pub(crate) cli_found: &'a dyn Fn(&str) -> bool,
    /// Reads a file: `None` when it does not exist or cannot be read.
    pub(crate) read: &'a dyn Fn(&str) -> Option<Vec<u8>>,
}

use crate::error::Problem;

/// `load` would fail: every problem, in catalogue order.
#[derive(Debug)]
pub(crate) struct Invalid {
    pub(crate) problems: Vec<Problem>,
}

impl From<Invalid> for crate::error::ConfigError {
    fn from(e: Invalid) -> Self {
        Self::Invalid {
            problems: e.problems,
        }
    }
}

/// The text of a `ConfigError` holding `problems` (`docs/config.md` section 2.5).
pub(crate) struct ProblemsText<'a>(pub(crate) &'a [Problem]);

impl std::fmt::Display for ProblemsText<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let problems = self.0;
        let n = problems.len();
        // No credentials and nothing else: the chain's own message (section 5.1).
        if let [only] = problems
            && only.setting == "credential"
        {
            return f.write_str(&only.message);
        }
        writeln!(
            f,
            "configuration is invalid ({n} problem{}):",
            if n == 1 { "" } else { "s" }
        )?;
        for (i, p) in problems.iter().enumerate() {
            let mut lines = p.message.lines();
            write!(f, "  {}: {}", p.setting, lines.next().unwrap_or_default())?;
            if !p.source.is_empty() {
                write!(f, " (from {})", p.source)?;
            }
            for l in lines {
                write!(f, "\n    {l}")?;
            }
            if i + 1 < n {
                writeln!(f)?;
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ty {
    Duration,
    Int,
    Bool,
    /// `scopes`: spaces in the environment.
    Scopes,
    /// Commas in the environment.
    List,
    Url,
    Path,
    Secret,
    Str,
    Enum(&'static [&'static str]),
    Proxy,
    Pins,
    Reserved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InFile {
    Yes,
    Never,
}

struct Setting {
    name: &'static str,
    ty: Ty,
    file: InFile,
    /// A credential piece: read only with a typed profile's prefix, chosen whole.
    credential: bool,
    /// Belongs to the caller's HTTP client when one is supplied (section 6.6).
    transport: bool,
    default: Option<fn() -> Value>,
}

const fn s(name: &'static str, ty: Ty) -> Setting {
    Setting {
        name,
        ty,
        file: InFile::Yes,
        credential: false,
        transport: false,
        default: None,
    }
}

const fn cred(name: &'static str, ty: Ty, file: InFile) -> Setting {
    Setting {
        name,
        ty,
        file,
        credential: true,
        transport: false,
        default: None,
    }
}

const fn net(name: &'static str, ty: Ty) -> Setting {
    Setting {
        name,
        ty,
        file: InFile::Yes,
        credential: false,
        transport: true,
        default: None,
    }
}

const fn d(mut x: Setting, f: fn() -> Value) -> Setting {
    x.default = Some(f);
    x
}

/// The catalogue (section 3), in its order; problems are reported in this order.
/// `profile` and `config_file` come first and are resolved on their own.
const CATALOGUE: &[Setting] = &[
    d(s("base_url", Ty::Url), || json!("https://api.inorbit.hr")),
    d(s("token_url", Ty::Url), || {
        json!("https://auth.inorbit.hr/oauth2/token")
    }),
    s("region", Ty::Reserved),
    cred("key_id", Ty::Str, InFile::Yes),
    cred("key_secret", Ty::Secret, InFile::Never),
    cred("key_secret_file", Ty::Path, InFile::Yes),
    s("scopes", Ty::Scopes),
    cred("token", Ty::Secret, InFile::Never),
    cred("token_file", Ty::Path, InFile::Yes),
    d(s("credential_sources", Ty::List), || json!(SOURCES)),
    s("cli_path", Ty::Path),
    d(net("connect_timeout", Ty::Duration), || json!("10s")),
    d(s("timeout", Ty::Duration), || json!("30s")),
    d(s("total_timeout", Ty::Duration), || json!("120s")),
    d(s("stream_idle_timeout", Ty::Duration), || json!("45s")),
    d(s("max_retries", Ty::Int), || json!(2)),
    d(s("retry_base_delay", Ty::Duration), || json!("500ms")),
    d(s("retry_max_delay", Ty::Duration), || json!("8s")),
    d(s("retry_after_max", Ty::Duration), || json!("60s")),
    d(s("retry_budget", Ty::Bool), || json!(true)),
    d(s("streams", Ty::Enum(&["sse", "socket"])), || json!("sse")),
    net("proxy", Ty::Proxy),
    net("no_proxy", Ty::List),
    net("ca_bundle", Ty::Path),
    d(net("system_trust", Ty::Bool), || json!(true)),
    net("client_cert", Ty::Path),
    net("client_key", Ty::Path),
    Setting {
        name: "client_key_password",
        ty: Ty::Secret,
        file: InFile::Never,
        credential: false,
        transport: true,
        default: None,
    },
    net("pinned_keys", Ty::Pins),
    d(
        s("log", Ty::Enum(&["off", "error", "warn", "info", "debug"])),
        || json!("off"),
    ),
    d(s("log_headers", Ty::Bool), || json!(false)),
    s("log_allow_headers", Ty::List),
    s("tracing", Ty::Bool),
    s("metrics", Ty::Bool),
    d(
        s("rate_limit", Ty::Enum(&["observe", "wait", "off"])),
        || json!("observe"),
    ),
    s("user_agent_suffix", Ty::Str),
];

fn order(setting: &str) -> usize {
    match setting {
        "profile" => 0,
        "config_file" => 1,
        "credential" => usize::MAX,
        other => CATALOGUE
            .iter()
            .position(|s| s.name == other)
            .map_or(usize::MAX - 1, |i| i + 2),
    }
}

/// A table of the config file that was read.
struct Layer<'t> {
    table: &'t toml::Table,
    label: String,
    /// The profile's own table, where the command line's keys are not SDK keys.
    profile: bool,
}

struct Resolver<'a> {
    inp: &'a Inputs<'a>,
    /// `INORBIT_<P>_` for a typed profile.
    prefix: Option<String>,
    file_path: Option<String>,
    file_dir: String,
    layers: Vec<Layer<'a>>,
    problems: Vec<Problem>,
    settings: Map<String, Value>,
    ignored: Vec<Value>,
    /// Secret values by setting, never in `settings`.
    secrets: BTreeMap<String, Secret<String>>,
}

/// Parses a duration: digits, then `ms`, `s`, `m` or `h`, greater than zero. The value
/// is in milliseconds.
pub(crate) fn parse_duration(v: &str) -> Option<u64> {
    let unit_at = v.find(|c: char| !c.is_ascii_digit())?;
    let (digits, unit) = v.split_at(unit_at);
    if digits.is_empty() {
        return None;
    }
    let n: u64 = digits.parse().ok()?;
    let ms = match unit {
        "ms" => n,
        "s" => n.checked_mul(1000)?,
        "m" => n.checked_mul(60_000)?,
        "h" => n.checked_mul(3_600_000)?,
        _ => return None,
    };
    (ms > 0).then_some(ms)
}

pub(crate) fn show_duration(ms: u64) -> String {
    if ms.is_multiple_of(1000) {
        format!("{}s", ms / 1000)
    } else {
        format!("{ms}ms")
    }
}

pub(crate) fn is_loopback(host: &str) -> bool {
    let h = host.trim_start_matches('[').trim_end_matches(']');
    h.eq_ignore_ascii_case("localhost")
        || h.parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// A URL with its user-info replaced by `<redacted>`.
pub(crate) fn redact_userinfo(raw: &str) -> String {
    let Some((scheme, rest)) = raw.split_once("://") else {
        return raw.to_owned();
    };
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    match rest[..end].rfind('@') {
        Some(at) => format!("{scheme}://{REDACTED}@{}", &rest[at + 1..]),
        None => raw.to_owned(),
    }
}

fn has_userinfo(raw: &str) -> bool {
    url::Url::parse(raw).is_ok_and(|u| !u.username().is_empty() || u.password().is_some())
}

/// The config file's path as the SDKs find it (section 4.1): `None` reads no file.
/// The flag says whether the path was named (code or `INORBIT_CONFIG_FILE`), and the
/// label where it came from.
pub(crate) fn config_path(
    os: Os,
    env: &BTreeMap<String, String>,
    home: Option<&str>,
    code: Option<&str>,
) -> Option<(String, bool, String)> {
    let var = |k: &str| env.get(k).map(String::as_str).filter(|v| !v.is_empty());
    if let Some(c) = code {
        return (c != "off").then(|| (c.to_owned(), true, "code".to_owned()));
    }
    if let Some(v) = var("INORBIT_CONFIG_FILE") {
        return (v != "off").then(|| (v.to_owned(), true, "env INORBIT_CONFIG_FILE".to_owned()));
    }
    if let Some(dir) = var("IOHR_CONFIG_DIR") {
        return Some((
            os.join(dir, "config.toml"),
            false,
            "env IOHR_CONFIG_DIR".into(),
        ));
    }
    let default = |p: String| Some((p, false, "default".to_owned()));
    match os {
        Os::Linux => {
            let xdg = var("XDG_CONFIG_HOME").filter(|x| os.is_absolute(x));
            match (xdg, home) {
                (Some(x), _) => default(os.join(x, "iohr/config.toml")),
                (None, Some(h)) => default(os.join(h, ".config/iohr/config.toml")),
                (None, None) => None,
            }
        }
        Os::Macos => home.and_then(|h| {
            default(os.join(h, "Library/Application Support/hr.InOrbit.iohr/config.toml"))
        }),
        Os::Windows => {
            var("APPDATA").and_then(|a| default(os.join(a, "InOrbit\\iohr\\config\\config.toml")))
        }
    }
}

fn env_name(profile: &str) -> String {
    profile
        .chars()
        .map(|c| match c {
            '-' => '_',
            c => c.to_ascii_uppercase(),
        })
        .collect()
}

/// A profile name the command line would accept (section 2.3): 1 to 64 characters from
/// `a-z`, `0-9`, `_` and `-`, starting with a letter or digit.
pub(crate) fn valid_profile(name: &str) -> bool {
    let b = name.as_bytes();
    !b.is_empty()
        && b.len() <= 64
        && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit())
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'_' || *c == b'-')
}

/// What resolution produced: the `describe()` document and, apart from it, the secret
/// values it names as `<redacted>` (`token`, `key_secret`, `client_key_password`, and a
/// `proxy` URL with its user-info).
pub(crate) struct Resolved {
    pub(crate) describe: Value,
    pub(crate) secrets: BTreeMap<String, Secret<String>>,
}

/// Resolves the configuration. `Ok` holds the `describe()` document.
#[allow(
    clippy::too_many_lines,
    reason = "the file, the profile, then the layers in order"
)]
pub(crate) fn resolve(inp: &Inputs<'_>) -> Result<Resolved, Invalid> {
    let mut problems = Vec::new();
    let var = |k: &str| inp.env.get(k).map(String::as_str).filter(|v| !v.is_empty());

    // The file.
    let code_file = inp.code.get("config_file").and_then(Value::as_str);
    let located = config_path(inp.os, inp.env, inp.home.as_deref(), code_file);
    let mut file_path = None;
    let mut doc: Option<toml::Table> = None;
    if let Some((path, named, label)) = located {
        let abs = if inp.os.is_absolute(&path) {
            path.clone()
        } else {
            inp.os.join(&inp.cwd, &path)
        };
        match (inp.read)(&abs) {
            Some(bytes) if bytes.len() as u64 > MAX_FILE => problems.push(Problem {
                setting: "config_file".into(),
                source: label,
                message: format!("{abs} is larger than 1 MiB"),
            }),
            Some(bytes) => match std::str::from_utf8(&bytes)
                .map_err(|_| "it is not UTF-8".to_owned())
                .and_then(|t| {
                    t.parse::<toml::Table>().map_err(|e| {
                        // The message only: the error's snippet could quote a secret.
                        e.message().to_owned()
                    })
                }) {
                Ok(t) => {
                    doc = Some(t);
                    file_path = Some(abs);
                }
                Err(m) => problems.push(Problem {
                    setting: "config_file".into(),
                    source: label,
                    message: format!("{abs} is not valid TOML: {m}"),
                }),
            },
            None if named => problems.push(Problem {
                setting: "config_file".into(),
                source: label,
                message: format!("there is no readable file at {abs}"),
            }),
            None => {}
        }
    }
    let empty = toml::Table::new();
    let doc_ref = doc.as_ref().unwrap_or(&empty);
    let file_label = |suffix: &str| match &file_path {
        Some(p) if suffix.is_empty() => format!("file {p}"),
        Some(p) => format!("file {p} [{suffix}]"),
        None => String::new(),
    };

    // The profile.
    let mut profile: Option<(String, String)> = None;
    let typed = inp.profile_type.clone();
    if let Some(t) = &typed {
        profile = Some((t.clone(), "code".into()));
    } else {
        let chosen = inp
            .code
            .get("profile")
            .and_then(Value::as_str)
            .map(|p| (p.to_owned(), "code".to_owned(), true))
            .or_else(|| {
                var("INORBIT_PROFILE")
                    .map(|p| (p.to_owned(), "env INORBIT_PROFILE".to_owned(), true))
            })
            .or_else(|| {
                doc_ref
                    .get("default")
                    .and_then(toml::Value::as_str)
                    .map(|p| (p.to_owned(), file_label(""), false))
            });
        if let Some((name, source, _)) = chosen {
            let has = doc_ref
                .get("profiles")
                .and_then(toml::Value::as_table)
                .is_some_and(|t| t.contains_key(&name));
            if !valid_profile(&name) {
                problems.push(Problem {
                    setting: "profile".into(),
                    source,
                    message: format!(
                        "{name:?} is not a profile name: 1 to 64 lower-case letters, digits, '-' or '_', starting with a letter or digit"
                    ),
                });
            } else if !has {
                let where_ = file_path.as_deref().map_or_else(
                    || "no config file was read".to_owned(),
                    |p| format!("{p} has no [profiles.{name}]"),
                );
                problems.push(Problem {
                    setting: "profile".into(),
                    source,
                    message: format!("there is no profile {name:?}: {where_}; `iohr profile list` shows the profiles"),
                });
            } else {
                profile = Some((name, source));
            }
        }
    }
    let profile_table = profile.as_ref().and_then(|(n, _)| {
        doc_ref
            .get("profiles")
            .and_then(toml::Value::as_table)
            .and_then(|t| t.get(n))
            .and_then(toml::Value::as_table)
    });

    let mut layers = Vec::new();
    if let (Some(t), Some((n, _))) = (profile_table, &profile) {
        layers.push(Layer {
            table: t,
            label: file_label(&format!("profiles.{n}")),
            profile: true,
        });
    }
    if let Some(t) = doc_ref.get("sdk").and_then(toml::Value::as_table) {
        layers.push(Layer {
            table: t,
            label: file_label("sdk"),
            profile: false,
        });
    }

    let file_dir = file_path
        .as_deref()
        .map_or_else(|| inp.cwd.clone(), |p| inp.os.parent(p));
    let mut r = Resolver {
        inp,
        prefix: typed
            .as_deref()
            .map(|t| format!("INORBIT_{}_", env_name(t))),
        file_path: file_path.clone(),
        file_dir,
        layers,
        problems,
        settings: Map::new(),
        ignored: Vec::new(),
        secrets: BTreeMap::new(),
    };
    r.check_file_keys();
    r.resolve_settings();
    let credential = r.chain(profile.as_ref().map(|(n, _)| n.as_str()), profile_table);
    r.cross_checks();

    let mut problems = std::mem::take(&mut r.problems);
    if !problems.is_empty() {
        problems.sort_by_key(|p| order(&p.setting));
        return Err(Invalid { problems });
    }
    let credential = credential.unwrap_or(Value::Null);
    let secrets = std::mem::take(&mut r.secrets);
    Ok(Resolved {
        describe: json!({
            "profile": profile.map_or(Value::Null, |(name, source)| json!({ "name": name, "source": source })),
            "config_file": file_path,
            "settings": r.settings,
            "credential": credential,
            "pipeline": PIPELINE,
            "ignored": r.ignored,
        }),
        secrets,
    })
}

impl Resolver<'_> {
    fn var(&self, k: &str) -> Option<&str> {
        self.inp
            .env
            .get(k)
            .map(String::as_str)
            .filter(|v| !v.is_empty())
    }

    fn problem(&mut self, setting: &str, source: &str, message: impl Into<String>) {
        self.problems.push(Problem {
            setting: setting.to_owned(),
            source: source.to_owned(),
            message: message.into(),
        });
    }

    fn http_client(&self) -> bool {
        self.inp.code.contains_key("http_client")
    }

    /// Secrets, a proxy password and unknown keys in the tables read.
    fn check_file_keys(&mut self) {
        let mut found = Vec::new();
        for layer in &self.layers {
            for (k, v) in layer.table {
                if layer.profile && CLI_KEYS.contains(&k.as_str()) {
                    continue;
                }
                found.push((k.clone(), v.clone(), layer.label.clone()));
            }
        }
        for (k, v, label) in found {
            match CATALOGUE.iter().find(|s| s.name == k) {
                Some(s) if s.file == InFile::Never => {
                    let way_out = match s.name {
                        "key_secret" => "key_secret_file, the environment, or iohr login",
                        "token" => "token_file, the environment, or iohr login",
                        _ => "the environment or code",
                    };
                    self.problem(
                        &k,
                        &label,
                        format!("secrets are not allowed in the config file; use {way_out}"),
                    );
                }
                Some(s) if s.ty == Ty::Proxy && v.as_str().is_some_and(has_userinfo) => {
                    self.problem(
                        &k,
                        &label,
                        "a proxy URL with a user name or password holds a secret, which is not allowed in the config file; set it in INORBIT_PROXY or in code",
                    );
                }
                Some(_) => {}
                None if k == "profile" || k == "config_file" => self.ignored.push(json!({
                    "key": k, "source": label, "reason": "not read from the config file",
                })),
                None => self.ignored.push(json!({
                    "key": k, "source": label, "reason": "unknown key",
                })),
            }
        }
    }

    /// The environment variables for a setting, in order, before the file.
    fn env_names(&self, s: &Setting) -> Vec<String> {
        let upper = s.name.to_ascii_uppercase();
        let mut names = Vec::new();
        if let Some(p) = &self.prefix {
            names.push(format!("{p}{upper}"));
        }
        if !(s.credential && self.prefix.is_some()) {
            names.push(format!("INORBIT_{upper}"));
        }
        names
    }

    /// The standard variables read after the file (section 6.2).
    fn env_after_file(s: &Setting) -> &'static [&'static str] {
        match s.name {
            "proxy" => &["https_proxy", "HTTPS_PROXY"],
            "no_proxy" => &["no_proxy", "NO_PROXY"],
            _ => &[],
        }
    }

    /// The raw value and its source for a non-credential setting.
    fn raw(&self, s: &Setting) -> Option<(Raw, String)> {
        if let Some(v) = self.inp.code.get(s.name) {
            return Some((Raw::Code(v.clone()), "code".into()));
        }
        for n in self.env_names(s) {
            if let Some(v) = self.var(&n) {
                return Some((Raw::Env(v.to_owned()), format!("env {n}")));
            }
        }
        if s.file == InFile::Yes {
            for l in &self.layers {
                if let Some(v) = l.table.get(s.name) {
                    return Some((Raw::File(v.clone()), l.label.clone()));
                }
            }
        }
        for n in Self::env_after_file(s) {
            if let Some(v) = self.var(n) {
                return Some((Raw::Env(v.to_owned()), format!("env {n}")));
            }
        }
        None
    }

    fn resolve_settings(&mut self) {
        for s in CATALOGUE {
            if s.credential {
                continue;
            }
            let Some((raw, source)) = self.raw(s) else {
                if let Some(f) = s.default
                    && !(s.transport && self.http_client())
                {
                    self.settings
                        .insert(s.name.into(), json!({ "value": f(), "source": "default" }));
                }
                continue;
            };
            if s.transport && self.http_client() {
                if source == "code" {
                    self.problem(
                        s.name,
                        &source,
                        "configure this on your HTTP client, or leave http_client out",
                    );
                } else {
                    self.ignored.push(json!({
                        "key": s.name, "source": source, "reason": "the caller's HTTP client decides this",
                    }));
                }
                continue;
            }
            match self.parse(s, &raw, &source) {
                Ok(v) => {
                    // The value itself, apart from the description, for the client.
                    if let Raw::Env(text) | Raw::Code(Value::String(text)) = &raw
                        && (s.ty == Ty::Secret || (s.ty == Ty::Proxy && has_userinfo(text)))
                    {
                        self.secrets
                            .insert(s.name.into(), Secret::new(text.clone()));
                    }
                    self.settings
                        .insert(s.name.into(), json!({ "value": v, "source": source }));
                }
                Err(m) => self.problem(s.name, &source, m),
            }
        }
    }

    /// A path from `source`: relative to the file's directory in the file, to the
    /// working directory elsewhere; `~/` is the home directory.
    fn path(&self, p: &str, from_file: bool) -> Result<String, String> {
        let os = self.inp.os;
        if let Some(rest) = p.strip_prefix("~/") {
            return self
                .inp
                .home
                .as_deref()
                .map(|h| os.join(h, rest))
                .ok_or_else(|| format!("{p} starts with ~/ but there is no home directory"));
        }
        if os.is_absolute(p) {
            return Ok(p.to_owned());
        }
        Ok(os.join(
            if from_file {
                &self.file_dir
            } else {
                &self.inp.cwd
            },
            p,
        ))
    }

    #[allow(clippy::too_many_lines, reason = "one match over the value types")]
    fn parse(&self, s: &Setting, raw: &Raw, _source: &str) -> Result<Value, String> {
        let from_file = matches!(raw, Raw::File(_));
        let text = |what: &str| -> Result<String, String> {
            match raw {
                Raw::Env(v) | Raw::File(toml::Value::String(v)) | Raw::Code(Value::String(v)) => {
                    Ok(v.clone())
                }
                Raw::File(other) => Err(format!("must be {what}, not {}", other.type_str())),
                Raw::Code(_) => Err(format!("must be {what}")),
            }
        };
        let list = |comma: bool| -> Result<Vec<String>, String> {
            match raw {
                Raw::Env(v) => Ok(if comma {
                    v.split(',')
                        .map(str::trim)
                        .filter(|x| !x.is_empty())
                        .map(str::to_owned)
                        .collect()
                } else {
                    v.split_whitespace().map(str::to_owned).collect()
                }),
                Raw::File(toml::Value::Array(a)) => a
                    .iter()
                    .map(|x| {
                        x.as_str()
                            .map(str::to_owned)
                            .ok_or_else(|| "must be an array of strings".to_owned())
                    })
                    .collect(),
                Raw::Code(Value::Array(a)) => a
                    .iter()
                    .map(|x| {
                        x.as_str()
                            .map(str::to_owned)
                            .ok_or_else(|| "must be a list of strings".to_owned())
                    })
                    .collect(),
                Raw::File(other) => Err(format!(
                    "must be an array of strings, not {}",
                    other.type_str()
                )),
                Raw::Code(_) => Err("must be a list of strings".into()),
            }
        };
        match s.ty {
            Ty::Duration => {
                let v = text("a duration string such as \"30s\"")?;
                parse_duration(&v).map(|ms| json!(show_duration(ms))).ok_or_else(|| {
                    if parse_duration(&format!("{v}s")).is_some() {
                        format!("{v:?} is not a duration; write it with a unit, such as 30s")
                    } else {
                        format!(
                            "{v:?} is not a duration greater than zero: digits, then ms, s, m or h, such as 30s"
                        )
                    }
                })
            }
            Ty::Int => match raw {
                Raw::Env(v) => v
                    .parse::<u32>()
                    .ok()
                    .filter(|_| v.bytes().all(|b| b.is_ascii_digit()))
                    .map(|n| json!(n))
                    .ok_or_else(|| format!("{v:?} is not a whole number of 0 or more")),
                Raw::File(toml::Value::Integer(n)) => u32::try_from(*n)
                    .map(|n| json!(n))
                    .map_err(|_| "must be a whole number of 0 or more".into()),
                Raw::Code(Value::Number(n)) => n
                    .as_u64()
                    .map(|n| json!(n))
                    .ok_or_else(|| "must be a whole number of 0 or more".into()),
                Raw::File(other) => Err(format!("must be an integer, not {}", other.type_str())),
                Raw::Code(_) => Err("must be an integer".into()),
            },
            Ty::Bool => match raw {
                Raw::Env(v) => match v.to_ascii_lowercase().as_str() {
                    "true" | "1" => Ok(json!(true)),
                    "false" | "0" => Ok(json!(false)),
                    _ => Err(format!("{v:?} is not true, false, 1 or 0")),
                },
                Raw::File(toml::Value::Boolean(b)) | Raw::Code(Value::Bool(b)) => Ok(json!(b)),
                Raw::File(other) => Err(format!("must be a boolean, not {}", other.type_str())),
                Raw::Code(_) => Err("must be a boolean".into()),
            },
            Ty::Scopes => list(false).map(|l| json!(l)),
            Ty::List => {
                let l = list(true)?;
                if s.name == "credential_sources"
                    && let Some(bad) = l.iter().find(|x| !SOURCES.contains(&x.as_str()))
                {
                    return Err(format!(
                        "{bad:?} is not a credential source; use env, workload, file or cli"
                    ));
                }
                if s.name == "no_proxy" {
                    for e in &l {
                        if !no_proxy_entry(e) {
                            return Err(format!(
                                "{e:?} is not a no_proxy entry: a host, .domain, host:port, an IP address or a CIDR range"
                            ));
                        }
                    }
                }
                Ok(json!(l))
            }
            Ty::Url => {
                let v = text("a URL string")?;
                let u = url::Url::parse(&v).map_err(|_| format!("{v:?} is not an absolute URL"))?;
                let ok = u.scheme() == "https"
                    || (u.scheme() == "http" && u.host_str().is_some_and(is_loopback));
                if ok {
                    Ok(json!(v))
                } else {
                    Err(format!(
                        "{v:?} must use https (plain http is allowed only for localhost and loopback addresses)"
                    ))
                }
            }
            Ty::Path => {
                let v = text("a path string")?;
                self.path(&v, from_file).map(|p| json!(p))
            }
            Ty::Secret => match raw {
                Raw::Env(_) | Raw::Code(Value::String(_)) => Ok(json!(REDACTED)),
                _ => Err("must be a string".into()),
            },
            Ty::Str => {
                let v = text("a string")?;
                if s.name == "user_agent_suffix"
                    && (v.chars().count() > 128
                        || v.chars().any(|c| !c.is_ascii_graphic() && c != ' '))
                {
                    return Err("must be product tokens (such as myapp/1.2), at most 128 printable ASCII characters".into());
                }
                Ok(json!(v))
            }
            Ty::Enum(allowed) => {
                let v = text("a string")?;
                if allowed.contains(&v.as_str()) {
                    Ok(json!(v))
                } else {
                    Err(format!("{v:?} is not one of {}", allowed.join(", ")))
                }
            }
            Ty::Proxy => {
                let v = text("a URL string")?;
                if v == "off" {
                    return Ok(json!("off"));
                }
                let shown = redact_userinfo(&v);
                let u =
                    url::Url::parse(&v).map_err(|_| format!("{shown:?} is not an absolute URL"))?;
                if u.scheme() != "http" && u.scheme() != "https" {
                    return Err(format!(
                        "{shown:?} must be an http:// or https:// proxy URL, or off"
                    ));
                }
                Ok(json!(shown))
            }
            Ty::Pins => {
                let l = list(true)?;
                if l.len() < 2 {
                    return Err("pin at least two keys (the current one and a backup)".into());
                }
                for p in &l {
                    let ok = base64::engine::general_purpose::STANDARD
                        .decode(p)
                        .is_ok_and(|b| b.len() == 32);
                    if !ok {
                        return Err(format!("{p:?} is not a base64 SHA-256 of a public key"));
                    }
                }
                Ok(json!(l))
            }
            Ty::Reserved => {
                Err("region is reserved until the API offers regions; remove it".into())
            }
        }
    }

    fn exists(&self, path: &str) -> bool {
        (self.inp.read)(path).is_some()
    }

    fn allowed(&self, source: &str) -> bool {
        self.settings
            .get("credential_sources")
            .and_then(|s| s.get("value"))
            .and_then(Value::as_array)
            .is_none_or(|a| a.iter().any(|x| x.as_str() == Some(source)))
    }

    fn scopes_set(&self) -> bool {
        self.settings.contains_key("scopes")
    }

    fn show(&mut self, name: &str, value: Value, source: &str) {
        let mut entry = Map::new();
        entry.insert("value".into(), value);
        entry.insert("source".into(), Value::String(source.to_owned()));
        self.settings.insert(name.into(), Value::Object(entry));
    }

    /// The credential chain (section 5.1). `None` when a problem stopped it.
    #[allow(clippy::too_many_lines, reason = "the five sources in order")]
    fn chain(&mut self, profile: Option<&str>, table: Option<&toml::Table>) -> Option<Value> {
        let mut tried: Vec<Value> = Vec::new();
        let skip = |tried: &mut Vec<Value>, source: &str, reason: &str| {
            tried.push(json!({ "source": source, "result": "skipped", "reason": reason }));
        };
        let p = self.prefix.clone().unwrap_or_else(|| "INORBIT_".into());
        let mut used: Option<(&str, &str)> = None;

        // 1. Code.
        let code_kind = if self.inp.code.contains_key("token_provider") {
            Some("custom")
        } else if self.inp.code.contains_key("token") {
            Some("static_token")
        } else if self.inp.code.contains_key("key_id") || self.inp.code.contains_key("key") {
            Some("client_credentials")
        } else {
            None
        };
        if let Some(k) = code_kind {
            tried.push(json!({ "source": "code", "result": "used" }));
            used = Some(("code", k));
        } else {
            skip(&mut tried, "code", "none set");
        }

        // 2. The environment.
        if used.is_none() {
            let n = |x: &str| format!("{p}{x}");
            let token = self.var(&n("TOKEN")).map(str::to_owned);
            let token_file = self.var(&n("TOKEN_FILE")).map(str::to_owned);
            let key_id = self.var(&n("KEY_ID")).map(str::to_owned);
            let secret = self.var(&n("KEY_SECRET")).is_some();
            let secret_file = self.var(&n("KEY_SECRET_FILE")).map(str::to_owned);
            let present = token.is_some() || token_file.is_some() || key_id.is_some();
            if !self.allowed("env") {
                skip(&mut tried, "env", "not in credential_sources");
            } else if !present {
                let reason = format!(
                    "{}, {} and {} are not set",
                    n("TOKEN"),
                    n("TOKEN_FILE"),
                    n("KEY_ID")
                );
                skip(&mut tried, "env", &reason);
            } else {
                let src = |x: &str| format!("env {p}{x}");
                let forms = [
                    ("TOKEN", token.is_some()),
                    ("TOKEN_FILE", token_file.is_some()),
                    ("KEY_ID", key_id.is_some()),
                ];
                let set: Vec<&str> = forms.iter().filter(|f| f.1).map(|f| f.0).collect();
                if set.len() > 1 {
                    let setting = set[0].to_ascii_lowercase();
                    self.problem(
                        &setting,
                        &src(set[0]),
                        format!(
                            "{} are both set; set one credential",
                            set.iter().map(|x| n(x)).collect::<Vec<_>>().join(" and ")
                        ),
                    );
                    return None;
                }
                if let Some(t) = token {
                    self.secrets.insert("token".into(), Secret::new(t));
                    self.show("token", json!(REDACTED), &src("TOKEN"));
                    used = Some(("env", "static_token"));
                } else if let Some(f) = token_file {
                    let path = self.path(&f, false).unwrap_or(f);
                    if !self.exists(&path) {
                        self.problem(
                            "token_file",
                            &src("TOKEN_FILE"),
                            format!("cannot read {path}"),
                        );
                        return None;
                    }
                    self.show("token_file", json!(path), &src("TOKEN_FILE"));
                    used = Some(("env", "token_file"));
                } else if let Some(id) = key_id {
                    if secret && secret_file.is_some() {
                        self.problem(
                            "key_secret",
                            &src("KEY_SECRET"),
                            format!(
                                "{} and {} are both set; set one",
                                n("KEY_SECRET"),
                                n("KEY_SECRET_FILE")
                            ),
                        );
                        return None;
                    }
                    if !secret && secret_file.is_none() {
                        self.problem(
                            "key_secret",
                            &src("KEY_ID"),
                            format!(
                                "{} is set without {} or {}",
                                n("KEY_ID"),
                                n("KEY_SECRET"),
                                n("KEY_SECRET_FILE")
                            ),
                        );
                        return None;
                    }
                    if !self.scopes_set() {
                        self.problem(
                            "scopes",
                            &src("KEY_ID"),
                            format!(
                                "a key needs scopes: set {}SCOPES",
                                if self.prefix.is_some() {
                                    &p
                                } else {
                                    "INORBIT_"
                                }
                            ),
                        );
                        return None;
                    }
                    self.show("key_id", json!(id), &src("KEY_ID"));
                    if let Some(v) = self.var(&n("KEY_SECRET")).filter(|_| secret) {
                        let v = Secret::new(v.to_owned());
                        self.secrets.insert("key_secret".into(), v);
                        self.show("key_secret", json!(REDACTED), &src("KEY_SECRET"));
                    } else if let Some(f) = secret_file {
                        let path = self.path(&f, false).unwrap_or(f);
                        if !self.exists(&path) {
                            self.problem(
                                "key_secret_file",
                                &src("KEY_SECRET_FILE"),
                                format!("cannot read {path}"),
                            );
                            return None;
                        }
                        self.show("key_secret_file", json!(path), &src("KEY_SECRET_FILE"));
                    }
                    used = Some(("env", "client_credentials"));
                }
                if used.is_some() {
                    tried.push(json!({ "source": "env", "result": "used" }));
                }
            }
        }

        // 3. Workload identity: reserved.
        if used.is_none() {
            let reason = if self.allowed("workload") {
                "not offered by the platform yet"
            } else {
                "not in credential_sources"
            };
            skip(&mut tried, "workload", reason);
        }

        // 4. The config file's profile table.
        if used.is_none() {
            let label = self
                .layers
                .first()
                .map(|l| l.label.clone())
                .unwrap_or_default();
            if !self.allowed("file") {
                skip(&mut tried, "file", "not in credential_sources");
            } else if self.file_path.is_none() {
                skip(&mut tried, "file", "no config file was read");
            } else if profile.is_none() {
                skip(&mut tried, "file", "no profile chosen");
            } else if let Some(t) =
                table.filter(|t| t.contains_key("token_file") || t.contains_key("key_id"))
            {
                let has_secret = t.contains_key("key_secret");
                let as_str = |k: &str| t.get(k).and_then(toml::Value::as_str).map(str::to_owned);
                if t.contains_key("token_file") && t.contains_key("key_id") {
                    self.problem(
                        "token_file",
                        &label,
                        "token_file and key_id are both set; set one credential",
                    );
                    return None;
                }
                if let Some(f) = as_str("token_file") {
                    let path = self.path(&f, true).unwrap_or(f);
                    if !self.exists(&path) {
                        self.problem("token_file", &label, format!("cannot read {path}"));
                        return None;
                    }
                    self.show("token_file", json!(path), &label);
                    used = Some(("file", "token_file"));
                } else if let Some(id) = as_str("key_id") {
                    if has_secret {
                        // Already reported: secrets are not allowed in the file.
                        return None;
                    }
                    let Some(f) = as_str("key_secret_file") else {
                        self.problem(
                            "key_secret",
                            &label,
                            "key_id is set without key_secret_file",
                        );
                        return None;
                    };
                    if !self.scopes_set() {
                        self.problem(
                            "scopes",
                            &label,
                            "a key needs scopes: set scopes in the profile's table",
                        );
                        return None;
                    }
                    let path = self.path(&f, true).unwrap_or(f);
                    if !self.exists(&path) {
                        self.problem("key_secret_file", &label, format!("cannot read {path}"));
                        return None;
                    }
                    self.show("key_id", json!(id), &label);
                    self.show("key_secret_file", json!(path), &label);
                    used = Some(("file", "client_credentials"));
                } else {
                    self.problem("token_file", &label, "must be a path string");
                    return None;
                }
                tried.push(json!({ "source": "file", "result": "used" }));
            } else {
                let reason = format!(
                    "profile {} sets no token_file or key_id",
                    profile.unwrap_or_default()
                );
                skip(&mut tried, "file", &reason);
            }
        }

        // 5. The iohr login.
        if used.is_none() {
            let program = self
                .settings
                .get("cli_path")
                .and_then(|s| s.get("value"))
                .and_then(Value::as_str)
                .unwrap_or("iohr")
                .to_owned();
            if !self.allowed("cli") {
                skip(&mut tried, "cli", "not in credential_sources");
            } else if profile.is_none() {
                skip(&mut tried, "cli", "skipped, no profile chosen");
            } else if !table.is_some_and(|t| t.contains_key("kind")) {
                let reason = format!(
                    "profile {} was not made by iohr login",
                    profile.unwrap_or_default()
                );
                skip(&mut tried, "cli", &reason);
            } else if !(self.inp.cli_found)(&program) {
                let reason = if program == "iohr" {
                    "iohr not found on PATH".to_owned()
                } else {
                    format!("iohr not found at {program}")
                };
                skip(&mut tried, "cli", &reason);
            } else {
                tried.push(json!({ "source": "cli", "result": "used" }));
                used = Some(("cli", "cli"));
            }
        }

        let Some((source, kind)) = used else {
            let mut m = format!(
                "no credentials found for profile {:?}; tried:",
                profile.unwrap_or("default")
            );
            for t in &tried {
                let src = t["source"].as_str().unwrap_or_default();
                let reason = t["reason"].as_str().unwrap_or_default();
                let _ = write!(m, "\n  {src}: {reason}");
            }
            let _ = write!(
                m,
                "\nSet {p}KEY_ID, {p}KEY_SECRET and {p}SCOPES, or {p}TOKEN, or run `iohr login`."
            );
            self.problem("credential", "", m);
            return None;
        };
        if kind != "client_credentials"
            && kind != "custom"
            && let Some(s) = self.settings.remove("scopes")
        {
            self.ignored.push(json!({
                "key": "scopes", "source": s["source"], "reason": "not used by this credential",
            }));
        }
        Some(json!({ "source": source, "kind": kind, "tried": tried }))
    }

    fn cross_checks(&mut self) {
        let settings = self.settings.clone();
        let get = |k: &str| settings.get(k).cloned();
        if get("system_trust").is_some_and(|v| v["value"] == json!(false))
            && get("ca_bundle").is_none()
        {
            let src = get("system_trust")
                .map(|v| v["source"].as_str().unwrap_or_default().to_owned())
                .unwrap_or_default();
            self.problem(
                "system_trust",
                &src,
                "system_trust = false needs a ca_bundle to trust instead",
            );
        }
        match (get("client_cert"), get("client_key")) {
            (Some(c), None) => {
                let src = c["source"].as_str().unwrap_or_default().to_owned();
                self.problem("client_key", &src, "client_cert needs client_key");
            }
            (None, Some(k)) => {
                let src = k["source"].as_str().unwrap_or_default().to_owned();
                self.problem("client_cert", &src, "client_key needs client_cert");
            }
            _ => {}
        }
        for name in ["ca_bundle", "client_cert", "client_key"] {
            if let Some(v) = get(name) {
                let path = v["value"].as_str().unwrap_or_default().to_owned();
                if !self.exists(&path) {
                    let src = v["source"].as_str().unwrap_or_default().to_owned();
                    self.problem(name, &src, format!("cannot read {path}"));
                }
            }
        }
    }
}

/// A value before it is parsed, by where it came from.
enum Raw {
    Code(Value),
    Env(String),
    File(toml::Value),
}

/// Whether `e` fits the `no_proxy` grammar (section 6.2).
pub(crate) fn no_proxy_entry(e: &str) -> bool {
    if e == "*" {
        return true;
    }
    if let Some((ip, bits)) = e.split_once('/') {
        let Ok(bits) = bits.parse::<u8>() else {
            return false;
        };
        return match ip.parse::<std::net::IpAddr>() {
            Ok(std::net::IpAddr::V4(_)) => bits <= 32,
            Ok(std::net::IpAddr::V6(_)) => bits <= 128,
            Err(_) => false,
        };
    }
    let bare = e.trim_start_matches('[').trim_end_matches(']');
    if bare.parse::<std::net::IpAddr>().is_ok() {
        return true;
    }
    let (host, port) = match e.rsplit_once(':') {
        Some((h, p)) if !h.contains(':') || h.ends_with(']') => (h, Some(p)),
        _ => (e, None),
    };
    if port.is_some_and(|p| p.parse::<u16>().is_err()) {
        return false;
    }
    let host = host.trim_start_matches('.');
    let host = host.trim_start_matches('[').trim_end_matches(']');
    !host.is_empty()
        && (host.parse::<std::net::IpAddr>().is_ok()
            || host.split('.').all(|l| {
                !l.is_empty()
                    && l.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            }))
}

/// Whether `program` can be run: a path to a file, or a name found on `PATH`.
pub(crate) fn program_found(program: &str, env: &BTreeMap<String, String>) -> bool {
    let p = std::path::Path::new(program);
    if p.components().count() > 1 || p.is_absolute() {
        return p.is_file();
    }
    let Some(path) = env.get("PATH") else {
        return false;
    };
    let exts: &[&str] = if cfg!(windows) {
        &["", ".exe", ".cmd", ".bat"]
    } else {
        &[""]
    };
    std::env::split_paths(path).any(|dir| {
        exts.iter()
            .any(|ext| dir.join(format!("{program}{ext}")).is_file())
    })
}
