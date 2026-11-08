//! `iohr asks`: the asks agents file for you, in the terminal (RFC 0204, ADR 0215).
//!
//! `list` and `show` stay live on a terminal: the account's `approval.changed` events (or one
//! ask's `Watch` stream) say something moved and the screen is drawn again; a pipe, `--json`
//! and `--once` print once. `deny` and `question` are decisions that need no key. `approve`
//! needs the proof of a device key: a Mac with an enrolled Secure Enclave key signs the asks
//! its tier allows after Touch ID (`key`, RFC 0113); a merge, a roll, a migration and an
//! auto-approve rule, and every machine that is not a Mac, hand it to the phone, so
//! `approve` says so and waits until the phone has used it (ADR 0142, ADR 0215).
//!
//! The text of an ask is other people's text: it is printed to stdout for the person who
//! asked to read it, with control characters dropped so it cannot move the cursor, and it is
//! never written to a log, an error or stderr. The API client logs method, path and status
//! only.

use std::fmt::Write as _;
use std::time::Duration;

use console::{Term, measure_text_width, truncate_str};
use serde_json::{Value, json};

use crate::Env;
use crate::cli::{AsksCommand, Global};
use crate::context::{Session, account, quiet_session, session};
use crate::error::Error;
use crate::mac_sign;
use crate::output::{Out, table_text};
use crate::prompt;
use crate::runner::approvals::{is_approval_id, note_text};
use crate::runner::service::{HttpService, ServiceError};
use crate::ui::{Choice, Paint, Ui};

use super::{day, headline, str_of};

mod key;

/// The statuses the service keeps (`approvals.proto`), besides the two groups below.
const STATUSES: &[&str] = &[
    "pending",
    "question",
    "approved",
    "denied",
    "in_progress",
    "done",
    "failed",
    "stale",
];

const SIGN_IN_HINT: &str =
    "Asks belong to signed-in people: run `iohr login`; an API key cannot use them.";

/// Why a live view returned.
#[derive(Debug)]
pub(crate) enum Ended {
    /// It was asked to stop, or ran its rounds.
    Done,
    /// The service refuses the stream (the credential has no events scope, an older
    /// service): the caller prints once instead.
    NoStream(ServiceError),
}

/// How the live views wait between a stream that ended and the next one, and how many times
/// they do it (`None` is until Ctrl-C; a test passes a number).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Pace {
    pub(crate) backoff: Duration,
    pub(crate) rounds: Option<usize>,
}

const LIVE: Pace = Pace {
    backoff: Duration::from_secs(3),
    rounds: None,
};

pub(crate) async fn run(g: &Global, env: &Env, cmd: AsksCommand, out: Out) -> Result<(), Error> {
    // The session is made per command: `count --cache` must not touch the credential (a
    // Keychain read) while its cache is fresh.
    match cmd {
        AsksCommand::Count { cache, account } => count(g, env, cache, account, out).await,
        AsksCommand::List {
            status,
            kind,
            once,
            limit,
            account,
        } => {
            let s = session(g, env).await?;
            list(&s, &status, kind.as_deref(), once, limit, account, out).await
        }
        AsksCommand::Show { id, once, account } => {
            let s = session(g, env).await?;
            show(&s, &id, once, account, out).await
        }
        AsksCommand::Decide { id, account } => {
            let s = session(g, env).await?;
            decide(g, &s, &id, account, out).await
        }
        AsksCommand::Deny { id, note, account } => {
            let s = session(g, env).await?;
            let note = note.as_deref().map(note_text).unwrap_or_default();
            deny(&s, &id, &note, account, out).await
        }
        AsksCommand::Question { id, note, account } => {
            let s = session(g, env).await?;
            let note = match note.as_deref().map(note_text) {
                Some(n) if !n.is_empty() => n,
                _ => ask_question_text(out)?,
            };
            question(&s, &id, &note, account, out).await
        }
        AsksCommand::Approve { id, once, account } => {
            let s = session(g, env).await?;
            approve(g, &s, &id, once, account, out).await
        }
        AsksCommand::Key { command } => key::run(g, env, command, out).await,
    }
}

// ---- count ----

/// How long a cached count is current: the shell hook asks again after this.
const COUNT_TTL: Duration = Duration::from_secs(30);

/// The prompt's cache: `~/.inorbit/term/asks-count`, the number and nothing else (no ask's
/// text, id or title ever goes in it). `IOHR_ASKS_COUNT_FILE` moves it (tests, odd homes).
pub(crate) fn count_cache_path() -> Option<std::path::PathBuf> {
    if let Some(p) = std::env::var_os("IOHR_ASKS_COUNT_FILE").filter(|p| !p.is_empty()) {
        return Some(p.into());
    }
    etcetera::home_dir()
        .ok()
        .map(|h| h.join(".inorbit").join("term").join("asks-count"))
}

/// Whether the cache file was written less than [`COUNT_TTL`] ago.
fn count_cache_fresh(path: &std::path::Path) -> bool {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|age| age < COUNT_TTL)
}

/// Writes the count whole: a temporary file beside the cache, then a rename, so the prompt
/// never reads half a number. The folder is made owner-only when it is new.
pub(crate) fn write_count_cache(path: &std::path::Path, count: usize) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(".asks-count.{}.tmp", std::process::id()));
    std::fs::write(&tmp, format!("{count}\n"))?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// How long the `?` card waits for the count before it shows none: the card never holds up
/// for a slow network.
const WAITING_SECS: u64 = 3;

/// The asks that wait for the owner, as the `?` card, `asks count` and the prompt's cache
/// all see them. `count` is the asks whose turn it is the person's; `only` is the title of
/// the one ask when exactly one waits, so the card can name it (kept in memory, never in the
/// cache file, a log or the JSON of `asks count`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Waiting {
    pub(crate) count: usize,
    pub(crate) only: Option<String>,
}

impl Waiting {
    /// The one rule for "waiting for you": an open ask is the person's turn only while it is
    /// `pending`. A `question` is an ask the person already answered with a note, so the turn
    /// is the agent's and it is left out (it still shows in `asks list`).
    fn of(rows: &[Value]) -> Self {
        let mine: Vec<&Value> = rows
            .iter()
            .filter(|a| str_of(a, "status") == "pending")
            .collect();
        let only = match mine.as_slice() {
            [one] => Some(headline(str_of(one, "title"), 200)).filter(|t| !t.is_empty()),
            _ => None,
        };
        Self {
            count: mine.len(),
            only,
        }
    }
}

