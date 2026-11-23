//! InOrbit monitors kept as files in a project's `.inorbit/monitors/` folder (RFC 0008).
//!
//! Each `*.toml` file is a block of the InOrbit agent's `checks.toml` (core RFC 0040.1 and
//! 0040.2; dataplane `crates/iohr-agent/src/checks_file.rs`): `[[check]]` entries the agent runs
//! and expects to pass, `[[refuse]]` entries it expects to be refused. The agent sends them in
//! its hello and the platform keeps one monitor per entry. This crate loads the folder, checks
//! it against the schema (`schemas/monitors/monitor/v0.json`) and the agent's rules the schema
//! cannot say, and runs each monitor once from here ([`run`]). It is what `iohr monitors` runs.

#![allow(
    clippy::must_use_candidate,
    clippy::missing_errors_doc,
    clippy::too_many_lines,
    clippy::doc_markdown,
    clippy::format_push_string,
    clippy::trivially_copy_pass_by_ref,
    clippy::many_single_char_names,
    clippy::items_after_statements,
    clippy::if_not_else,
    clippy::assert_is_empty,
    clippy::needless_pass_by_value,
    clippy::unnecessary_wraps,
    clippy::manual_let_else,
    clippy::uninlined_format_args,
    clippy::cast_precision_loss,
    reason = "ported whole from the Eye monitors crate, where these pedantic style lints were not on; its tests are the contract"
)]

pub mod cli;
pub mod edit;
pub mod explain;
pub mod install;
pub mod run;
pub mod shape;
pub mod state;

use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use url::Url;

use crate::schema::{self, Schemas};

/// Where the monitors live under a project root.
pub const DIR: &str = ".inorbit/monitors";
/// The most entries one agent reads (checks_file.rs `MAX_ENTRIES`).
pub const MAX_ENTRIES: usize = 100;
/// The shortest and longest interval the agent and the platform accept, in seconds.
pub const EVERY: (u64, u64) = (60, 86_400);
const MAX_URL: usize = 2048;
const MAX_BODY: usize = 8 * 1024;

/// Why a command did not do what it was asked.
#[derive(Debug)]
pub enum Error {
    /// Wrong usage: no folder, an unknown monitor. Exit 2.
    Usage(String),
    /// A failed check, one line per reason. Exit 1.
    Refused(Vec<String>),
    /// A file could not be read. Exit 1.
    Io(String),
}

impl Error {
    /// The exit code: 2 for wrong usage, 1 for everything else.
    pub fn code(&self) -> i32 {
        match self {
            Error::Usage(_) => 2,
            Error::Refused(_) | Error::Io(_) => 1,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Usage(m) | Error::Io(m) => f.write_str(m),
            Error::Refused(lines) => f.write_str(&lines.join("\n")),
        }
    }
}

impl std::error::Error for Error {}

/// The nearest `.inorbit/monitors` folder, walking up from `start`.
pub fn find(start: &Path) -> Option<PathBuf> {
    start.ancestors().map(|d| d.join(DIR)).find(|p| p.is_dir())
}

/// One monitor file, read as JSON.
#[derive(Debug, Clone)]
pub struct File {
    pub path: PathBuf,
    pub value: Value,
}

impl File {
    /// The file's name, for messages.
    pub fn name(&self) -> String {
        self.path.file_name().map_or_else(
            || self.path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        )
    }
}

/// Every `*.toml` in `dir`, sorted by name.
pub fn load(dir: &Path) -> Result<Vec<File>, Error> {
    let entries = fs::read_dir(dir).map_err(|e| Error::Usage(format!("{}: {e}", dir.display())))?;
    let mut paths: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "toml"))
        .collect();
    paths.sort();
    let mut out = Vec::with_capacity(paths.len());
    for path in paths {
        let text =
            fs::read_to_string(&path).map_err(|e| Error::Io(format!("{}: {e}", path.display())))?;
        let value =
            parse(&text).map_err(|e| Error::Refused(vec![format!("{}: {e}", path.display())]))?;
        out.push(File { path, value });
    }
    Ok(out)
}

/// A TOML monitor file as JSON, the shape the schema reads.
pub fn parse(text: &str) -> Result<Value, String> {
    let t: toml::Table = toml::from_str(text).map_err(|e| e.message().to_owned())?;
    serde_json::to_value(t).map_err(|e| e.to_string())
}

