//! One `coordinate` turn on the user's installed, unmodified `claude -p` (RFC 0120,
//! "What the runner runs" and "Resuming a session").
//!
//! Invariant 8: no patched copy, no wrapper that alters requests, no custom endpoint,
//! never `--bare`. [`build_args`] is the only place an argument vector is made, and it
//! ends in [`assert_honest`], which fails on a flag that would break that.

use std::path::{Path, PathBuf};

use serde_json::Value;

/// Flags that must never reach `claude`. `--bare` skips the user's own sign-in and
/// settings (invariant 8); the rest would point the binary at a key or an endpoint.
pub(crate) const FORBIDDEN_FLAGS: &[&str] = &[
    "--bare",
    "--api-key",
    "--auth-token",
    "--base-url",
    "--settings",
];

/// What keeps a turn's context small without `--bare` (which never reads the keychain
/// sign-in, so it is forbidden): the user's own settings (hooks, enabled plugins, their
/// MCP servers and skills) stay out, and so does any MCP server not named by
/// `--mcp-config`. Sign-in is untouched. Measured on `claude` 2.1.307 with one short
/// turn: 33,220 context tokens without these two flags, 15,054 with them. The
/// `project` and `local` sources still apply, so the working directory's own settings do.
/// `--disable-slash-commands` was measured too and made the context larger, so it is not here.
pub(crate) const LEAN_SESSION: &[&str] =
    &["--setting-sources", "project,local", "--strict-mcp-config"];

/// Why a turn's arguments were refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ArgError {
    /// The lease named no model: the cheapest-model rule is the service's to apply.
    NoModel,
    /// No turn cap: a turn never runs unbounded.
    NoMaxTurns,
    /// A value that would be read as a flag.
    FlagLike(String),
    /// A forbidden flag in the result (invariant 8).
    Forbidden(String),
    /// A session id that is not a UUID.
    BadSession(String),
}

impl std::fmt::Display for ArgError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoModel => f.write_str("the turn names no model"),
            Self::NoMaxTurns => f.write_str("the turn has no turn cap"),
            Self::FlagLike(v) => write!(f, "a value that reads as a flag: {v}"),
            Self::Forbidden(v) => write!(f, "invariant 8: {v} is never passed to claude"),
            Self::BadSession(v) => write!(f, "not a session id: {v}"),
        }
    }
}

/// Which session a turn uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Session {
    /// `--resume <id>`: this runner ran the thread's last turn and the session exists.
    Resume(String),
    /// `--session-id <new>`: everything needed is in the pack's `state`.
    Fresh(String),
}

impl Session {
    pub(crate) fn id(&self) -> &str {
        match self {
            Self::Resume(id) | Self::Fresh(id) => id,
        }
    }

    pub(crate) fn resumed(&self) -> bool {
        matches!(self, Self::Resume(_))
    }
}

/// What the service's turn asks for.
#[derive(Debug, Clone, Copy)]
pub(crate) struct TurnSpec<'a> {
    pub(crate) system_prompt: &'a str,
    pub(crate) model: &'a str,
    pub(crate) max_turns: u32,
    pub(crate) allowed_tools: &'a [String],
    pub(crate) session: &'a Session,
}

/// Resume only when this runner ran the previous turn, the session is on its disk and
/// its last measured context is not over `ceiling`; otherwise a fresh session with
/// `new_id`, started from the pack. A takeover runner can never resume another machine's
/// session. A session is never forked (`--fork-session` is not a flag this runner builds).
///
/// `last_context` is the context the provider measured at the end of the session's last
/// turn ([`context_tokens`]); `None` (never measured) does not block a resume, since the
/// turn that follows measures it. A `ceiling` of 0 means the service sent none.
pub(crate) fn choose_session(
    resume_session: &str,
    resume_runner_id: &str,
    my_runner_id: &str,
    session_exists: impl Fn(&str) -> bool,
    last_context: impl Fn(&str) -> Option<u64>,
    ceiling: u32,
    new_id: impl FnOnce() -> String,
) -> Session {
    let over = ceiling > 0 && last_context(resume_session).is_some_and(|n| n > u64::from(ceiling));
    if !resume_session.is_empty()
        && !my_runner_id.is_empty()
        && resume_runner_id == my_runner_id
        && session_exists(resume_session)
        && !over
    {
        Session::Resume(resume_session.to_owned())
    } else {
        Session::Fresh(new_id())
    }
}