/// The asks waiting for the signed-in account, for the one line under a `?` card:
/// `Some` only when there is one or more. Signed out, offline or slow is `None`: the card
/// says nothing then. It refreshes the prompt's cache, so the menu's Asks entry is current.
pub(crate) async fn waiting(g: &Global, env: &Env) -> Option<Waiting> {
    let fetched = tokio::time::timeout(
        Duration::from_secs(WAITING_SECS),
        fetch_waiting(g, env, None),
    )
    .await
    .ok()?
    .ok()?;
    if let Some(path) = count_cache_path() {
        let _ = write_count_cache(&path, fetched.count);
    }
    (fetched.count > 0).then_some(fetched)
}

/// The count the prompt's cache holds now (none when it has none, or it is unreadable): the
/// `?` menu reads it to offer the Asks entry without a call of its own.
pub(crate) fn cached_count() -> usize {
    count_cache_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| t.trim().parse().ok())
        .unwrap_or(0)
}

/// "3 asks", "1 ask", nothing for none.
fn count_text(n: usize) -> String {
    match n {
        0 => String::new(),
        1 => "1 ask\n".to_owned(),
        n => format!("{n} asks\n"),
    }
}

async fn count(
    g: &Global,
    env: &Env,
    cache: bool,
    account_flag: Option<String>,
    out: Out,
) -> Result<(), Error> {
    let path = if cache {
        let path = count_cache_path()
            .ok_or_else(|| Error::Usage("--cache needs a home folder to write to".into()))?;
        if count_cache_fresh(&path) {
            return Ok(());
        }
        Some(path)
    } else {
        None
    };
    let fetched = fetch_waiting(g, env, account_flag).await.map(|w| w.count);
    let Some(n) = settle(path.as_deref(), fetched)? else {
        return Ok(());
    };
    if out.json {
        Out::print_json(&json!({ "count": n }));
    } else {
        Out::raw(count_text(n).as_bytes());
    }
    Ok(())
}

/// What a fetch leaves in the cache and what `count` prints. `Ok(None)` is "print nothing".
///
/// The credential store refusing to ask (`Error::StoreWouldAsk`, see `QuietStore`) is not a
/// failure of the count: with `--cache` the last number stays, its file is touched so the
/// 30 s age still limits the next try, and nothing is printed, stdout or stderr, so a
/// prompt's background refresh never says anything. Without `--cache` the error is the
/// answer (it says what to do). Any other failed refresh (signed out, offline) leaves no
/// number: the prompt shows nothing rather than a stale one.
fn settle(
    path: Option<&std::path::Path>,
    fetched: Result<usize, Error>,
) -> Result<Option<usize>, Error> {
    match (path, fetched) {
        (Some(path), Ok(n)) => {
            let _ = write_count_cache(path, n);
            Ok(Some(n))
        }
        (Some(path), Err(Error::StoreWouldAsk(_))) => {
            let last = std::fs::read_to_string(path)
                .ok()
                .and_then(|t| t.trim().parse::<usize>().ok())
                .unwrap_or(0);
            let _ = write_count_cache(path, last);
            Ok(None)
        }
        (Some(path), Err(e)) => {
            let _ = write_count_cache(path, 0);
            Err(e)
        }
        (None, fetched) => fetched.map(Some),
    }
}

/// The asks waiting for you: the same call as `list` (pending and question), narrowed to
/// the ones whose turn is yours by [`Waiting::of`], the only place that decides it.
///
/// It reads the credential through the quiet store: the prompt and the shell hook run it in
/// the background, where a Keychain dialog or a wait nobody sees is the wrong answer.
async fn fetch_waiting(
    g: &Global,
    env: &Env,
    account_flag: Option<String>,
) -> Result<Waiting, Error> {
    Ok(Waiting::of(&fetch_open(g, env, account_flag).await?))
}

/// The open asks, read through the quiet store (see [`fetch_waiting`]).
async fn fetch_open(
    g: &Global,
    env: &Env,
    account_flag: Option<String>,
) -> Result<Vec<Value>, Error> {
    let s = quiet_session(g, env).await?;
    let svc = service(&s, None, account_flag)?;
    svc.list_approvals("open", "").await.map_err(scoped)
}

/// An ask waiting for the person, as the terminal panel lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PanelAsk {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) kind: String,
    pub(crate) created_at: String,
    /// The name the asking agent gives itself: how a workload that waits on this ask is found.
    pub(crate) agent: String,
}

/// The asks that are the person's turn (the same rule as the count: `pending` only), oldest
/// first as the service returns them. The terminal panel's list.
pub(crate) async fn pending_asks(g: &Global, env: &Env) -> Result<Vec<PanelAsk>, Error> {
    let rows = fetch_open(g, env, None).await?;
    Ok(rows
        .iter()
        .filter(|a| str_of(a, "status") == "pending")
        .map(|a| PanelAsk {
            id: str_of(a, "approval_id").to_owned(),
            title: headline(str_of(a, "title"), 200),
            kind: str_of(a, "kind").to_owned(),
            created_at: str_of(a, "created_at").to_owned(),
            agent: str_of(a, "agent").to_owned(),
        })
        .collect())
}

/// Denies one ask, quietly (no output): what `asks deny` does, for the terminal panel, which
/// confirms with the person first. Approving is never done here: it needs the phone's key.
pub(crate) async fn deny_quiet(g: &Global, env: &Env, id: &str) -> Result<(), Error> {
    let s = quiet_session(g, env).await?;
    let svc = service(&s, Some(id), None)?;
    svc.decide_approval(id, "deny", "denied from the terminal panel")
        .await
        .map_err(scoped)?;
    Ok(())
}

/// Sends the agent a question on one ask, quietly: what `asks question` does, for the terminal
/// panel. The ask stays open.
pub(crate) async fn question_quiet(
    g: &Global,
    env: &Env,
    id: &str,
    text: &str,
) -> Result<(), Error> {
    let note = note_text(text);
    if note.is_empty() {
        return Err(Error::Usage("a question cannot be empty".into()));
    }
    let s = quiet_session(g, env).await?;
    let svc = service(&s, Some(id), None)?;
    svc.decide_approval(id, "question", &note)
        .await
        .map_err(scoped)?;
    Ok(())
}