/// Every file folded into one `checks.toml` the InOrbit agent reads today: the same entries,
/// with the keys the agent does not take yet left out (`expect.shape`, `enabled`; RFC 0008), and
/// the cli checks and the entries with `enabled = false` left out whole: the agent has no cli
/// surface and runs no project binary, and an entry off by default would page from every Mac
/// whose setup does not run it (ADR 0009). The agent reads its file with unknown keys refused,
/// so a file with `shape` or `enabled` would be refused whole.
pub fn export(files: &[File]) -> Result<String, String> {
    let mut out = toml::Table::new();
    for f in files {
        for kind in ["check", "refuse"] {
            let Some(entries) = f.value.get(kind).and_then(Value::as_array) else {
                continue;
            };
            for e in entries
                .iter()
                .filter(|e| !is_cli(e) && e.get("enabled").and_then(Value::as_bool) != Some(false))
            {
                let mut e = e.clone();
                e.as_object_mut().map(|o| o.remove("enabled"));
                if let Some(x) = e.get_mut("expect").and_then(Value::as_object_mut) {
                    x.remove("shape");
                    if x.is_empty() {
                        e.as_object_mut().map(|o| o.remove("expect"));
                    }
                }
                let t: toml::Value =
                    serde_json::from_value(e).map_err(|err| format!("{}: {err}", f.name()))?;
                match out
                    .entry(kind)
                    .or_insert_with(|| toml::Value::Array(Vec::new()))
                {
                    toml::Value::Array(a) => a.push(t),
                    _ => unreachable!("only arrays are inserted"),
                }
            }
        }
    }
    toml::to_string(&out).map_err(|e| e.to_string())
}

/// A cli entry: `target = { cli = ... }` or `surface = "cli"`.
fn is_cli(e: &Value) -> bool {
    s(e, "surface") == Some("cli") || e.get("target").is_some_and(|t| t.get("cli").is_some())
}

/// How a monitor is checked (checks_file.rs `Surface`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    Http,
    Tcp,
    Tls,
    GrpcHealth,
    Grpc,
    Sse,
    Ws,
    Mqtt,
    Mcp,
    Graphql,
    Hwmon,
    Examples,
    /// The project's own built binary, run with argv in a throwaway folder ([`cli`]).
    Cli,
}

impl Surface {
    pub fn as_str(self) -> &'static str {
        match self {
            Surface::Http => "http",
            Surface::Tcp => "tcp",
            Surface::Tls => "tls",
            Surface::GrpcHealth => "grpc_health",
            Surface::Grpc => "grpc",
            Surface::Sse => "sse",
            Surface::Ws => "ws",
            Surface::Mqtt => "mqtt",
            Surface::Mcp => "mcp",
            Surface::Graphql => "graphql",
            Surface::Hwmon => "hwmon",
            Surface::Examples => "examples",
            Surface::Cli => "cli",
        }
    }

    fn parse(s: &str) -> Option<Surface> {
        Some(match s {
            "http" => Surface::Http,
            "tcp" => Surface::Tcp,
            "tls" => Surface::Tls,
            "grpc_health" => Surface::GrpcHealth,
            "grpc" => Surface::Grpc,
            "sse" => Surface::Sse,
            "ws" => Surface::Ws,
            "mqtt" => Surface::Mqtt,
            "mcp" => Surface::Mcp,
            "graphql" => Surface::Graphql,
            "hwmon" => Surface::Hwmon,
            "examples" => Surface::Examples,
            "cli" => Surface::Cli,
            _ => return None,
        })
    }
}

/// Where a monitor points.
#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    Url(Url),
    HostPort {
        host: String,
        port: u16,
    },
    Sensor(String),
    Set(String),
    /// The binary's name; the runner resolves it to the one just built, never a path.
    Cli(String),
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Target::Url(u) => f.write_str(u.as_str()),
            Target::HostPort { host, port } => write!(f, "{host}:{port}"),
            Target::Sensor(s) => write!(f, "sensor {s}"),
            Target::Set(s) => write!(f, "set {s}"),
            Target::Cli(s) => write!(f, "cli {s}"),
        }
    }
}

/// What a run must see to pass.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Expect {
    pub status: Option<u16>,
    pub max_ms: Option<u64>,
    pub valid_for_days: Option<u16>,
    pub location: Option<String>,
    pub location_prefix: Option<String>,
    /// Lower-case header name to the start its value must have.
    pub headers: BTreeMap<String, String>,
    /// The JSON Schema the body must match (http and sse; for cli the stdout; RFC 0008). The
    /// InOrbit agent does not read bodies yet and refuses this key, so [`export`] leaves it out.
    pub shape: Option<Shape>,
    /// cli: the exit code expected; 0 when left out.
    pub exit: Option<i32>,
    /// cli: the schema path the argv must match as a JSON array of strings.
    pub request: Option<String>,
}

/// One run of a cli check's binary: the argv after the binary and what it must give back.
#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    pub argv: Vec<String>,
    pub expect: Expect,
}

/// `expect.shape`: one schema for the body, or for sse one schema per event name.
#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    /// A repo-relative schema path such as `schemas/api/items/v0.json`.
    One(String),
    /// sse: event name to schema path; an event not named fails.
    Events(BTreeMap<String, String>),
}

/// One monitor, checked and typed.
#[derive(Debug, Clone, PartialEq)]
pub struct Monitor {
    pub file: String,
    pub name: String,
    /// A `[[refuse]]` entry.
    pub refuse: bool,
    /// A `[[refuse]]` the policy or the platform refuses, never sent.
    pub by: Option<String>,
    pub surface: Surface,
    pub target: Target,
    pub every_secs: u64,
    pub fail_after: u8,
    pub expect: Expect,
    pub method: Option<String>,
    /// The request body as JSON text.
    pub body: Option<String>,
    pub headers: BTreeMap<String, String>,
    /// sse: data events to wait for.
    pub events: u32,
    pub label: Option<String>,
    pub category: Option<String>,
    pub tags: BTreeMap<String, String>,
    /// `enabled = false`: off by default, for a monitor only some setups run (such as a second
    /// local server); `iohr monitors enable` turns it on locally (see [`state`]).
    pub enabled: bool,
    /// cli: the runs in order, in one throwaway folder; `argv` alone is one step.
    pub steps: Vec<Step>,
    /// cli: let the binary reach the network.
    pub network: bool,
}