/// The context a turn ended with, as the provider measured it in the result's `usage`:
/// input + cache read + cache creation tokens.
pub(crate) fn context_tokens(usage: &Value) -> u64 {
    [
        "input_tokens",
        "cache_read_input_tokens",
        "cache_creation_input_tokens",
    ]
    .iter()
    .map(|k| usage.get(k).and_then(Value::as_u64).unwrap_or(0))
    .fold(0, u64::saturating_add)
}

/// Remembers a session's last measured context in `dir` (one small file per session).
pub(crate) fn record_context(dir: &Path, id: &str, tokens: u64) {
    if !is_uuid(id) {
        return;
    }
    if std::fs::create_dir_all(dir).is_ok() {
        let _ = std::fs::write(dir.join(id), tokens.to_string());
    }
}

/// What [`record_context`] remembered, if anything.
pub(crate) fn recorded_context(dir: &Path, id: &str) -> Option<u64> {
    if !is_uuid(id) {
        return None;
    }
    std::fs::read_to_string(dir.join(id))
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// The argument vector for `claude`, without the program name. The turn's prompt is
/// written to stdin, so it is not in the process list.
///
/// A resumed session keeps the system prompt it started with; only a fresh one is given
/// the pack.
pub(crate) fn build_args(spec: &TurnSpec<'_>) -> Result<Vec<String>, ArgError> {
    if spec.model.trim().is_empty() {
        return Err(ArgError::NoModel);
    }
    if spec.max_turns == 0 {
        return Err(ArgError::NoMaxTurns);
    }
    let id = spec.session.id();
    if !is_uuid(id) {
        return Err(ArgError::BadSession(id.to_owned()));
    }
    for v in std::iter::once(spec.model).chain(spec.allowed_tools.iter().map(String::as_str)) {
        if v.starts_with('-') || v.trim().is_empty() {
            return Err(ArgError::FlagLike(v.to_owned()));
        }
    }
    let mut a: Vec<String> = vec![
        "-p".into(),
        "--output-format".into(),
        "json".into(),
        "--model".into(),
        spec.model.to_owned(),
        "--max-turns".into(),
        spec.max_turns.to_string(),
    ];
    a.extend(LEAN_SESSION.iter().map(|f| (*f).to_owned()));
    match spec.session {
        Session::Resume(id) => a.extend(["--resume".into(), id.clone()]),
        Session::Fresh(id) => {
            a.extend(["--session-id".into(), id.clone()]);
            a.extend(["--system-prompt".into(), spec.system_prompt.to_owned()]);
        }
    }
    if !spec.allowed_tools.is_empty() {
        // Last, so the variadic flag cannot take anything after it.
        a.push("--allowedTools".into());
        a.extend(spec.allowed_tools.iter().cloned());
    }
    assert_honest(&a)?;
    Ok(a)
}

/// Fails when `args` holds a forbidden flag, alone or as `--flag=value`.
pub(crate) fn assert_honest(args: &[String]) -> Result<(), ArgError> {
    for a in args {
        let flag = a.split('=').next().unwrap_or(a);
        if FORBIDDEN_FLAGS.contains(&flag) {
            return Err(ArgError::Forbidden(flag.to_owned()));
        }
    }
    Ok(())
}

/// Lowercase hex of `bytes`.
pub(crate) fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

fn is_uuid(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 36
        && b.iter().enumerate().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => *c == b'-',
            _ => c.is_ascii_hexdigit(),
        })
}

/// A random (version 4) UUID, the shape `--session-id` takes.
pub(crate) fn new_session_id() -> String {
    let mut b = [0_u8; 16];
    // A failing system random source is not recoverable here; the id then repeats
    // nothing it could clash with because claude refuses an existing id.
    let _ = getrandom::fill(&mut b);
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h = hex(&b);
    format!(
        "{}-{}-{}-{}-{}",
        &h[..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..]
    )
}