/// One ask opened in the terminal panel: its labelled lines (what `asks show` prints under the
/// title), plain text, and whether it is still waiting for a decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PanelAskDetail {
    pub(crate) pairs: Vec<(String, String)>,
    pub(crate) open: bool,
}

/// Reads one ask in full, for the terminal panel (the same lines as `asks show`).
pub(crate) async fn detail_quiet(g: &Global, env: &Env, id: &str) -> Result<PanelAskDetail, Error> {
    let s = quiet_session(g, env).await?;
    let svc = service(&s, Some(id), None)?;
    let a = svc.get_approval(id).await.map_err(scoped)?;
    Ok(PanelAskDetail {
        pairs: show_pairs(Paint::from_parts(false, true, None, None), &a),
        open: open(str_of(&a, "status")),
    })
}

/// The account's auto-approve policy as the panel shows it (ADR 0142): the one setting the
/// Mac app and the phone read and write, not the ask mode a shell started in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PanelAuto {
    /// Set, not expired and not revoked.
    pub(crate) active: bool,
    /// When the active policy ends (RFC 3339); empty when none is active.
    pub(crate) expires_at: String,
    /// How many asks the policy decided.
    pub(crate) approved: u64,
    /// The kinds no policy decides, from the server's rules.
    pub(crate) never_kinds: Vec<String>,
}

fn strings(v: &Value, key: &str) -> Vec<String> {
    v.get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(|s| headline(s, 40))
        .collect()
}

/// `GetAutoApprovePolicy`'s answer as the panel keeps it.
pub(crate) fn auto_of(v: &Value) -> PanelAuto {
    let policy = v.get("policy").unwrap_or(&Value::Null);
    let active = policy.get("active").and_then(Value::as_bool) == Some(true);
    PanelAuto {
        active,
        expires_at: if active {
            str_of(policy, "expires_at").to_owned()
        } else {
            String::new()
        },
        approved: policy
            .get("approved_count")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        never_kinds: strings(v.get("rules").unwrap_or(&Value::Null), "never_kinds"),
    }
}

/// Reads the account's auto-approve policy.
pub(crate) async fn auto_state(g: &Global, env: &Env) -> Result<PanelAuto, Error> {
    let s = quiet_session(g, env).await?;
    let svc = service(&s, None, None)?;
    Ok(auto_of(&svc.auto_approve_policy().await.map_err(scoped)?))
}

/// The longest window the service grants (ADR 0142), and the Mac app's default.
pub(crate) const AUTO_WINDOW_HOURS: i64 = 8;

/// Asks the phone to turn auto-approve on for [`AUTO_WINDOW_HOURS`]: the ask is put to the
/// phone, which approves it with its device key. Nothing is on until it does.
pub(crate) async fn auto_request(g: &Global, env: &Env) -> Result<String, Error> {
    let until = time::OffsetDateTime::now_utc() + time::Duration::hours(AUTO_WINDOW_HOURS);
    let until = until
        .replace_nanosecond(0)
        .map_err(|e| Error::Failed(e.to_string()))?
        .format(&time::format_description::well_known::Rfc3339)
        .map_err(|e| Error::Failed(e.to_string()))?;
    let s = quiet_session(g, env).await?;
    let svc = service(&s, None, None)?;
    let a = svc
        .request_auto_approve(&until, "iohr")
        .await
        .map_err(scoped)?;
    Ok(str_of(&a, "approval_id").to_owned())
}

/// Turns auto-approve off at once (no proof is needed to turn it off).
pub(crate) async fn auto_revoke(g: &Global, env: &Env) -> Result<(), Error> {
    let s = quiet_session(g, env).await?;
    let svc = service(&s, None, None)?;
    svc.revoke_auto_approve("turned off from the terminal panel")
        .await
        .map_err(scoped)?;
    Ok(())
}

fn service<'a>(
    s: &'a Session,
    id: Option<&str>,
    account_flag: Option<String>,
) -> Result<HttpService<'a>, Error> {
    if let Some(id) = id
        && !is_approval_id(id)
    {
        return Err(Error::Usage(
            "an ask id is apr_ and 26 capital letters and digits".into(),
        ));
    }
    let acct = account(s, account_flag)?;
    Ok(HttpService::new(&s.api, acct, None))
}

/// A failed call as the command's error: a refused sign-in says what to do.
fn scoped(e: ServiceError) -> Error {
    match e {
        ServiceError::Api(a) if matches!(a.status(), Some(401 | 403)) => {
            Error::with_hint(a, SIGN_IN_HINT)
        }
        other => other.into(),
    }
}

/// Whether to draw a live screen: a terminal on stdout, no `--json`, no `--once`.
fn live_wanted(once: bool, json: bool) -> bool {
    !once && !json && Paint::stdout(json).tty()
}

// ---- list ----

async fn list(
    s: &Session,
    status: &str,
    kind: Option<&str>,
    once: bool,
    limit: u32,
    account_flag: Option<String>,
    out: Out,
) -> Result<(), Error> {
    let status = match status {
        "all" => "",
        "open" => "open",
        other if STATUSES.contains(&other) => other,
        _ => {
            return Err(Error::Usage(format!(
                "--status is open, all, or one of {}",
                STATUSES.join(", ")
            )));
        }
    };
    let svc = service(s, None, account_flag)?;
    let kind = kind.unwrap_or_default();
    let cap = limit as usize;
    let p = Paint::stdout(out.json);
    if live_wanted(once, out.json) {
        let mut screen = Screen::new();
        let mut draw = |rows: &[Value]| {
            let mut text = list_text(p, &rows[..rows.len().min(cap)]);
            let _ = writeln!(text, "{}", p.dim("live: Ctrl-C to stop"));
            screen.draw(&text);
        };
        let run = live_list(&svc, status, kind, LIVE, &mut draw);
        let ended = tokio::select! {
            r = run => Some(r.map_err(scoped)?),
            _ = tokio::signal::ctrl_c() => None,
        };
        match ended {
            Some(Ended::NoStream(e)) => {
                Out::note(&format!(
                    "live updates are not available ({e}); printing once"
                ));
            }
            _ => return Ok(()),
        }
    }
    let mut rows = svc.list_approvals(status, kind).await.map_err(scoped)?;
    rows.truncate(cap);
    if out.json {
        Out::print_json(&json!({ "approvals": rows }));
    } else {
        Out::raw(list_text(p, &rows).as_bytes());
    }
    Ok(())
}