/// What [`check_files`] found.
#[derive(Debug, Default)]
pub struct Report {
    pub monitors: Vec<Monitor>,
    /// One line per problem, `file: name: why`; empty when every file is valid.
    pub problems: Vec<String>,
    /// Allowed, but worth a look.
    pub warnings: Vec<String>,
}

/// Every problem with one file's JSON, one line each; empty when it is valid.
///
/// The schema runs first; when it fails, only its errors are given. Then the agent's rules a
/// schema cannot express (see [`check_files`]).
pub fn check(value: &Value) -> Vec<String> {
    let f = File {
        path: PathBuf::from("file"),
        value: value.clone(),
    };
    let r = check_files(std::slice::from_ref(&f));
    r.problems
        .into_iter()
        .map(|p| p.strip_prefix("file: ").map_or(p.clone(), str::to_owned))
        .collect()
}

/// Checks every file and the folder as a whole: the schema, then per entry the agent's rules
/// (every between 60s and 24h; the keys each surface takes; status only on http, sse, mcp and
/// graphql; header and location asserts on http only; URLs without a user, a password or a
/// fragment; auth by reference only), then names unique across every file and at most
/// [`MAX_ENTRIES`] entries.
pub fn check_files(files: &[File]) -> Report {
    let mut r = Report::default();
    let schemas = match Schemas::embedded() {
        Ok(v) => v,
        Err(e) => {
            r.problems.push(format!("monitor schema: {e}"));
            return r;
        }
    };
    let mut seen: HashMap<String, String> = HashMap::new();
    for f in files {
        let file = f.name();
        let schema: Vec<String> = match schemas.validate(schema::MONITOR, &f.value) {
            Ok(()) => Vec::new(),
            Err(schema::ValidateError::Invalid(found)) => found
                .iter()
                .map(|v| format!("{file}: {}: breaks {}", v.path, v.rule))
                .collect(),
            Err(e) => vec![format!("{file}: monitor schema: {e}")],
        };
        if !schema.is_empty() {
            r.problems.extend(schema);
            continue;
        }
        for (kind, refuse) in [("check", false), ("refuse", true)] {
            let Some(list) = f.value.get(kind).and_then(Value::as_array) else {
                continue;
            };
            for e in list {
                let name = e
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("?")
                    .to_owned();
                if let Some(other) = seen.insert(name.clone(), file.clone()) {
                    r.problems.push(format!(
                        "{file}: {name}: the name is used twice (also in {other})"
                    ));
                }
                match entry(e, refuse, &file) {
                    Ok((m, warns)) => {
                        r.warnings
                            .extend(warns.into_iter().map(|w| format!("{file}: {name}: {w}")));
                        r.monitors.push(m);
                    }
                    Err(why) => r.problems.push(format!("{file}: {name}: {why}")),
                }
            }
        }
    }
    if seen.len() > MAX_ENTRIES {
        r.problems.push(format!(
            "{} entries; the agent reads at most {MAX_ENTRIES}",
            seen.len()
        ));
    }
    r
}

fn s<'a>(e: &'a Value, k: &str) -> Option<&'a str> {
    e.get(k).and_then(Value::as_str)
}

/// `"60s"`, `"5m"`, `"1h"` or `"1d"` in seconds, between [`EVERY`].
pub fn every_secs(text: &str) -> Result<u64, String> {
    let n = secs(text).map_err(|e| format!("every = {e}"))?;
    if (EVERY.0..=EVERY.1).contains(&n) {
        Ok(n)
    } else {
        Err(format!("every = {text:?}: must be 60s to 24h"))
    }
}

/// A duration such as `"30s"`, `"5m"`, `"2h"` or `"1d"` in seconds, any length (for `every`,
/// `disable --for` and `run --every`).
pub fn secs(text: &str) -> Result<u64, String> {
    let bad = || format!("{text:?}: use a number with s, m, h or d (\"60s\", \"5m\", \"1h\")");
    let unit = text.chars().last().ok_or_else(bad)?;
    let n: u64 = text[..text.len() - unit.len_utf8()]
        .parse()
        .map_err(|_| bad())?;
    match unit {
        's' => Ok(n),
        'm' => Ok(n.saturating_mul(60)),
        'h' => Ok(n.saturating_mul(3600)),
        'd' => Ok(n.saturating_mul(86_400)),
        _ => Err(bad()),
    }
}