/// Claude Code's own directory: `CLAUDE_CONFIG_DIR`, else `~/.claude`. Only its listing is
/// read, to see whether a session exists; never a credential file.
pub(crate) fn claude_dir(
    config_dir_var: Option<PathBuf>,
    home: Option<PathBuf>,
) -> Option<PathBuf> {
    config_dir_var.or_else(|| home.map(|h| h.join(".claude")))
}

/// Whether `<root>/projects/*/<id>.jsonl` exists.
pub(crate) fn session_exists_in(root: &Path, id: &str) -> bool {
    if !is_uuid(id) {
        return false;
    }
    let Ok(dirs) = std::fs::read_dir(root.join("projects")) else {
        return false;
    };
    dirs.flatten()
        .any(|d| d.path().join(format!("{id}.jsonl")).is_file())
}

/// How a turn ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// A reply in the shape.
    Replied,
    /// A final message that is not the shape: still a reply, a quality signal.
    Unshaped,
    CapHit,
    LimitHit,
    LoginExpired,
    RefusedTool,
    Failed,
}

impl Outcome {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Replied => "replied",
            Self::Unshaped => "unshaped",
            Self::CapHit => "cap_hit",
            Self::LimitHit => "limit_hit",
            Self::LoginExpired => "login_expired",
            Self::RefusedTool => "refused_tool",
            Self::Failed => "failed",
        }
    }
}

/// `say`, `state`, `actions`, `evidence`, `left`.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Reply {
    pub(crate) say: String,
    pub(crate) state: String,
    pub(crate) actions: Vec<Value>,
    pub(crate) evidence: Vec<Value>,
    pub(crate) left: Vec<Value>,
}

/// What the provider's JSON result says.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Parsed {
    pub(crate) outcome: Outcome,
    pub(crate) reply: Reply,
    pub(crate) session_id: String,
    pub(crate) num_turns: u64,
    pub(crate) duration_ms: u64,
    pub(crate) total_cost_usd: Option<f64>,
    pub(crate) usage: Value,
    /// Tools Claude Code was refused, for a `refused_tool` outcome.
    pub(crate) denied_tools: Vec<String>,
    /// For `limit_hit`: when the plan window resets (RFC 3339), if the message said.
    pub(crate) resets_at: String,
}

/// The reset time of a usage-limit message, which Claude Code ends with
/// `|<unix seconds>`; empty when there is none.
pub(crate) fn limit_resets_at(text: &str) -> String {
    let Some((_, tail)) = text.trim().rsplit_once('|') else {
        return String::new();
    };
    let secs: String = tail
        .trim()
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    secs.parse::<i64>()
        .ok()
        .and_then(|t| time::OffsetDateTime::from_unix_timestamp(t).ok())
        .and_then(|d| {
            d.format(&time::format_description::well_known::Rfc3339)
                .ok()
        })
        .unwrap_or_default()
}