/// The list's screen: a table, or one line saying nothing waits.
fn list_text(p: Paint, asks: &[Value]) -> String {
    if asks.is_empty() {
        return format!("{}\n", p.dim("Nothing is waiting for you."));
    }
    let rows: Vec<Vec<String>> = asks
        .iter()
        .map(|a| {
            vec![
                str_of(a, "approval_id").to_owned(),
                status_cell(p, str_of(a, "status")),
                str_of(a, "kind").to_owned(),
                day(str_of(a, "created_at")),
                headline(by(a), 24),
                headline(str_of(a, "title"), 70),
            ]
        })
        .collect();
    let head: Vec<String> = ["ASK", "STATUS", "KIND", "ASKED", "BY", "TITLE"]
        .iter()
        .map(|h| p.head(h))
        .collect();
    let head: Vec<&str> = head.iter().map(String::as_str).collect();
    table_text(&head, &rows)
}

/// Who asked: the agent, else the person or tool behind the call.
fn by(a: &Value) -> &str {
    let agent = str_of(a, "agent");
    if agent.is_empty() {
        str_of(a, "requested_by")
    } else {
        agent
    }
}

/// A status as one cell: open ones stand out, closed ones fade, a bad ending is red.
fn status_cell(p: Paint, status: &str) -> String {
    match status {
        "pending" | "question" => p.mark(status),
        "approved" | "in_progress" | "done" => p.name(status),
        "denied" | "failed" | "stale" => p.status(status),
        other => p.dim(other),
    }
}

// ---- show ----

async fn show(
    s: &Session,
    id: &str,
    once: bool,
    account_flag: Option<String>,
    out: Out,
) -> Result<(), Error> {
    let svc = service(s, Some(id), account_flag)?;
    let p = Paint::stdout(out.json);
    if live_wanted(once, out.json) {
        let mut screen = Screen::new();
        let mut draw = |a: &Value| {
            let mut text = show_text(p, a);
            if !closed(str_of(a, "status")) {
                let _ = writeln!(text, "{}", p.dim("live: Ctrl-C to stop"));
            }
            screen.draw(&text);
            !closed(str_of(a, "status"))
        };
        let run = live_one(&svc, id, LIVE, &mut draw);
        let ended = tokio::select! {
            r = run => Some(r.map_err(scoped)?),
            _ = tokio::signal::ctrl_c() => None,
        };
        match ended {
            Some(Ended::NoStream(e)) => {
                Out::note(&format!(
                    "live updates are not available ({e}); printing once"
                ));
            }
            _ => return Ok(()),
        }
    }
    let a = svc.get_approval(id).await.map_err(scoped)?;
    if out.json {
        Out::print_json(&json!({ "approval": a }));
    } else {
        Out::raw(show_text(p, &a).as_bytes());
    }
    Ok(())
}

/// Nothing more will happen to an ask in these states.
fn closed(status: &str) -> bool {
    matches!(status, "denied" | "done" | "failed" | "stale")
}

/// Still waiting for a decision.
fn open(status: &str) -> bool {
    matches!(status, "pending" | "question")
}

fn clean(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).collect()
}

/// Up to `max` lines of other people's text, each clean and cut.
fn clean_lines(s: &str, max: usize) -> Vec<String> {
    s.lines()
        .map(|l| headline(&clean(l), 160))
        .filter(|l| !l.trim().is_empty())
        .take(max)
        .collect()
}

/// The key and value lines under an ask's title.
fn show_pairs(p: Paint, a: &Value) -> Vec<(String, String)> {
    let mut kv: Vec<(String, String)> = vec![
        ("ask".into(), str_of(a, "approval_id").to_owned()),
        ("status".into(), status_cell(p, str_of(a, "status"))),
        ("kind".into(), str_of(a, "kind").to_owned()),
    ];
    let who = headline(by(a), 60);
    if !who.is_empty() {
        let via = str_of(a, "via");
        kv.push((
            "asked by".into(),
            if via.is_empty() {
                who
            } else {
                format!("{who} via {}", headline(via, 30))
            },
        ));
    }
    for (label, key) in [
        ("asked of", "asked_of"),
        ("asked", "created_at"),
        ("github", "github"),
    ] {
        let v = headline(str_of(a, key), 80);
        if !v.is_empty() {
            kv.push((label.into(), v));
        }
    }
    payload_pairs(a, &mut kv);
    for l in a
        .get("links")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(l) = l.as_str() {
            kv.push(("link".into(), headline(l, 120)));
        }
    }
    let decided = str_of(a, "decided_by");
    if !decided.is_empty() {
        let on = str_of(a, "decided_on");
        let at = str_of(a, "decided_at");
        let mut d = headline(decided, 60);
        if !on.is_empty() {
            let _ = write!(d, " on {}", headline(on, 40));
        }
        if !at.is_empty() {
            let _ = write!(d, ", {}", day(at));
        }
        kv.push(("decided".into(), d));
    }
    let note = str_of(a, "decision_note");
    if !note.is_empty() {
        kv.push(("note".into(), headline(note, 160)));
    }
    kv
}