fn url(text: &str, warns: &mut Vec<String>) -> Result<Url, String> {
    if text.len() > MAX_URL {
        return Err(format!("the URL is longer than {MAX_URL}"));
    }
    let u = Url::parse(text).map_err(|e| format!("target {text:?}: {e}"))?;
    if !u.username().is_empty() || u.password().is_some() {
        return Err("the URL names a user or password; use auth = \"env:NAME\"".into());
    }
    if u.fragment().is_some() {
        return Err("the URL has a #fragment, which is never sent".into());
    }
    if u.host_str().is_none() {
        return Err(format!("target {text:?} has no host"));
    }
    if u.query().is_some() {
        warns.push(
            "the URL has a query string, which the platform shows in the monitor's name and logs"
                .into(),
        );
    }
    Ok(u)
}

fn host_port(text: &str, surface: Option<Surface>) -> Result<(String, u16), String> {
    let bad = || format!("target {text:?} is not a URL, host or host:port");
    let (host, port) = if let Some(rest) = text.strip_prefix('[') {
        let (h, after) = rest.split_once(']').ok_or_else(bad)?;
        (h, after.strip_prefix(':'))
    } else {
        match text.rsplit_once(':') {
            Some((h, p)) if !h.contains(':') => (h, Some(p)),
            _ => (text, None),
        }
    };
    if host.is_empty()
        || host
            .chars()
            .any(|c| c.is_whitespace() || c == '/' || c == '@')
    {
        return Err(bad());
    }
    let port = match (port, surface) {
        (Some(p), _) => p.parse().map_err(|_| bad())?,
        (None, Some(Surface::Tls)) => 443,
        (None, _) => {
            return Err(format!(
                "target {text:?} needs a port (host:port), or a URL"
            ));
        }
    };
    Ok((host.to_owned(), port))
}

