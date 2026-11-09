//! One `coordinate` turn on the user's installed, unmodified `claude -p` (RFC 0120,
//! "What the runner runs" and "Resuming a session").
//!
//! Invariant 8: no patched copy, no wrapper that alters requests, no custom endpoint,
//! never `--bare`. [`build`] is the only place a `claude` argument vector and its sign-in
//! environment are made (a runner turn through [`build_args`], `iohr ask` and the
//! headless answer through their own [`Call`]s), and it ends in [`assert_honest`], which
//! fails on a flag that would break that.

use std::path::{Path, PathBuf};

use iohr_auth::Redacted;
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

/// The sign-in a `claude` call runs on (RFC 0214).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Auth {
    /// Whatever `claude` itself is signed in with (`claude`, then `/login`); the call
    /// gets no credential from iohr.
    OwnSignIn,
    /// An Anthropic API key from the credential store. It reaches the `claude` child as
    /// `ANTHROPIC_API_KEY` and nothing else: `CLAUDE_CODE_SUBPROCESS_ENV_SCRUB` keeps it
    /// out of the shells, hooks and MCP servers `claude` starts.
    ApiKey(Redacted<String>),
}

/// The variable `claude` reads an Anthropic API key from.
pub(crate) const API_KEY_VAR: &str = "ANTHROPIC_API_KEY";
/// Set beside the key: `claude` strips its credentials from every subprocess it starts.
pub(crate) const SUBPROCESS_SCRUB_VAR: &str = "CLAUDE_CODE_SUBPROCESS_ENV_SCRUB";

/// The environment a call on `auth` adds after the scrubbed one: nothing on its own
/// sign-in, the key and the subprocess scrub on a key. The value stays redacted until
/// the spawn.
pub(crate) fn auth_env(auth: &Auth) -> Vec<(&'static str, Redacted<String>)> {
    match auth {
        Auth::OwnSignIn => Vec::new(),
        Auth::ApiKey(key) => vec![
            (API_KEY_VAR, key.clone()),
            (SUBPROCESS_SCRUB_VAR, Redacted::new("1".to_owned())),
        ],
    }
}

/// How `claude -p` prints.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Output {
    /// One JSON result at the end.
    #[default]
    Json,
    /// JSON lines as the answer streams (`stream-json` needs `--verbose`).
    Stream,
}