/// One ask in full: what it is, who asked, what the payload names, what the choices mean,
/// how it was decided and the timeline.
fn show_text(p: Paint, a: &Value) -> String {
    let mut t = String::new();
    let _ = writeln!(t, "{}", p.name(&headline(str_of(a, "title"), 120)));
    let kv = show_pairs(p, a);
    let w = kv.iter().map(|(k, _)| k.chars().count()).max().unwrap_or(0);
    for (k, v) in &kv {
        let _ = writeln!(t, "{}{}  {v}", p.dim(k), " ".repeat(w - k.chars().count()));
    }
    let summary = clean_lines(str_of(a, "summary"), 12);
    if !summary.is_empty() {
        let _ = writeln!(t, "\n{}", p.head("Summary"));
        for l in summary {
            let _ = writeln!(t, "  {l}");
        }
    }
    let means: Vec<(&str, String)> = [
        ("Approve means", str_of(a, "approve_means")),
        ("Deny means", str_of(a, "deny_means")),
    ]
    .into_iter()
    .filter(|(_, v)| !v.is_empty())
    .map(|(k, v)| (k, headline(v, 160)))
    .collect();
    if !means.is_empty() {
        let _ = writeln!(t);
        for (k, v) in means {
            let _ = writeln!(t, "{}  {v}", p.dim(k));
        }
    }
    let timeline = a
        .get("timeline")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    if !timeline.is_empty() {
        let _ = writeln!(t, "\n{}", p.head("Timeline"));
        for e in timeline {
            let by = headline(str_of(e, "by"), 40);
            let mut line = format!("  {}  {}", day(str_of(e, "at")), p.name(str_of(e, "step")));
            if !by.is_empty() {
                let _ = write!(line, "  {by}");
            }
            let note = headline(str_of(e, "note"), 100);
            if !note.is_empty() {
                let _ = write!(line, "  {}", p.dim(&note));
            }
            let _ = writeln!(t, "{line}");
        }
    }
    t
}

/// What the payload names (`payload.<kind>`): its text and number fields, as they come.
fn payload_pairs(a: &Value, kv: &mut Vec<(String, String)>) {
    let Some(body) = a
        .get("payload")
        .and_then(Value::as_object)
        .and_then(|o| o.values().next())
        .and_then(Value::as_object)
    else {
        return;
    };
    for (key, v) in body {
        let shown = match v {
            Value::String(s) => headline(s, 120),
            Value::Number(n) => n.to_string(),
            Value::Array(items) => items
                .iter()
                .filter_map(Value::as_str)
                .map(|s| headline(s, 40))
                .collect::<Vec<_>>()
                .join(", "),
            _ => continue,
        };
        if !shown.is_empty() {
            kv.push((clean(key), shown));
        }
    }
}

// ---- deciding ----

async fn deny(
    s: &Session,
    id: &str,
    note: &str,
    account_flag: Option<String>,
    out: Out,
) -> Result<(), Error> {
    let svc = service(s, Some(id), account_flag)?;
    let a = svc
        .decide_approval(id, "deny", note)
        .await
        .map_err(scoped)?;
    done(out, &a, "Denied");
    Ok(())
}

async fn question(
    s: &Session,
    id: &str,
    note: &str,
    account_flag: Option<String>,
    out: Out,
) -> Result<(), Error> {
    if note.is_empty() {
        return Err(Error::Usage("a question needs its text: --note".into()));
    }
    let svc = service(s, Some(id), account_flag)?;
    let a = svc
        .decide_approval(id, "question", note)
        .await
        .map_err(scoped)?;
    done(out, &a, "Question sent; the ask stays open");
    Ok(())
}

/// The question's text: asked for on a terminal, else a usage error.
fn ask_question_text(out: Out) -> Result<String, Error> {
    if !prompt::interactive() || out.json {
        return Err(Error::Usage("a question needs its text: --note".into()));
    }
    let q = Ui::detect(out.json).input("Your question for the agent", "", |v| {
        if note_text(v).is_empty() {
            Err("a question cannot be empty".into())
        } else {
            Ok(())
        }
    })?;
    Ok(note_text(&q))
}

/// What a decision printed: the new ask as JSON, or one line.
fn done(out: Out, a: &Value, said: &str) {
    if out.json {
        Out::print_json(&json!({ "approval": a }));
    } else {
        let p = Paint::stdout(false);
        Out::raw(
            format!(
                "{} {}  {}\n",
                p.name(said),
                str_of(a, "approval_id"),
                p.dim(str_of(a, "status"))
            )
            .as_bytes(),
        );
    }
}

/// The menu: approve (here when this Mac may sign it, else on the phone), deny or question,
/// for one open ask.
async fn decide(
    g: &Global,
    s: &Session,
    id: &str,
    account_flag: Option<String>,
    out: Out,
) -> Result<(), Error> {
    if !prompt::interactive() || out.json {
        return Err(Error::Usage(
            "`asks decide` is a menu and needs a terminal; use `asks deny`, `asks question` or `asks approve`".into(),
        ));
    }
    let svc = service(s, Some(id), account_flag.clone())?;
    let a = svc.get_approval(id).await.map_err(scoped)?;
    let status = str_of(&a, "status").to_owned();
    Out::raw(show_text(Paint::stdout(false), &a).as_bytes());
    if !open(&status) {
        return Err(Error::Failed(format!(
            "this ask is {status}; there is nothing left to decide"
        )));
    }
    let choices = [
        choice(
            "approve",
            "Approve",
            "with Touch ID on a Mac that has a key, else on your phone",
            'a',
        ),
        choice("deny", "Deny", "with a note if you like", 'd'),
        choice("question", "Ask a question", "the ask stays open", 'w'),
    ];
    match Ui::select_keyed("What do you want to do?", &choices, "approve")?.as_deref() {
        Some("approve") => approve(g, s, id, false, account_flag, out).await,
        Some("deny") => {
            let note = Ui::detect(false).input("Why? (optional, Enter to skip)", "", |_| Ok(()))?;
            deny(s, id, &note_text(&note), account_flag, out).await
        }
        Some("question") => {
            let q = ask_question_text(out)?;
            question(s, id, &q, account_flag, out).await
        }
        _ => {
            Out::note("Nothing changed.");
            Ok(())
        }
    }
}

fn choice(value: &str, label: &str, hint: &str, key: char) -> Choice {
    Choice {
        value: value.to_owned(),
        label: label.to_owned(),
        hint: hint.to_owned(),
        available: true,
        key: Some(key),
    }
}