/// One entry's rules, after the schema passed. The order follows checks_file.rs, so the first
/// problem named is the one the agent would name.
fn entry(e: &Value, refuse: bool, file: &str) -> Result<(Monitor, Vec<String>), String> {
    let mut warns = Vec::new();
    let name = s(e, "name").unwrap_or_default().to_owned();
    let given = s(e, "surface").and_then(Surface::parse);
    let t = e.get("target").ok_or("no target")?;
    let cli = t.get("cli").and_then(Value::as_str);
    if matches!((cli, given), (Some(_), Some(g)) if g != Surface::Cli)
        || (cli.is_none() && given == Some(Surface::Cli))
    {
        return Err(
            "a cli check names its binary as target = { cli = \"...\" }, and only a cli check does"
                .into(),
        );
    }
    if cli.is_some() && refuse {
        return Err("a cli check is a [[check]]; there is no one to refuse it".into());
    }
    // A cli check runs on each build, not on a clock, so it takes no every.
    let every = if cli.is_some() {
        0
    } else {
        every_secs(s(e, "every").unwrap_or_default())?
    };
    let (target, inferred) = match t {
        Value::Object(o) if o.contains_key("cli") => (
            Target::Cli(cli.unwrap_or_default().to_owned()),
            Surface::Cli,
        ),
        Value::String(text) if given == Some(Surface::Hwmon) => {
            if text.contains("://") {
                return Err("an hwmon check names a sensor, not a URL".into());
            }
            (Target::Sensor(text.clone()), Surface::Hwmon)
        }
        Value::String(text) if text.contains("://") => {
            (Target::Url(url(text, &mut warns)?), Surface::Http)
        }
        Value::String(text) => {
            let (host, port) = host_port(text, given)?;
            (Target::HostPort { host, port }, Surface::Tcp)
        }
        Value::Object(o) => match (o.get("url"), o.get("host"), o.get("sensor"), o.get("set")) {
            (Some(Value::String(u)), ..) => (Target::Url(url(u, &mut warns)?), Surface::Http),
            (_, Some(Value::String(h)), ..) => {
                let port = o
                    .get("port")
                    .and_then(Value::as_u64)
                    .and_then(|p| u16::try_from(p).ok())
                    .ok_or("target.port must be 1 to 65535")?;
                (
                    Target::HostPort {
                        host: h.clone(),
                        port,
                    },
                    Surface::Tcp,
                )
            }
            (_, _, Some(Value::String(x)), _) => (Target::Sensor(x.clone()), Surface::Hwmon),
            (.., Some(Value::String(x))) => (Target::Set(x.clone()), Surface::Examples),
            _ => return Err("target is not a URL, host or host:port".into()),
        },
        _ => return Err("target is not a URL, host or host:port".into()),
    };
    let surface = given.unwrap_or(inferred);
    use Surface as S;
    if matches!(surface, S::Http | S::Sse) && !matches!(target, Target::Url(_)) {
        return Err(format!("a {} check needs a URL target", surface.as_str()));
    }
    if let Target::Url(u) = &target
        && matches!(surface, S::Http | S::Sse)
        && !matches!(u.scheme(), "http" | "https")
    {
        return Err(format!(
            "a {} check needs an http or https URL, not {}:",
            surface.as_str(),
            u.scheme()
        ));
    }

    let expect = expect_of(e.get("expect").unwrap_or(&Value::Null));
    match (&expect.shape, surface) {
        (None, _)
        | (Some(Shape::One(_)), S::Http | S::Sse | S::Cli)
        | (Some(Shape::Events(_)), S::Sse) => {}
        (Some(Shape::Events(_)), S::Http | S::Cli) => {
            return Err(format!(
                "expect.shape as a table of events is for sse checks; {} takes one schema path",
                surface.as_str()
            ));
        }
        _ => return Err("expect.shape is for http, sse and cli checks".into()),
    }
    if (expect.exit.is_some() || expect.request.is_some()) && surface != S::Cli {
        return Err("expect.exit and expect.request are for cli checks".into());
    }
    if expect.status.is_some() && !matches!(surface, S::Http | S::Sse | S::Mcp | S::Graphql) {
        return Err("expect.status is for http, sse, mcp and graphql checks".into());
    }
    if expect.valid_for_days.is_some() && !matches!(surface, S::Tls | S::Http) {
        return Err("expect.valid_for_days is for tls and http checks".into());
    }
    let locates = expect.location.as_ref().or(expect.location_prefix.as_ref());
    if (locates.is_some() || !expect.headers.is_empty()) && surface != S::Http {
        return Err(
            "expect.location, expect.location_prefix and expect.headers are for http checks".into(),
        );
    }
    if let Some(l) = locates {
        if !expect.status.is_some_and(|c| (300..=399).contains(&c)) {
            return Err("expect.location needs expect.status 300 to 399".into());
        }
        if !(l.starts_with('/') || l.starts_with("http://") || l.starts_with("https://")) {
            return Err(format!(
                "expect.location {l:?} must be a path (/...) or an http(s) URL"
            ));
        }
    }

    let has = |k: &str| e.get(k).is_some();
    let only = |k: &str, for_: &[Surface]| -> Result<(), String> {
        if has(k) && !for_.contains(&surface) {
            let names: Vec<&str> = for_.iter().map(|s| s.as_str()).collect();
            Err(format!("{k} is for {} checks", names.join(" and ")))
        } else {
            Ok(())
        }
    };
    only("service", &[S::GrpcHealth])?;
    if has("auth_scheme") && !has("auth") {
        return Err("auth_scheme needs auth".into());
    }
    if has("auth") && matches!(surface, S::Tcp | S::Tls | S::Hwmon | S::Examples | S::Cli) {
        return Err("auth is not used by tcp, tls, hwmon, examples and cli checks".into());
    }
    for k in ["argv", "steps", "network"] {
        only(k, &[S::Cli])?;
    }
    let steps = if surface == S::Cli {
        cli_steps(e, &expect)?
    } else {
        Vec::new()
    };
    only("method", &[S::Http, S::Ws, S::Grpc])?;
    if matches!(surface, S::Ws | S::Grpc) && !has("method") {
        return Err(format!("a {} check needs method", surface.as_str()));
    }
    only("body", &[S::Http])?;
    only("headers", &[S::Http])?;
    only("query", &[S::Graphql])?;
    only("variables", &[S::Graphql])?;
    only("topic", &[S::Mqtt])?;
    only("params", &[S::Ws, S::Mqtt])?;
    only("events", &[S::Sse])?;
    for k in ["min_tools", "tool", "args", "allow_side_effects"] {
        only(k, &[S::Mcp])?;
    }
    only("expect_error", &[S::Ws, S::Mqtt])?;
    only("expect_code", &[S::Grpc])?;
    for k in ["kind", "warn", "crit", "rate_warn", "rate_crit", "below"] {
        only(k, &[S::Hwmon])?;
    }
    let body = match e.get("body") {
        None => None,
        Some(b) => {
            let text = serde_json::to_string(b).map_err(|e| format!("body: {e}"))?;
            if text.len() > MAX_BODY {
                return Err(format!(
                    "body is {} bytes as JSON; at most {MAX_BODY}",
                    text.len()
                ));
            }
            Some(text)
        }
    };
    let strings = |k: &str| -> BTreeMap<String, String> {
        e.get(k)
            .and_then(Value::as_object)
            .map(|o| {
                o.iter()
                    .map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_owned()))
                    .collect()
            })
            .unwrap_or_default()
    };
    let m = Monitor {
        file: file.to_owned(),
        name,
        refuse,
        by: s(e, "by").map(str::to_owned),
        surface,
        target,
        every_secs: every,
        fail_after: e
            .get("fail_after")
            .and_then(Value::as_u64)
            .and_then(|n| u8::try_from(n).ok())
            .unwrap_or(if refuse { 1 } else { 2 }),
        expect,
        method: s(e, "method").map(str::to_owned),
        body,
        headers: strings("headers"),
        events: e
            .get("events")
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())
            .unwrap_or(1),
        label: s(e, "label").map(str::to_owned),
        category: s(e, "category").map(str::to_owned),
        tags: strings("tags"),
        enabled: e.get("enabled").and_then(Value::as_bool).unwrap_or(true),
        steps,
        network: e.get("network").and_then(Value::as_bool).unwrap_or(false),
    };
    Ok((m, warns))
}