/// Reads `claude -p --output-format json`'s output. `exited_ok` is the exit status.
pub(crate) fn parse_result(stdout: &str, exited_ok: bool) -> Parsed {
    let failed = |outcome| Parsed {
        outcome,
        reply: Reply::default(),
        session_id: String::new(),
        num_turns: 0,
        duration_ms: 0,
        total_cost_usd: None,
        usage: Value::Null,
        denied_tools: Vec::new(),
        resets_at: String::new(),
    };
    let Ok(v) = serde_json::from_str::<Value>(stdout.trim()) else {
        return failed(Outcome::Failed);
    };
    // Older and verbose forms give an array of events; the result is the last one.
    let v = match v {
        Value::Array(items) => items
            .into_iter()
            .rev()
            .find(|i| i.get("type").and_then(Value::as_str) == Some("result"))
            .unwrap_or(Value::Null),
        other => other,
    };
    let text = v.get("result").and_then(Value::as_str).unwrap_or_default();
    let subtype = v.get("subtype").and_then(Value::as_str).unwrap_or_default();
    let is_error = v.get("is_error").and_then(Value::as_bool).unwrap_or(false);
    let denied: Vec<String> = v
        .get("permission_denials")
        .and_then(Value::as_array)
        .map(|d| {
            d.iter()
                .filter_map(|x| x.get("tool_name").and_then(Value::as_str))
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let lower = text.to_ascii_lowercase();
    let outcome = if subtype == "error_max_turns" {
        Outcome::CapHit
    } else if is_error
        && (lower.contains("limit") && (lower.contains("reset") || lower.contains("usage")))
    {
        Outcome::LimitHit
    } else if is_error
        && (lower.contains("/login")
            || lower.contains("not logged in")
            || lower.contains("authentication"))
    {
        Outcome::LoginExpired
    } else if !is_error && exited_ok && parse_reply(text).is_some() {
        // A shaped answer is the turn's outcome even when a tool was denied on the way;
        // the denials still go in the report (`denied_tools`).
        Outcome::Replied
    } else if !denied.is_empty() {
        Outcome::RefusedTool
    } else if is_error || !exited_ok || v.is_null() {
        Outcome::Failed
    } else {
        Outcome::Unshaped
    };
    let reply = parse_reply(text).unwrap_or_else(|| Reply {
        say: text.to_owned(),
        ..Reply::default()
    });
    Parsed {
        outcome,
        reply,
        session_id: v
            .get("session_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        num_turns: v.get("num_turns").and_then(Value::as_u64).unwrap_or(0),
        duration_ms: v.get("duration_ms").and_then(Value::as_u64).unwrap_or(0),
        total_cost_usd: v.get("total_cost_usd").and_then(Value::as_f64),
        usage: v.get("usage").cloned().unwrap_or(Value::Null),
        denied_tools: denied,
        resets_at: if outcome == Outcome::LimitHit {
            limit_resets_at(text)
        } else {
            String::new()
        },
    }
}

/// The reply object in a final message: the whole text, a fenced block, or the outermost
/// braces. It must have a `say`.
pub(crate) fn parse_reply(text: &str) -> Option<Reply> {
    let t = text.trim();
    let mut candidates: Vec<&str> = vec![t];
    if let Some(start) = t.find("```") {
        let rest = &t[start + 3..];
        let rest = rest.strip_prefix("json").unwrap_or(rest);
        if let Some(end) = rest.find("```") {
            candidates.push(rest[..end].trim());
        }
    }
    if let (Some(a), Some(b)) = (t.find('{'), t.rfind('}'))
        && a < b
    {
        candidates.push(&t[a..=b]);
    }
    candidates.into_iter().find_map(|c| {
        let v: Value = serde_json::from_str(c).ok()?;
        let say = v.get("say")?.as_str()?.to_owned();
        let list = |k: &str| {
            v.get(k)
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        };
        Some(Reply {
            say,
            state: v
                .get("state")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            actions: list("actions"),
            evidence: list("evidence"),
            left: list("left"),
        })
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]
    use super::*;

    const ID: &str = "0b6c1c3e-6a1f-4a64-9a2e-5d3f0b8c9d11";
    const OLD: &str = "7d2f9a10-1111-4222-8333-444455556666";

    fn tools(t: &[&str]) -> Vec<String> {
        t.iter().map(|s| (*s).to_owned()).collect()
    }

    fn args(session: &Session, t: &[String]) -> Result<Vec<String>, ArgError> {
        build_args(&TurnSpec {
            system_prompt: "PACK",
            model: "sonnet",
            max_turns: 12,
            allowed_tools: t,
            session,
        })
    }

    fn strs(a: &[String]) -> Vec<&str> {
        a.iter().map(String::as_str).collect()
    }

    /// The argument table: each row is a turn, and the exact vector it must produce.
    #[test]
    fn a_limit_message_gives_its_reset_time() {
        assert_eq!(
            limit_resets_at("Claude usage limit reached|1760270400"),
            "2025-10-12T12:00:00Z"
        );
        assert_eq!(limit_resets_at("limit reached"), "");
        assert_eq!(limit_resets_at("x|soon"), "");
    }

    #[test]
    fn every_turn_leaves_the_users_settings_and_mcp_out_but_never_uses_bare() {
        for session in [Session::Fresh(ID.into()), Session::Resume(OLD.into())] {
            let built = args(&session, &[]).unwrap();
            let got = strs(&built);
            assert!(
                got.windows(2)
                    .any(|w| w == ["--setting-sources", "project,local"])
            );
            assert!(got.contains(&"--strict-mcp-config"));
            assert!(!got.contains(&"--bare"));
            assert!(!got.contains(&"--settings"));
        }
    }

    #[test]
    fn argument_table() {
        struct Row {
            name: &'static str,
            session: Session,
            tools: Vec<String>,
            want: Vec<&'static str>,
        }
        let rows = vec![
            Row {
                name: "fresh with tools",
                session: Session::Fresh(ID.into()),
                tools: tools(&["Read", "Bash(git log:*)"]),
                want: vec![
                    "-p",
                    "--output-format",
                    "json",
                    "--model",
                    "sonnet",
                    "--max-turns",
                    "12",
                    "--setting-sources",
                    "project,local",
                    "--strict-mcp-config",
                    "--session-id",
                    ID,
                    "--system-prompt",
                    "PACK",
                    "--allowedTools",
                    "Read",
                    "Bash(git log:*)",
                ],
            },
            Row {
                name: "resume carries no pack",
                session: Session::Resume(OLD.into()),
                tools: tools(&["Read"]),
                want: vec![
                    "-p",
                    "--output-format",
                    "json",
                    "--model",
                    "sonnet",
                    "--max-turns",
                    "12",
                    "--setting-sources",
                    "project,local",
                    "--strict-mcp-config",
                    "--resume",
                    OLD,
                    "--allowedTools",
                    "Read",
                ],
            },
            Row {
                name: "no tools, no flag",
                session: Session::Fresh(ID.into()),
                tools: vec![],
                want: vec![
                    "-p",
                    "--output-format",
                    "json",
                    "--model",
                    "sonnet",
                    "--max-turns",
                    "12",
                    "--setting-sources",
                    "project,local",
                    "--strict-mcp-config",
                    "--session-id",
                    ID,
                    "--system-prompt",
                    "PACK",
                ],
            },
        ];
        for r in rows {
            let got = args(&r.session, &r.tools).unwrap();
            assert_eq!(strs(&got), r.want, "{}", r.name);
            // Whatever the row, invariant 8 holds on the output.
            assert!(!got.iter().any(|a| a == "--bare"), "{}", r.name);
            assert!(assert_honest(&got).is_ok(), "{}", r.name);
            // A coordinate turn is resumable later: never ephemeral.
            assert!(
                !got.iter().any(|a| a == "--no-session-persistence"),
                "{}",
                r.name
            );
        }
    }

    fn refuse(
        model: &str,
        max_turns: u32,
        t: &[String],
        s: &Session,
    ) -> Result<Vec<String>, ArgError> {
        build_args(&TurnSpec {
            system_prompt: "P",
            model,
            max_turns,
            allowed_tools: t,
            session: s,
        })
    }

    #[test]
    fn refusals_table() {
        let fresh = Session::Fresh(ID.into());
        let none: Vec<String> = vec![];
        assert_eq!(refuse("", 3, &none, &fresh), Err(ArgError::NoModel));
        assert_eq!(
            refuse("sonnet", 0, &none, &fresh),
            Err(ArgError::NoMaxTurns)
        );
        assert_eq!(
            refuse("--bare", 3, &none, &fresh),
            Err(ArgError::FlagLike("--bare".into()))
        );
        // A lease cannot smuggle a flag in as a tool.
        assert_eq!(
            refuse("sonnet", 3, &tools(&["--bare"]), &fresh),
            Err(ArgError::FlagLike("--bare".into()))
        );
        let bad = Session::Fresh("not-a-uuid".into());
        assert!(matches!(
            refuse("sonnet", 3, &none, &bad),
            Err(ArgError::BadSession(_))
        ));
    }

    #[test]
    fn bare_and_endpoint_flags_fail_the_guard() {
        for flag in FORBIDDEN_FLAGS {
            assert_eq!(
                assert_honest(&tools(&["-p", flag])),
                Err(ArgError::Forbidden((*flag).to_owned()))
            );
            assert_eq!(
                assert_honest(&tools(&[&format!("{flag}=x")])),
                Err(ArgError::Forbidden((*flag).to_owned()))
            );
        }
        assert!(assert_honest(&tools(&["--model", "sonnet"])).is_ok());
    }

    #[test]
    fn resume_versus_fresh() {
        let exists = |id: &str| id == OLD;
        let fresh = || ID.to_owned();
        // Same runner, session on disk: resume.
        assert_eq!(
            choose_session(OLD, "run_a", "run_a", exists, |_| None, 0, fresh),
            Session::Resume(OLD.into())
        );
        // A takeover runner never resumes another machine's session.
        assert_eq!(
            choose_session(OLD, "run_a", "run_b", exists, |_| None, 0, fresh),
            Session::Fresh(ID.into())
        );
        // Same runner, but the session is gone (cleaned, new disk).
        assert_eq!(
            choose_session("gone", "run_a", "run_a", exists, |_| None, 0, fresh),
            Session::Fresh(ID.into())
        );
        // First turn of a thread.
        assert_eq!(
            choose_session("", "", "run_a", exists, |_| None, 0, fresh),
            Session::Fresh(ID.into())
        );
        // An unregistered runner id matches nothing, not even an empty one.
        assert_eq!(
            choose_session(OLD, "", "", exists, |_| None, 0, fresh),
            Session::Fresh(ID.into())
        );
    }

    #[test]
    fn a_session_over_the_context_ceiling_is_never_resumed() {
        let exists = |id: &str| id == OLD;
        let fresh = || ID.to_owned();
        let ceiling = 120_000;
        // At the ceiling: still resumed. One token over: fresh, from the pack.
        assert_eq!(
            choose_session(
                OLD,
                "run_a",
                "run_a",
                exists,
                |_| Some(120_000),
                ceiling,
                fresh
            ),
            Session::Resume(OLD.into())
        );
        assert_eq!(
            choose_session(
                OLD,
                "run_a",
                "run_a",
                exists,
                |_| Some(120_001),
                ceiling,
                fresh
            ),
            Session::Fresh(ID.into())
        );
        // Never measured: the turn after it measures; no ceiling sent: no block.
        assert_eq!(
            choose_session(OLD, "run_a", "run_a", exists, |_| None, ceiling, fresh),
            Session::Resume(OLD.into())
        );
        assert_eq!(
            choose_session(OLD, "run_a", "run_a", exists, |_| Some(900_000), 0, fresh),
            Session::Resume(OLD.into())
        );
        // A fresh session is never a fork: the flag is not one this runner builds.
        let args = build_args(&TurnSpec {
            system_prompt: "s",
            model: "sonnet",
            max_turns: 3,
            allowed_tools: &[],
            session: &Session::Fresh(ID.into()),
        })
        .unwrap();
        assert!(!args.iter().any(|a| a == "--fork-session"));
    }

    #[test]
    fn context_is_input_plus_cache_read_plus_cache_creation_and_is_remembered() {
        let u = serde_json::json!({"input_tokens": 10, "cache_read_input_tokens": 100_000,
            "cache_creation_input_tokens": 5_000, "output_tokens": 999});
        assert_eq!(context_tokens(&u), 105_010);
        assert_eq!(context_tokens(&Value::Null), 0);
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(recorded_context(dir.path(), OLD), None);
        record_context(dir.path(), OLD, 105_010);
        assert_eq!(recorded_context(dir.path(), OLD), Some(105_010));
        assert_eq!(recorded_context(dir.path(), "../x"), None);
    }

    #[test]
    fn a_session_exists_when_its_transcript_is_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!session_exists_in(dir.path(), OLD));
        let proj = dir.path().join("projects").join("-some-dir");
        std::fs::create_dir_all(&proj).unwrap();
        std::fs::write(proj.join(format!("{OLD}.jsonl")), "{}\n").unwrap();
        assert!(session_exists_in(dir.path(), OLD));
        assert!(!session_exists_in(dir.path(), ID));
        assert!(!session_exists_in(dir.path(), "../x"));
    }

    #[test]
    fn new_session_ids_are_uuids_and_differ() {
        let a = new_session_id();
        assert!(is_uuid(&a), "{a}");
        assert_ne!(a, new_session_id());
        assert_eq!(&a[14..15], "4");
    }

    #[test]
    fn results_are_read_into_outcomes() {
        let shaped = serde_json::json!({
            "type": "result", "subtype": "success", "is_error": false,
            "result": "{\"say\":\"done\",\"state\":\"s\",\"actions\":[{\"kind\":\"x\"}],\"evidence\":[\"e1\"],\"left\":[]}",
            "session_id": ID, "num_turns": 3, "duration_ms": 900, "total_cost_usd": 0.12,
            "usage": {"input_tokens": 5}
        });
        let p = parse_result(&shaped.to_string(), true);
        assert_eq!(p.outcome, Outcome::Replied);
        assert_eq!(p.reply.say, "done");
        assert_eq!(p.reply.actions.len(), 1);
        assert_eq!(
            (p.num_turns, p.duration_ms, p.session_id.as_str()),
            (3, 900, ID)
        );
        assert_eq!(p.total_cost_usd, Some(0.12));

        let fenced = serde_json::json!({"type":"result","is_error":false,
            "result":"Here:\n```json\n{\"say\":\"hi\"}\n```"});
        assert_eq!(
            parse_result(&fenced.to_string(), true).outcome,
            Outcome::Replied
        );

        let plain = serde_json::json!({"type":"result","is_error":false,"result":"just text"});
        let p = parse_result(&plain.to_string(), true);
        assert_eq!(p.outcome, Outcome::Unshaped);
        assert_eq!(p.reply.say, "just text");
        assert_eq!(p.reply.actions.len(), 0);

        let cap = serde_json::json!({"type":"result","subtype":"error_max_turns","is_error":true,"result":""});
        assert_eq!(
            parse_result(&cap.to_string(), false).outcome,
            Outcome::CapHit
        );

        let limit = serde_json::json!({"type":"result","is_error":true,
            "result":"Claude usage limit reached. Your limit will reset at 5pm"});
        assert_eq!(
            parse_result(&limit.to_string(), false).outcome,
            Outcome::LimitHit
        );

        let login = serde_json::json!({"type":"result","is_error":true,
            "result":"Please run /login"});
        assert_eq!(
            parse_result(&login.to_string(), false).outcome,
            Outcome::LoginExpired
        );

        let denied = serde_json::json!({"type":"result","is_error":false,"result":"x",
            "permission_denials":[{"tool_name":"Bash","tool_input":{}}]});
        let p = parse_result(&denied.to_string(), true);
        assert_eq!(p.outcome, Outcome::RefusedTool);
        assert_eq!(p.denied_tools, vec!["Bash"]);

        assert_eq!(parse_result("not json", false).outcome, Outcome::Failed);
        let arr = serde_json::json!([{"type":"system"},{"type":"result","is_error":false,"result":"{\"say\":\"a\"}"}]);
        assert_eq!(
            parse_result(&arr.to_string(), true).outcome,
            Outcome::Replied
        );
    }

    /// `job_01M5N5FG1B1FVJZGKVW6XPYW94`: the model tried a tool it lacks, then answered in
    /// the reply shape. The answer is the turn's outcome; the denial is still reported.
    #[test]
    fn a_shaped_reply_after_a_denied_tool_is_replied_and_names_the_denial() {
        let v = serde_json::json!({"type":"result","is_error":false,
            "result":"{\"say\":\"done\",\"actions\":[]}",
            "permission_denials":[{"tool_name":"Bash","tool_input":{}}]});
        let p = parse_result(&v.to_string(), true);
        assert_eq!(p.outcome, Outcome::Replied);
        assert_eq!(p.reply.say, "done");
        assert_eq!(p.denied_tools, vec!["Bash"]);
        // A failed run with a shaped text is still not a reply.
        let failed = serde_json::json!({"type":"result","is_error":false,
            "result":"{\"say\":\"done\"}",
            "permission_denials":[{"tool_name":"Bash","tool_input":{}}]});
        assert_eq!(
            parse_result(&failed.to_string(), false).outcome,
            Outcome::RefusedTool
        );
    }

    #[test]
    fn the_older_array_form_still_parses() {
        assert_eq!(parse_result("not json", false).outcome, Outcome::Failed);
        let arr = serde_json::json!([{"type":"system"},{"type":"result","is_error":false,"result":"{\"say\":\"a\"}"}]);
        assert_eq!(
            parse_result(&arr.to_string(), true).outcome,
            Outcome::Replied
        );
    }
}