async fn approve(
    g: &Global,
    s: &Session,
    id: &str,
    once: bool,
    account_flag: Option<String>,
    out: Out,
) -> Result<(), Error> {
    let svc = service(s, Some(id), account_flag)?;
    let a = svc.get_approval(id).await.map_err(scoped)?;
    let status = str_of(&a, "status");
    if !open(status) {
        done(out, &a, &format!("Already {status}:"));
        return Ok(());
    }
    // A Mac with an enrolled key signs the asks its tier allows; the rest is the phone's.
    let why = match key::attempt(g, &svc, &a).await? {
        key::Attempt::Signed(approved) => {
            done(out, &approved, "Approved");
            return Ok(());
        }
        key::Attempt::Phone(why) => why,
    };
    Out::note("Approve on your phone");
    if let Some(why) = why {
        Out::note(&format!(
            "{why}: it needs your phone's key. Open InOrbit there; this ask is under Needs you."
        ));
    } else {
        Out::note(
            "Approving needs your phone's key. Open InOrbit there; this ask is under Needs you.",
        );
        if mac_sign::can_sign_here() {
            Out::note("To approve the smaller asks from this Mac, run `iohr asks key enroll`.");
        }
    }
    let means = headline(str_of(&a, "approve_means"), 160);
    if !means.is_empty() {
        Out::note(&format!("Approving means: {means}"));
    }
    if once {
        return Ok(());
    }
    Out::note("Waiting for you; Ctrl-C to stop.");
    let mut last = a;
    let mut draw = |a: &Value| {
        last = a.clone();
        open(str_of(a, "status"))
    };
    let run = live_one(&svc, id, LIVE, &mut draw);
    let ended = tokio::select! {
        r = run => Some(r.map_err(scoped)?),
        _ = tokio::signal::ctrl_c() => None,
    };
    match ended {
        None => {
            Out::note("Stopped waiting; the ask is unchanged.");
            Ok(())
        }
        Some(Ended::NoStream(e)) => Err(Error::Failed(format!(
            "cannot watch this ask for your approval ({e}); check it with `iohr asks show {id}`"
        ))),
        Some(Ended::Done) => {
            let said = match str_of(&last, "status") {
                "approved" | "in_progress" | "done" => "Approved",
                "denied" => "Denied",
                _ => "Settled:",
            };
            done(out, &last, said);
            Ok(())
        }
    }
}

// ---- the live loops ----

/// The list, drawn now and again whenever an `approval.changed` event says something moved.
/// The stream is opened before the list is read, so a change between the two is not lost; a
/// stream that ends or fails is opened again after `pace.backoff`, and the list read again.
pub(crate) async fn live_list(
    svc: &HttpService<'_>,
    status: &str,
    kind: &str,
    pace: Pace,
    draw: &mut dyn FnMut(&[Value]),
) -> Result<Ended, ServiceError> {
    let mut round = 0_usize;
    loop {
        let mut stream = match svc.approval_changes().await {
            Ok(s) => Some(s),
            Err(e) if e.is_permanent() => return Ok(Ended::NoStream(e)),
            Err(_) => None,
        };
        match svc.list_approvals(status, kind).await {
            Ok(rows) => draw(&rows),
            Err(e) if e.is_permanent() || round == 0 => return Err(e),
            Err(_) => {}
        }
        if let Some(st) = stream.as_mut() {
            while let Some(frame) = st.next().await {
                if frame.is_err() {
                    break;
                }
                match svc.list_approvals(status, kind).await {
                    Ok(rows) => draw(&rows),
                    Err(e) if e.is_permanent() => return Err(e),
                    Err(_) => break,
                }
            }
        }
        round += 1;
        if pace.rounds.is_some_and(|m| round >= m) {
            return Ok(Ended::Done);
        }
        tokio::time::sleep(pace.backoff).await;
    }
}

/// One ask, drawn now and on each `Watch` frame, until `draw` says to stop (it returns
/// whether to go on).
pub(crate) async fn live_one(
    svc: &HttpService<'_>,
    id: &str,
    pace: Pace,
    draw: &mut dyn FnMut(&Value) -> bool,
) -> Result<Ended, ServiceError> {
    let mut round = 0_usize;
    loop {
        let mut stream = match svc.watch_approval(id).await {
            Ok(s) => Some(s),
            Err(e) if e.is_permanent() => return Ok(Ended::NoStream(e)),
            Err(_) => None,
        };
        match svc.get_approval(id).await {
            Ok(a) => {
                if !draw(&a) {
                    return Ok(Ended::Done);
                }
            }
            Err(e) if e.is_permanent() || round == 0 => return Err(e),
            Err(_) => {}
        }
        if let Some(st) = stream.as_mut() {
            while let Some(frame) = st.next().await {
                let Ok(frame) = frame else { break };
                if let Some(a) = frame.get("approval")
                    && !draw(a)
                {
                    return Ok(Ended::Done);
                }
            }
        }
        round += 1;
        if pace.rounds.is_some_and(|m| round >= m) {
            return Ok(Ended::Done);
        }
        tokio::time::sleep(pace.backoff).await;
    }
}

/// A screen redrawn in place: the last frame's lines are cleared, the new one cut to the
/// terminal's width so no line wraps and the count of lines stays true.
struct Screen {
    drawn: usize,
}

impl Screen {
    fn new() -> Self {
        Self { drawn: 0 }
    }