/// An `expect` table, typed; the rules on which keys go with which surface are the caller's.
fn expect_of(x: &Value) -> Expect {
    let xs = |k: &str| x.get(k).and_then(Value::as_str).map(str::to_owned);
    let xn = |k: &str| x.get(k).and_then(Value::as_u64);
    Expect {
        status: xn("status").and_then(|n| u16::try_from(n).ok()),
        max_ms: xn("max_ms"),
        valid_for_days: xn("valid_for_days").and_then(|n| u16::try_from(n).ok()),
        location: xs("location"),
        location_prefix: xs("location_prefix"),
        headers: x
            .get("headers")
            .and_then(Value::as_object)
            .map(|h| {
                h.iter()
                    .map(|(k, v)| {
                        (
                            k.to_ascii_lowercase(),
                            v.as_str().unwrap_or_default().to_owned(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default(),
        shape: match x.get("shape") {
            Some(Value::String(p)) => Some(Shape::One(p.clone())),
            Some(Value::Object(t)) => Some(Shape::Events(
                t.iter()
                    .map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_owned()))
                    .collect(),
            )),
            _ => None,
        },
        exit: xn("exit").and_then(|n| i32::try_from(n).ok()),
        request: xs("request"),
    }
}

/// A cli check's steps: `argv` with the entry's expect as one step, or `steps`, each with its
/// own expect and the entry's expect holding only max_ms (the deadline for them all).
fn cli_steps(e: &Value, expect: &Expect) -> Result<Vec<Step>, String> {
    let argv = |v: &Value| -> Vec<String> {
        v.as_array()
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    };
    let only_run = |x: &Expect, at: &str| -> Result<(), String> {
        let run = Expect {
            exit: x.exit,
            request: x.request.clone(),
            shape: x.shape.clone(),
            max_ms: x.max_ms,
            ..Expect::default()
        };
        if *x != run {
            return Err(format!("{at} takes only exit, request, shape and max_ms"));
        }
        if matches!(x.shape, Some(Shape::Events(_))) {
            return Err(format!("{at}.shape takes one schema path"));
        }
        Ok(())
    };
    match (e.get("argv"), e.get("steps").and_then(Value::as_array)) {
        (Some(a), None) => {
            only_run(expect, "expect")?;
            Ok(vec![Step {
                argv: argv(a),
                expect: expect.clone(),
            }])
        }
        (None, Some(list)) => {
            if *expect
                != (Expect {
                    max_ms: expect.max_ms,
                    ..Expect::default()
                })
            {
                return Err("with steps, the entry's expect takes only max_ms; each step has its own expect".into());
            }
            list.iter()
                .enumerate()
                .map(|(i, st)| {
                    let x = expect_of(st.get("expect").unwrap_or(&Value::Null));
                    only_run(&x, &format!("steps[{i}].expect"))?;
                    if x.max_ms.is_some() {
                        return Err(format!(
                            "steps[{i}].expect.max_ms: the deadline is the entry's expect.max_ms"
                        ));
                    }
                    Ok(Step {
                        argv: argv(st.get("argv").unwrap_or(&Value::Null)),
                        expect: x,
                    })
                })
                .collect()
        }
        _ => Err("a cli check takes argv or steps, one of them".into()),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn one(toml: &str) -> Vec<String> {
        check(&parse(toml).unwrap())
    }

    const API: &str = "[[check]]\nname = \"api\"\ntarget = \"http://127.0.0.1:7171/api/health\"\nevery = \"60s\"\n";

    #[test]
    fn a_plain_check_passes_and_is_typed() {
        assert!(one(API).is_empty(), "{:?}", one(API));
        let f = File {
            path: "monitors.toml".into(),
            value: parse(API).unwrap(),
        };
        let r = check_files(&[f]);
        let m = &r.monitors[0];
        assert_eq!(
            (m.surface, m.every_secs, m.fail_after, m.refuse),
            (Surface::Http, 60, 2, false)
        );
        assert_eq!(m.target.to_string(), "http://127.0.0.1:7171/api/health");
    }

    #[test]
    fn the_agents_rules_refuse_what_the_schema_lets_through() {
        let cases = [
            ("every = \"30s\"", "60s to 24h"),
            ("every = \"2d\"", "60s to 24h"),
            ("events = 1", "events is for sse"),
            ("service = \"x\"", "service is for grpc_health"),
            ("query = \"{ a }\"", "query is for graphql"),
            ("auth_scheme = \"Bearer\"", "auth_scheme needs auth"),
            ("expect = { valid_for_days = 0 }", "valid_for_days"),
            (
                "expect = { location = \"/x\" }",
                "needs expect.status 300 to 399",
            ),
            (
                "expect = { shape = { item = \"schemas/api/item/v0.json\" } }",
                "a table of events is for sse",
            ),
        ];
        for (extra, want) in cases {
            let text = API.replace("every = \"60s\"\n", "")
                + extra
                + "\n"
                + if extra.starts_with("every") {
                    ""
                } else {
                    "every = \"60s\"\n"
                };
            let got = one(&text).join("; ");
            assert!(got.contains(want), "{extra}: wanted {want:?}, got {got:?}");
        }
        let tcp = "[[check]]\nname = \"db\"\ntarget = \"localhost:9494\"\nevery = \"5m\"\nexpect = { status = 200 }\n";
        assert!(one(tcp).join(" ").contains("expect.status is for http"));
        let auth_tcp = "[[check]]\nname = \"db\"\ntarget = \"localhost:9494\"\nevery = \"5m\"\nauth = \"env:X\"\n";
        assert!(one(auth_tcp).join(" ").contains("auth is not used by tcp"));
        let no_port = "[[check]]\nname = \"db\"\ntarget = \"localhost\"\nevery = \"5m\"\n";
        assert!(one(no_port).join(" ").contains("needs a port"));
        let tls = "[[check]]\nname = \"cert\"\nsurface = \"tls\"\ntarget = \"example.com\"\nevery = \"1d\"\nexpect = { valid_for_days = 14 }\n";
        assert!(one(tls).is_empty(), "{:?}", one(tls));
        let ws = "[[check]]\nname = \"ws\"\nsurface = \"ws\"\ntarget = \"wss://example.com/v1/ws\"\nevery = \"5m\"\n";
        assert!(one(ws).join(" ").contains("needs method"));
    }

    #[test]
    fn urls_carry_no_secrets_and_queries_warn() {
        let user = API.replace("http://127.0.0.1", "http://me:pw@127.0.0.1");
        assert!(one(&user).join(" ").contains("user or password"));
        let frag = API.replace("/api/health", "/api/health#x");
        assert!(one(&frag).join(" ").contains("fragment"));
        let q = API.replace("/api/health", "/api/health?x=1");
        let r = check_files(&[File {
            path: "a.toml".into(),
            value: parse(&q).unwrap(),
        }]);
        assert!(r.problems.is_empty());
        assert!(r.warnings[0].contains("query string"));
    }

    #[test]
    fn refusals_name_by_or_expect_and_checks_never_by() {
        let by = "[[refuse]]\nname = \"meta\"\ntarget = \"http://169.254.169.254/\"\nevery = \"1h\"\nby = \"policy\"\n";
        assert!(one(by).is_empty(), "{:?}", one(by));
        assert!(
            !one(&format!("{by}expect = {{ status = 403 }}\n")).is_empty(),
            "both"
        );
        let neither =
            "[[refuse]]\nname = \"meta\"\ntarget = \"http://169.254.169.254/\"\nevery = \"1h\"\n";
        assert!(!one(neither).is_empty());
        assert!(
            !one(&format!("{API}by = \"policy\"\n")).is_empty(),
            "a check with by"
        );
        let f = File {
            path: "x.toml".into(),
            value: parse(by).unwrap(),
        };
        assert_eq!(
            check_files(&[f]).monitors[0].fail_after,
            1,
            "a refuse fails after 1"
        );
    }

    #[test]
    fn names_are_unique_across_files_and_entries_are_capped() {
        let a = File {
            path: "a.toml".into(),
            value: parse(API).unwrap(),
        };
        let b = File {
            path: "b.toml".into(),
            value: parse(API).unwrap(),
        };
        let r = check_files(&[a, b]);
        assert_eq!(
            r.problems,
            vec!["b.toml: api: the name is used twice (also in a.toml)"]
        );
        let many: String = (0..=MAX_ENTRIES)
            .map(|i| API.replace("\"api\"", &format!("\"api-{i}\"")))
            .collect();
        let r = check_files(&[File {
            path: "m.toml".into(),
            value: parse(&many).unwrap(),
        }]);
        assert!(
            r.problems.iter().any(|p| p.contains("at most 100")),
            "{:?}",
            r.problems
        );
    }

    #[test]
    fn a_shape_is_typed_and_export_leaves_it_out_for_the_agent() {
        let text = format!(
            "{API}expect = {{ status = 200, shape = \"schemas/api/health/v0.json\" }}\n[[check]]\nname = \"s\"\nsurface = \"sse\"\ntarget = \"http://127.0.0.1:7171/api/stream\"\nevery = \"5m\"\nexpect = {{ shape = {{ item = \"schemas/api/item/v0.json\" }} }}\n[[check]]\nname = \"t\"\ntarget = \"127.0.0.1:9\"\nevery = \"5m\"\n"
        );
        let f = File {
            path: "monitors.toml".into(),
            value: parse(&text).unwrap(),
        };
        let r = check_files(std::slice::from_ref(&f));
        assert!(r.problems.is_empty(), "{:?}", r.problems);
        assert_eq!(
            r.monitors[0].expect.shape,
            Some(Shape::One("schemas/api/health/v0.json".into()))
        );
        assert!(matches!(&r.monitors[1].expect.shape, Some(Shape::Events(t)) if t.len() == 1));
        let tcp = "[[check]]\nname = \"t\"\ntarget = \"127.0.0.1:9\"\nevery = \"5m\"\nexpect = { shape = \"schemas/api/health/v0.json\" }\n";
        assert!(
            one(tcp)
                .iter()
                .any(|p| p.contains("shape is for http, sse and cli")),
            "{:?}",
            one(tcp)
        );

        let out = export(&[f]).unwrap();
        assert!(!out.contains("shape"), "{out}");
        let back = parse(&out).unwrap();
        assert_eq!(back["check"].as_array().unwrap().len(), 3);
        assert_eq!(back["check"][0]["expect"]["status"], 200);
        assert!(
            back["check"][1].get("expect").is_none(),
            "an expect left empty goes: {out}"
        );
    }

    const CLI: &str = "[[check]]\nname = \"board\"\ntarget = { cli = \"app\" }\nargv = [\"scrum\", \"board\", \"--json\"]\nexpect = { shape = \"schemas/cli/scrum-board/v0.json\", request = \"schemas/cli/scrum-board/v0.json#/$defs/request\" }\n";

    #[test]
    fn a_cli_check_is_typed_with_its_steps_and_export_leaves_it_out() {
        let chain = "[[check]]\nname = \"chain\"\ntarget = { cli = \"app\" }\nexpect = { max_ms = 5000 }\n[[check.steps]]\nargv = [\"scrum\", \"add\", \"x\"]\n[[check.steps]]\nargv = [\"scrum\", \"show\", \"x\"]\nexpect = { exit = 2 }\n";
        let f = File {
            path: "cli.toml".into(),
            value: parse(&format!("{API}{CLI}{chain}")).unwrap(),
        };
        let r = check_files(std::slice::from_ref(&f));
        assert!(r.problems.is_empty(), "{:?}", r.problems);
        let board = &r.monitors[1];
        assert_eq!(
            (board.surface, &board.target, board.every_secs),
            (Surface::Cli, &Target::Cli("app".into()), 0)
        );
        assert_eq!(board.steps.len(), 1);
        assert_eq!(board.steps[0].argv, ["scrum", "board", "--json"]);
        assert_eq!(
            board.steps[0].expect.request.as_deref(),
            Some("schemas/cli/scrum-board/v0.json#/$defs/request")
        );
        let chain = &r.monitors[2];
        assert_eq!(
            (
                chain.steps.len(),
                chain.steps[1].expect.exit,
                chain.expect.max_ms
            ),
            (2, Some(2), Some(5000))
        );

        let out = export(&[f]).unwrap();
        assert!(!out.contains("cli"), "{out}");
        assert_eq!(parse(&out).unwrap()["check"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn a_cli_check_refuses_what_it_cannot_run() {
        let bad = |extra: &str, want: &str| {
            let p = one(&format!(
                "[[check]]\nname = \"c\"\ntarget = {{ cli = \"app\" }}\n{extra}\n"
            ));
            assert!(p.iter().any(|p| p.contains(want)), "{extra}: {p:?}");
        };
        bad("", "argv or steps");
        bad(
            "argv = [\"a\"]\nsteps = [{ argv = [\"b\"] }]",
            "argv or steps",
        );
        bad(
            "argv = [\"a\"]\nexpect = { status = 200 }",
            "expect.status is for",
        );
        bad(
            "steps = [{ argv = [\"b\"] }]\nexpect = { exit = 1 }",
            "takes only max_ms",
        );
        bad(
            "steps = [{ argv = [\"b\"], expect = { max_ms = 9 } }]",
            "the deadline is the entry's",
        );
        bad(
            "argv = [\"a\"]\nexpect = { shape = { e = \"schemas/a/b/v0.json\" } }",
            "takes one schema path",
        );
        bad("argv = [\"a\"]\nauth = \"env:X\"", "auth is not used");
        bad("argv = [\"a\"]\nsurface = \"http\"", "only a cli check");
        bad("argv = [\"a\"]\nevery = \"5m\"", "/check/0");
        let p = one(
            "[[check]]\nname = \"c\"\ntarget = \"http://127.0.0.1:1/\"\nevery = \"5m\"\nargv = [\"a\"]\n",
        );
        assert!(p.iter().any(|p| p.contains("argv is for cli")), "{p:?}");
        let p = one(
            "[[check]]\nname = \"c\"\ntarget = \"http://127.0.0.1:1/\"\nevery = \"5m\"\nexpect = { exit = 0 }\n",
        );
        assert!(
            p.iter()
                .any(|p| p.contains("expect.exit and expect.request are for cli")),
            "{p:?}"
        );
        let p = one(
            "[[refuse]]\nname = \"c\"\ntarget = { cli = \"app\" }\nargv = [\"a\"]\nexpect = { exit = 2 }\n",
        );
        assert!(
            p.iter().any(|p| p.contains("there is no one to refuse it")),
            "{p:?}"
        );
    }

    #[test]
    fn the_schema_examples_pass_and_the_invalid_cases_fail() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("schemas/monitors/monitor");
        for (sub, ok) in [("examples", true), ("invalid", false)] {
            let mut seen = 0;
            for e in std::fs::read_dir(dir.join(sub)).unwrap() {
                let p = e.unwrap().path();
                let v: serde_json::Value =
                    serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
                let problems = check(&v);
                assert_eq!(problems.is_empty(), ok, "{}: {problems:?}", p.display());
                seen += 1;
            }
            assert!(seen > 0, "no {sub}");
        }
    }
}