/// What a call keeps of its session.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Keep<'a> {
    /// `--no-session-persistence`: nothing on disk.
    #[default]
    Nothing,
    /// `--session-id <id>`: a new session under that id.
    Fresh(&'a str),
    /// `--resume <id>`.
    Resume(&'a str),
}

/// The system prompt a call gives.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Prompt<'a> {
    /// Claude Code's own.
    #[default]
    Own,
    /// `--system-prompt`: in place of Claude Code's.
    Replace(&'a str),
    /// `--append-system-prompt`: after Claude Code's.
    Append(&'a str),
}

/// One `claude -p` call, as each caller asks for it; [`build`] makes it an argument
/// vector in one order.
#[derive(Debug, Clone, Default)]
pub(crate) struct Call<'a> {
    pub(crate) output: Output,
    pub(crate) model: Option<&'a str>,
    pub(crate) max_turns: Option<u32>,
    /// `--setting-sources <value>` (`""` is none of the person's settings).
    pub(crate) setting_sources: Option<&'a str>,
    /// `--strict-mcp-config`: no MCP server not named by `--mcp-config`.
    pub(crate) strict_mcp: bool,
    pub(crate) keep: Keep<'a>,
    pub(crate) prompt: Prompt<'a>,
    /// `--permission-mode <mode>`.
    pub(crate) permission_mode: Option<&'a str>,
    /// `--restricted`.
    pub(crate) restricted: bool,
    /// `--tools <list>` (`""` is no tools at all).
    pub(crate) tools: Option<String>,
    /// `--add-dir=<dir>` each (the `=` form: the list form would swallow what follows).
    pub(crate) add_dirs: Vec<String>,
    /// Flags a caller adds as they are (the broker's `--mcp-config` and its prompt tool).
    pub(crate) extra: Vec<String>,
    /// `--disallowedTools=<rule>` each, one flag a rule.
    pub(crate) disallowed: Vec<String>,
    /// `--allowedTools <rule>...`, last before the question, so the list takes nothing else.
    pub(crate) allowed: Vec<String>,
    /// `-- <question>`; `None` when the prompt goes on stdin.
    pub(crate) question: Option<&'a str>,
}

/// A call's arguments (without the program name) and what it adds to the environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Built {
    pub(crate) args: Vec<String>,
    pub(crate) env: Vec<(&'static str, Redacted<String>)>,
}

/// The argument vector and sign-in environment of one call: `-p`, the output, the model
/// and turn cap, the settings and MCP scope, the session, the system prompt, the
/// permission mode, the tools, the extra flags, the denies and the allows, then `--` and
/// the question. Everything before `--` is checked by [`assert_honest`].
pub(crate) fn build(call: &Call<'_>, auth: &Auth) -> Result<Built, ArgError> {
    let mut a: Vec<String> = vec!["-p".into(), "--output-format".into()];
    match call.output {
        Output::Json => a.push("json".into()),
        Output::Stream => {
            a.extend(["stream-json", "--verbose", "--include-partial-messages"].map(str::to_owned));
        }
    }
    if let Some(m) = call.model {
        a.extend(["--model".into(), m.to_owned()]);
    }
    if let Some(n) = call.max_turns {
        a.extend(["--max-turns".into(), n.to_string()]);
    }
    if let Some(s) = call.setting_sources {
        a.extend(["--setting-sources".into(), s.to_owned()]);
    }
    if call.strict_mcp {
        a.push("--strict-mcp-config".into());
    }
    match call.keep {
        Keep::Nothing => a.push("--no-session-persistence".into()),
        Keep::Fresh(id) => a.extend(["--session-id".into(), id.to_owned()]),
        Keep::Resume(id) => a.extend(["--resume".into(), id.to_owned()]),
    }
    match call.prompt {
        Prompt::Own => {}
        Prompt::Replace(p) => a.extend(["--system-prompt".into(), p.to_owned()]),
        Prompt::Append(p) => a.extend(["--append-system-prompt".into(), p.to_owned()]),
    }
    if let Some(m) = call.permission_mode {
        a.extend(["--permission-mode".into(), m.to_owned()]);
    }
    if call.restricted {
        a.push("--restricted".into());
    }
    if let Some(t) = &call.tools {
        a.extend(["--tools".into(), t.clone()]);
    }
    a.extend(call.add_dirs.iter().map(|d| format!("--add-dir={d}")));
    a.extend(call.extra.iter().cloned());
    a.extend(
        call.disallowed
            .iter()
            .map(|r| format!("--disallowedTools={r}")),
    );
    if !call.allowed.is_empty() {
        a.push("--allowedTools".into());
        a.extend(call.allowed.iter().cloned());
    }
    assert_honest(&a)?;
    if let Some(q) = call.question {
        a.extend(["--".into(), q.to_owned()]);
    }
    Ok(Built {
        args: a,
        env: auth_env(auth),
    })
}

/// The argument vector for `claude`, without the program name. The turn's prompt is
/// written to stdin, so it is not in the process list.
///
/// A resumed session keeps the system prompt it started with; only a fresh one is given
/// the pack. The sign-in environment is the spawn's ([`auth_env`] of the runner's
/// [`Auth`]), the same for every turn, so a turn's arguments never depend on it.
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
    let (keep, prompt) = match spec.session {
        Session::Resume(id) => (Keep::Resume(id), Prompt::Own),
        Session::Fresh(id) => (Keep::Fresh(id), Prompt::Replace(spec.system_prompt)),
    };
    build(
        &Call {
            output: Output::Json,
            model: Some(spec.model),
            max_turns: Some(spec.max_turns),
            setting_sources: Some(LEAN_SESSION[1]),
            strict_mcp: true,
            keep,
            prompt,
            allowed: spec.allowed_tools.to_vec(),
            ..Call::default()
        },
        &Auth::OwnSignIn,
    )
    .map(|b| b.args)
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

/// The provider this executor runs, as the service names it.
pub(crate) const PROVIDER: &str = "claude";

/// The model a review turn runs on when its pack names none: the service leaves the
/// critic's model to the runner, and a Critique wants the strongest one.
pub(crate) const CRITIC_MODEL: &str = "opus";

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

    /// RFC 0214: one builder for every `claude` call. Both sign-ins give the same
    /// arguments; only a key adds environment, the key and the subprocess scrub, and
    /// never as an argument or in what `Debug` prints.
    #[test]
    fn one_builder_gives_both_sign_ins_the_same_args_and_only_a_key_its_env() {
        const KEY: &str = "sk-ant-api03-UNIT-KEY-never-printed";
        let call = Call {
            output: Output::Stream,
            model: Some("sonnet"),
            max_turns: Some(3),
            prompt: Prompt::Append("role"),
            permission_mode: Some("plan"),
            disallowed: vec!["Read(./.env)".to_owned()],
            question: Some("what changed?"),
            ..Call::default()
        };
        let own = build(&call, &Auth::OwnSignIn).unwrap();
        let key = build(&call, &Auth::ApiKey(Redacted::new(KEY.to_owned()))).unwrap();
        assert_eq!(own.args, key.args);
        assert_eq!(own.env.len(), 0, "the own sign-in adds no environment");
        let env: Vec<(&str, &str)> = key
            .env
            .iter()
            .map(|(k, v)| (*k, v.expose().as_str()))
            .collect();
        assert_eq!(env, [(API_KEY_VAR, KEY), (SUBPROCESS_SCRUB_VAR, "1")]);
        assert!(!key.args.iter().any(|a| a.contains(KEY)));
        assert!(!format!("{key:?}").contains(KEY));
        assert_eq!(
            strs(&own.args)[..4],
            ["-p", "--output-format", "stream-json", "--verbose"]
        );
        let tail = &own.args[own.args.len() - 2..];
        assert_eq!(strs(tail), ["--", "what changed?"]);
        // Every caller goes through the guard, whatever it adds.
        let bad = Call {
            extra: vec!["--bare".to_owned()],
            ..call
        };
        assert_eq!(
            build(&bad, &Auth::OwnSignIn),
            Err(ArgError::Forbidden("--bare".to_owned()))
        );
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
        assert!(is_uuid(&a), "a new session id is not a uuid");
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

    /// Seen on a real turn (RFC 0120): the model tried a tool it lacks, then answered in
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