    fn draw(&mut self, text: &str) {
        let term = Term::stdout();
        let width = usize::from(term.size().1).max(40);
        let fitted: String = text
            .lines()
            .map(|l| {
                if measure_text_width(l) > width {
                    truncate_str(l, width - 1, "...").into_owned()
                } else {
                    l.to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        if self.drawn > 0 {
            let _ = term.clear_last_lines(self.drawn);
        }
        Out::raw(format!("{fitted}\n").as_bytes());
        self.drawn = fitted.lines().count();
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "tests fail by panicking")]
mod tests {
    use std::sync::Arc;

    use iohr_auth::{Redacted, StaticToken};
    use wiremock::matchers::{body_partial_json, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::context::{AnyCredential, Api};

    fn ask(id: &str, status: &str, title: &str) -> Value {
        json!({
            "approval_id": id, "kind": "pull_request", "title": title, "status": status,
            "agent": "builder", "created_at": "2026-11-05T09:00:00Z",
            "summary": "Fixes the thing.\nSecond line.",
            "payload": {"pull_request": {"repo": "inorbithr/core", "number": 12, "head": "abc123"}},
            "links": ["https://github.com/inorbithr/core/pull/12"],
            "approve_means": "Merge the pull request.",
            "timeline": [{"step": "asked", "by": "builder", "at": "2026-11-05T09:00:00Z"}]
        })
    }

    fn cache_file(content: Option<&str>) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("asks-count");
        if let Some(c) = content {
            std::fs::write(&path, c).unwrap();
        }
        (dir, path)
    }

    fn would_ask() -> Error {
        Error::StoreWouldAsk("the credential store would ask".into())
    }

    #[test]
    fn a_store_that_would_ask_keeps_the_cached_count_and_prints_nothing() {
        let (_d, path) = cache_file(Some("3\n"));
        let old = std::time::SystemTime::now() - Duration::from_secs(120);
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(old)
            .unwrap();
        assert!(!count_cache_fresh(&path));
        let shown = settle(Some(&path), Err(would_ask())).unwrap();
        assert_eq!(shown, None, "nothing is printed");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "3\n");
        assert!(count_cache_fresh(&path), "the age limits the next try");
    }

    #[test]
    fn a_store_that_would_ask_with_no_cache_yet_leaves_zero() {
        let (_d, path) = cache_file(None);
        assert_eq!(settle(Some(&path), Err(would_ask())).unwrap(), None);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "0\n");
    }

    #[test]
    fn without_cache_a_store_that_would_ask_is_the_error() {
        assert!(matches!(
            settle(None, Err(would_ask())),
            Err(Error::StoreWouldAsk(_))
        ));
    }

    #[test]
    fn another_failed_refresh_still_clears_the_count() {
        let (_d, path) = cache_file(Some("3\n"));
        let r = settle(Some(&path), Err(Error::Failed("offline".into())));
        assert!(r.is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "0\n");
    }

    #[test]
    fn a_fetched_count_is_cached_and_shown() {
        let (_d, path) = cache_file(Some("3\n"));
        assert_eq!(settle(Some(&path), Ok(5)).unwrap(), Some(5));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "5\n");
    }

    fn waiting_of(rows: &[(&str, &str)]) -> Waiting {
        let rows: Vec<Value> = rows.iter().map(|(st, t)| ask(ID1, st, t)).collect();
        Waiting::of(&rows)
    }

    #[test]
    fn a_pending_ask_is_counted_and_named() {
        let w = waiting_of(&[("pending", "Merge #12")]);
        assert_eq!(w.count, 1);
        assert_eq!(w.only.as_deref(), Some("Merge #12"));
    }

    #[test]
    fn a_question_is_the_agents_turn_and_is_not_counted() {
        let w = waiting_of(&[("question", "Roll accounts")]);
        assert_eq!(
            w,
            Waiting {
                count: 0,
                only: None
            }
        );
    }

    #[test]
    fn a_mixed_list_counts_only_the_pending_and_names_the_one() {
        let w = waiting_of(&[("question", "Sent back"), ("pending", "Merge #12")]);
        assert_eq!((w.count, w.only.as_deref()), (1, Some("Merge #12")));
        let w = waiting_of(&[
            ("pending", "A"),
            ("question", "B"),
            ("pending", "C"),
            ("question", "D"),
        ]);
        assert_eq!((w.count, w.only), (2, None), "several keep the count");
    }

    #[test]
    fn a_title_is_one_clean_line() {
        let w = waiting_of(&[("pending", "Merge\u{1b}[31m #12\nsecond line")]);
        assert_eq!(w.only.as_deref(), Some("Merge[31m #12"));
    }

    #[test]
    fn the_cache_holds_the_pending_count_only() {
        let (_d, path) = cache_file(None);
        let w = waiting_of(&[
            ("question", "B"),
            ("pending", "Merge #12"),
            ("pending", "C"),
        ]);
        assert_eq!(settle(Some(&path), Ok(w.count)).unwrap(), Some(2));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "2\n");
    }

    fn api(server: &MockServer) -> Api {
        let cred = Arc::new(AnyCredential::Token(StaticToken::new(Redacted::new(
            "person-token".to_owned(),
        ))));
        Api::new(&server.uri(), cred, false).unwrap()
    }

    const ID1: &str = "apr_01M6NZ452CGYK04BSGM9BNJD4Q";
    const ID2: &str = "apr_01M6NZ45CZY218FHED8G63N458";
    const ID9: &str = "apr_01M6NZ9Z0AZ5YY02R0F40YNNJ2";

    const FAST: Pace = Pace {
        backoff: Duration::from_millis(5),
        rounds: Some(2),
    };

    #[test]
    fn the_list_is_a_table_with_the_ask_in_it() {
        let t = list_text(
            Paint::plain(),
            &[
                ask(ID1, "pending", "Merge #12"),
                ask(ID2, "question", "Roll"),
            ],
        );
        let lines: Vec<&str> = t.lines().collect();
        assert_eq!(lines.len(), 3, "{t}");
        assert!(
            lines[0].starts_with("ASK") && lines[0].contains("STATUS"),
            "{t}"
        );
        assert!(
            lines[1].contains(ID1) && lines[1].contains("pending"),
            "{t}"
        );
        assert!(
            lines[1].contains("2026-11-05") && lines[1].contains("Merge #12"),
            "{t}"
        );
        assert!(!t.contains('\u{1b}'), "no colour in a pipe");
    }

    #[test]
    fn an_empty_list_says_so() {
        assert_eq!(
            list_text(Paint::plain(), &[]),
            "Nothing is waiting for you.\n"
        );
    }

    #[test]
    fn an_asks_text_cannot_move_the_cursor() {
        let mut a = ask(ID1, "pending", "Merge\u{1b}[2J #12");
        a["summary"] = json!("line\u{1b}]0;x\u{7}\nnext");
        let t = format!(
            "{}{}",
            list_text(Paint::plain(), &[a.clone()]),
            show_text(Paint::plain(), &a)
        );
        assert!(!t.contains('\u{1b}') && !t.contains('\u{7}'), "{t:?}");
        assert!(t.contains("Merge[2J #12"));
    }

    #[test]
    fn show_has_the_payload_the_timeline_and_what_approving_means() {
        let t = show_text(Paint::plain(), &ask(ID1, "pending", "Merge #12"));
        for want in [
            "Merge #12",
            ID1,
            "pending",
            "builder",
            "inorbithr/core",
            "12",
            "abc123",
            "https://github.com/inorbithr/core/pull/12",
            "Summary",
            "Fixes the thing.",
            "Approve means",
            "Merge the pull request.",
            "Timeline",
            "asked",
        ] {
            assert!(t.contains(want), "missing {want:?} in:\n{t}");
        }
    }

    #[test]
    fn a_decided_ask_shows_who_and_the_note() {
        let mut a = ask(ID1, "denied", "Merge #12");
        a["decided_by"] = json!("nevio");
        a["decided_on"] = json!("terminal");
        a["decided_at"] = json!("2026-11-06T10:00:00Z");
        a["decision_note"] = json!("not yet");
        let t = show_text(Paint::plain(), &a);
        assert!(
            t.contains("nevio on terminal, 2026-11-06") && t.contains("not yet"),
            "{t}"
        );
    }

    #[test]
    fn states_are_open_or_closed() {
        assert!(open("pending") && open("question") && !open("approved"));
        assert!(closed("done") && closed("denied") && !closed("approved") && !closed("pending"));
    }

    #[tokio::test]
    async fn the_live_list_draws_again_when_an_event_arrives() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/events/events"))
            .and(query_param("types", "approval.changed"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(
                        "data: {\"type\":\"approval.changed\",\"data\":{\"approval_id\":\"apr_1\"}}\n\n",
                    ),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v1/approvals"))
            .and(query_param("status", "open"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "approvals": [ask(ID1, "pending", "Merge #12")]
            })))
            .mount(&server)
            .await;
        let api = api(&server);
        let svc = HttpService::new(&api, "acc_1".into(), None);
        let mut frames = 0_usize;
        let mut draw = |rows: &[Value]| {
            assert_eq!(rows.len(), 1);
            frames += 1;
        };
        let ended = live_list(&svc, "open", "", FAST, &mut draw).await.unwrap();
        assert!(matches!(ended, Ended::Done));
        // Per round: the first list, then one more for the event; two rounds.
        assert_eq!(frames, 4);
    }

    #[tokio::test]
    async fn a_refused_stream_falls_back_instead_of_failing() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/events/events"))
            .respond_with(ResponseTemplate::new(403).set_body_json(json!({"message": "no scope"})))
            .mount(&server)
            .await;
        let api = api(&server);
        let svc = HttpService::new(&api, "acc_1".into(), None);
        let mut draw = |_: &[Value]| panic!("nothing is drawn before the fallback");
        let ended = live_list(&svc, "open", "", FAST, &mut draw).await.unwrap();
        assert!(matches!(ended, Ended::NoStream(_)));
    }

    #[tokio::test]
    async fn the_live_ask_follows_watch_frames_until_asked_to_stop() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(format!("/v1/approvals/{ID1}/events")))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(format!(
                        "data: {}\n\n",
                        json!({"approval": ask(ID1, "approved", "Merge #12")})
                    )),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!("/v1/approvals/{ID1}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "approval": ask(ID1, "pending", "Merge #12")
            })))
            .mount(&server)
            .await;
        let api = api(&server);
        let svc = HttpService::new(&api, "acc_1".into(), None);
        let mut seen = Vec::new();
        let mut draw = |a: &Value| {
            seen.push(str_of(a, "status").to_owned());
            open(str_of(a, "status"))
        };
        let ended = live_one(&svc, ID1, FAST, &mut draw).await.unwrap();
        assert!(matches!(ended, Ended::Done), "{ended:?}");
        assert_eq!(seen, ["pending", "approved"]);
    }

    #[tokio::test]
    async fn a_missing_ask_is_an_error_not_a_loop() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(format!("/v1/approvals/{ID9}/events")))
            .respond_with(
                ResponseTemplate::new(200).insert_header("content-type", "text/event-stream"),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!("/v1/approvals/{ID9}")))
            .respond_with(ResponseTemplate::new(404).set_body_json(json!({"message": "not found"})))
            .mount(&server)
            .await;
        let api = api(&server);
        let svc = HttpService::new(&api, "acc_1".into(), None);
        let mut draw = |_: &Value| true;
        assert!(live_one(&svc, ID9, FAST, &mut draw).await.is_err());
    }
    #[tokio::test]
    async fn a_signed_approval_carries_its_proof_and_a_plain_one_never_does() {
        let server = MockServer::start().await;
        let proof = json!({
            "key_id": "apk_1", "signed_at": 9, "nonce": "n", "signature": "sig"
        });
        Mock::given(method("POST"))
            .and(path(format!("/v1/approvals/{ID1}/decision")))
            .and(body_partial_json(json!({
                "account_id": "acc_1", "decision": "approve", "proof": proof
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "approval": ask(ID1, "approved", "Review the plan")
            })))
            .expect(1)
            .mount(&server)
            .await;
        let signed_api = api(&server);
        let svc = HttpService::new(&signed_api, "acc_1".into(), None);
        let got = svc.decide_signed(ID1, "approve", "", &proof).await.unwrap();
        assert_eq!(got["status"], "approved");
        // A denial sends no proof field at all.
        let plain = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(format!("/v1/approvals/{ID1}/decision")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "approval": ask(ID1, "denied", "x")
            })))
            .mount(&plain)
            .await;
        let plain_api = api(&plain);
        let svc = HttpService::new(&plain_api, "acc_1".into(), None);
        svc.decide_approval(ID1, "deny", "no").await.unwrap();
        let sent = plain.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&sent[0].body).unwrap();
        assert!(body.get("proof").is_none(), "{body}");
    }

    #[tokio::test]
    async fn enrolling_posts_the_body_and_revoking_refuses_a_foreign_id() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/approvals/keys"))
            .and(body_partial_json(
                json!({"platform": "macos", "public_key": "pk"}),
            ))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"key": {"key_id": "apk_9"}})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let api = api(&server);
        let svc = HttpService::new(&api, "acc_1".into(), None);
        let key = svc
            .enroll_key(&json!({"platform": "macos", "public_key": "pk"}))
            .await
            .unwrap();
        assert_eq!(key["key_id"], "apk_9");
        assert!(svc.revoke_key("../x").await.is_err());
        assert!(svc.revoke_key("key_1").await.is_err());
    }
}
