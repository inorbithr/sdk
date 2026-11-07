//! Workloads in the panel: what is running for the person, as the Mac app's Workload page reads
//! it (PRD 0055), flattened into a list.
//!
//! Two real sources, no third. The sessions are the iohr-agents registry
//! (`~/.inorbit/agents/sessions/*.json`, the files `iohr-agents list` and the Mac app read):
//! a session is listed while its process runs, and it is `working` when it was seen in the last
//! ten minutes, else `idle` (the Mac app's `ACTIVE_MIN`). The queue is the coordinator's picture
//! (`GET /v1/coordinator/picture`, the same snapshot the Mac app folds). A session `needs you`
//! when an open ask names it as its agent (the Mac app's rule); the join is done by the model,
//! which holds the asks.
//!
//! Not read here, and shown as `not available` rather than guessed: a session's pull request,
//! and the workers (sub-agents) a session started. A session's model and latest line come
//! from the tail of its transcript; its memory and CPU from `ps` for its own process.
//!
//! Reading is blocking and pure of the network; the proxy runs it off the loop. Every piece of
//! text read here is sanitised by [`super::cells::one_wide`] when it is drawn.

use std::collections::HashMap;
use std::io::{Read as _, Seek as _, SeekFrom};
use std::path::{Path, PathBuf};

use serde_json::Value;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// A session seen within this many minutes is working; the Mac app's `ACTIVE_MIN`.
const ACTIVE_MIN: u64 = 10;
/// How much of a transcript's end is read for its model and latest line.
const TAIL: u64 = 128 * 1024;
/// Sessions listed at most.
const SESSIONS: usize = 30;
/// Queue jobs listed at most (the count says how many there are).
const JOBS: usize = 6;

/// Where a workload stands: the dot's colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum State {
    Working,
    Needs,
    Idle,
    Stuck,
}

impl State {
    /// How much the roll-up of a group counts this state (the Mac app's rank).
    fn rank(self) -> u8 {
        match self {
            State::Needs => 4,
            State::Stuck => 3,
            State::Working => 2,
            State::Idle => 1,
        }
    }

    /// Running now: everything but idle.
    pub(super) fn running(self) -> bool {
        self != State::Idle
    }

    pub(super) fn word(self) -> &'static str {
        match self {
            State::Working => "working",
            State::Needs => "needs you",
            State::Idle => "idle",
            State::Stuck => "stuck",
        }
    }
}

/// What a row is: the coordinator on this Mac, one of its sessions, the coordinator service's
/// queue, or one job in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Role {
    Root,
    Session,
    Service,
    Job,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Workload {
    pub(super) id: String,
    pub(super) role: Role,
    pub(super) state: State,
    pub(super) label: String,
    /// Its model, or where it runs.
    pub(super) sub: String,
    /// Minutes since it was last seen (sessions), or since it last changed (jobs).
    pub(super) idle_min: Option<u64>,
    /// The right-hand cell when it is not the state's own words.
    pub(super) right: Option<String>,
    pub(super) why: Option<String>,
    pub(super) latest: Option<String>,
    pub(super) uses: Option<String>,
    pub(super) waits: Option<String>,
    pub(super) pr: Option<String>,
    /// The open ask it waits on (set by the model's join, or by a job's own approval).
    pub(super) ask: Option<String>,
    /// The names an ask may call it by (lower case): its name, then its id.
    pub(super) agent_keys: Vec<String>,
    pub(super) parent: String,
}

impl Workload {
    fn blank(id: &str, role: Role, label: &str, parent: &str) -> Self {
        Self {
            id: id.to_owned(),
            role,
            state: State::Idle,
            label: label.to_owned(),
            sub: String::new(),
            idle_min: None,
            right: None,
            why: None,
            latest: None,
            uses: None,
            waits: None,
            pr: None,
            ask: None,
            agent_keys: Vec::new(),
            parent: parent.to_owned(),
        }
    }

    /// The time cell: "needs you", "idle 18 min", "4 min ago", or what the source said.
    pub(super) fn right(&self) -> String {
        if let Some(r) = &self.right {
            return r.clone();
        }
        match (self.state, self.idle_min) {
            (State::Needs | State::Stuck, _) => self.state.word().to_owned(),
            (State::Idle, Some(m)) => format!("idle {}", span(m)),
            (State::Working, Some(m)) if m >= 1 => format!("{} ago", span(m)),
            (State::Working, _) => "active".to_owned(),
            (State::Idle, None) => "idle".to_owned(),
        }
    }

    /// The `Since` line: the time cell, what it runs on, and where it sits.
    pub(super) fn since(&self) -> String {
        let mut parts = vec![self.right()];
        if !self.sub.is_empty() {
            parts.push(self.sub.clone());
        }
        if !self.parent.is_empty() {
            parts.push(format!("under {}", self.parent));
        }
        parts.join("  ·  ")
    }
}

/// "just now", "14 min", "3 h".
fn span(m: u64) -> String {
    match m {
        0 => "just now".to_owned(),
        1..=119 => format!("{m} min"),
        _ => format!("{} h", m / 60),
    }
}

/// What one read of the sources found. The queue says why it is missing when it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Snapshot {
    pub(super) sessions: Vec<Workload>,
    pub(super) queue: Result<Queue, String>,
}

/// The coordinator service's open jobs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Queue {
    pub(super) open: usize,
    pub(super) jobs: Vec<Workload>,
    /// The server cut the picture for a reader who may not see everything.
    pub(super) redacted: bool,
}

/// The rows of a snapshot as the list shows them: the coordinator and its sessions, then the
/// service and its jobs. `asks` are the open asks as (id, title, agent), joined to sessions.
pub(super) fn tree(snap: &Snapshot, asks: &[(String, String, String)]) -> Vec<Workload> {
    let mut sessions = snap.sessions.clone();
    for s in &mut sessions {
        if let Some((id, title, _)) = asks
            .iter()
            .find(|(_, _, agent)| names(&s.agent_keys, agent))
            && s.state != State::Stuck
        {
            s.state = State::Needs;
            s.ask = Some(id.clone());
            s.waits = Some(format!("{title}  ·  approve on your phone"));
        }
    }
    let mut root = Workload::blank("coordinator", Role::Root, "Coordinator", "");
    "this Mac".clone_into(&mut root.sub);
    root.state = sessions
        .iter()
        .map(|s| s.state)
        .max_by_key(|s| s.rank())
        .unwrap_or(State::Idle);
    root.why = Some("Everything open on this Mac, from the iohr-agents registry.".to_owned());
    root.latest = Some(match sessions.len() {
        0 => "No session is running.".to_owned(),
        1 => "1 session is running.".to_owned(),
        n => format!("{n} sessions are running."),
    });
    root.right = match root.state {
        State::Idle => None,
        s => Some(s.word().to_owned()),
    };
    let mut out = vec![root];
    let n = sessions.len();
    for (i, mut s) in sessions.into_iter().enumerate() {
        "Coordinator".clone_into(&mut s.parent);
        let _ = i + 1 == n;
        out.push(s);
    }

    let mut service = Workload::blank("service", Role::Service, "Coordinator service", "");
    "queue".clone_into(&mut service.sub);
    service.why = Some("The coordinator's job queue on the server (RFC 0120).".to_owned());
    match &snap.queue {
        Ok(q) => {
            service.state = if q.jobs.iter().any(|j| j.state == State::Working) {
                State::Working
            } else {
                State::Idle
            };
            service.right = Some(match q.open {
                1 => "1 job".to_owned(),
                n => format!("{n} jobs"),
            });
            service.latest = Some(if q.redacted {
                "The server shows you the public picture: job titles are hidden.".to_owned()
            } else {
                format!("{} open", q.open)
            });
            out.push(service);
            out.extend(q.jobs.iter().cloned());
        }
        Err(why) => {
            service.state = State::Idle;
            service.right = Some("unavailable".to_owned());
            service.latest = Some(format!("The queue could not be read: {why}"));
            out.push(service);
        }
    }
    out
}

/// Whether an ask's agent name is one of a workload's names: its name or id exactly, or an id
/// prefix of at least six characters (the Mac app's rule).
fn names(keys: &[String], agent: &str) -> bool {
    let a = agent.trim().to_lowercase();
    !a.is_empty()
        && keys
            .iter()
            .any(|k| *k == a || (a.len() >= 6 && k.len() > a.len() && k.starts_with(&a)))
}

/// The tree glyphs (`├ `, `└ `) for the rows of a list: a session or job row is the last of its
/// group when the next row is not of the same role.
pub(super) fn glyph(rows: &[Workload], i: usize) -> &'static str {
    let row = &rows[i];
    match row.role {
        Role::Root | Role::Service => "",
        role => {
            if rows.get(i + 1).is_some_and(|n| n.role == role) {
                "├ "
            } else {
                "└ "
            }
        }
    }
}

/// The sessions registry's folder: `~/.inorbit`.
pub(super) fn home_root() -> Option<PathBuf> {
    etcetera::home_dir().ok().map(|h| h.join(".inorbit"))
}

/// The demo's workloads (`--demo`, labelled stub in the title): the design's tree, not read
/// from anything. The session named "Fix sdk #448 reads" is the one the stub ask names.
pub(super) fn stub() -> Snapshot {
    let session = |id: &str, label: &str, model: &str, state, right: &str| {
        let mut w = Workload::blank(&format!("s:stub-{id}"), Role::Session, label, "Coordinator");
        model.clone_into(&mut w.sub);
        w.state = state;
        w.right = Some(right.to_owned());
        w.agent_keys = vec![label.to_lowercase()];
        w
    };
    let mut builder = session(
        "1",
        "Floating band builder",
        "Sonnet",
        State::Working,
        "14 min",
    );
    builder.why = Some("draw the floating band over the shell".to_owned());
    builder.latest = Some("rendering the band over the cursor row".to_owned());
    builder.uses = Some("388 MB  \u{b7}  21% CPU".to_owned());
    let mut roles = session(
        "2",
        "Roles session-mode",
        "Sonnet",
        State::Working,
        "22 min",
    );
    roles.why = Some("start a session as a role".to_owned());
    roles.latest = Some("writing the session-mode tests".to_owned());
    let mut fix = session(
        "3",
        "Fix sdk #448 reads",
        "Sonnet",
        State::Working,
        "needs you",
    );
    fix.why = Some("clear the security hold on sdk #448 (more read folders)".to_owned());
    fix.latest = Some("rewriting grant_access in ask.rs".to_owned());
    fix.uses = Some("412 MB  \u{b7}  38% CPU".to_owned());
    fix.pr = Some("inorbithr/sdk #448  draft  \u{b7}  3 checks failing".to_owned());
    let mut atlas = session("4", "Atlas sync", "job", State::Idle, "idle 18 min");
    atlas.latest = Some("synced 214 pages".to_owned());
    let jobs = [
        "Roll runner to 0.7.21",
        "Apply migration 20261106",
        "Index the docs",
    ]
    .iter()
    .enumerate()
    .map(|(i, t)| {
        let mut j = Workload::blank(&format!("j:stub-{i}"), Role::Job, t, "Coordinator service");
        "job".clone_into(&mut j.sub);
        j.state = if i == 0 { State::Working } else { State::Idle };
        j.right = Some(if i == 0 { "running" } else { "approved" }.to_owned());
        j.why = Some((*t).to_owned());
        j
    })
    .collect();
    Snapshot {
        sessions: vec![builder, roles, fix, atlas],
        queue: Ok(Queue {
            open: 3,
            jobs,
            redacted: false,
        }),
    }
}

// ------------------------------------------------------------------ sessions

fn text<'a>(o: &'a Value, keys: &[&str]) -> &'a str {
    keys.iter()
        .find_map(|k| o.get(*k).and_then(Value::as_str))
        .unwrap_or("")
}

fn minutes_since(at: &str, now: OffsetDateTime) -> Option<u64> {
    let at = OffsetDateTime::parse(at, &Rfc3339).ok()?;
    u64::try_from((now - at).whole_minutes().max(0)).ok()
}

/// The live sessions of the registry under `root` (`~/.inorbit`). `alive` says whether a pid
/// runs and `uses` gives each pid's memory and CPU, so tests answer without processes.
pub(super) fn sessions(
    root: &Path,
    now: OffsetDateTime,
    alive: &dyn Fn(u32) -> bool,
    uses: &dyn Fn(&[u32]) -> HashMap<u32, String>,
) -> Vec<Workload> {
    let mut found: Vec<(Workload, u32)> = Vec::new();
    for entry in std::fs::read_dir(root.join("agents/sessions"))
        .into_iter()
        .flatten()
        .flatten()
    {
        let path = entry.path();
        if path.extension().and_then(|x| x.to_str()) != Some("json") {
            continue;
        }
        let Some(o) = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        else {
            continue;
        };
        let Some(pid) = o
            .get("pid")
            .and_then(Value::as_u64)
            .and_then(|p| u32::try_from(p).ok())
            .filter(|p| alive(*p))
        else {
            continue;
        };
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let id = Some(text(
            &o,
            &["claude_session_id", "session_id", "sessionId", "id"],
        ))
        .filter(|s| !s.is_empty())
        .map_or(stem, str::to_owned);
        found.push((one_session(&o, &id, now), pid));
    }
    found.sort_by(|a, b| {
        a.0.idle_min
            .cmp(&b.0.idle_min)
            .then_with(|| a.0.id.cmp(&b.0.id))
    });
    found.truncate(SESSIONS);
    let pids: Vec<u32> = found.iter().map(|f| f.1).collect();
    let by_pid = uses(&pids);
    found
        .into_iter()
        .map(|(mut w, pid)| {
            w.uses = by_pid.get(&pid).cloned();
            w
        })
        .collect()
}

fn one_session(o: &Value, id: &str, now: OffsetDateTime) -> Workload {
    let name = text(o, &["name", "agent", "title"]).trim().to_owned();
    let cwd = text(o, &["cwd"]);
    let folder = Path::new(cwd)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let branch = text(o, &["branch"]);
    let short: String = id.chars().take(8).collect();
    let label = [name.as_str(), branch, folder.as_str()]
        .into_iter()
        .find(|s| !s.is_empty())
        .map_or_else(|| format!("Session {short}"), str::to_owned);
    let (model, latest) = text(o, &["transcript"])
        .strip_suffix(".jsonl")
        .map(|_| PathBuf::from(text(o, &["transcript"])))
        .map(|p| transcript_tail(&p))
        .unwrap_or_default();
    let idle_min = minutes_since(text(o, &["last_seen"]), now);
    let provider = Some(text(o, &["provider"]))
        .filter(|p| !p.is_empty())
        .unwrap_or("claude");
    let mut w = Workload::blank(&format!("s:{id}"), Role::Session, &label, "Coordinator");
    w.sub = model.unwrap_or_else(|| provider.to_owned());
    w.state = if idle_min.is_some_and(|m| m <= ACTIVE_MIN) {
        State::Working
    } else {
        State::Idle
    };
    w.idle_min = idle_min;
    w.why = Some(text(o, &["working_on"]).trim().to_owned()).filter(|s| !s.is_empty());
    w.latest = latest;
    w.agent_keys = [name.to_lowercase(), id.to_lowercase()]
        .into_iter()
        .filter(|k| !k.is_empty())
        .collect();
    w
}

/// The model family and the latest thing the assistant said, from the end of a transcript
/// (JSON lines). Both `None` when the file is missing or says neither.
fn transcript_tail(path: &Path) -> (Option<String>, Option<String>) {
    let read = || -> std::io::Result<String> {
        let mut f = std::fs::File::open(path)?;
        let len = f.metadata()?.len();
        f.seek(SeekFrom::Start(len.saturating_sub(TAIL)))?;
        let mut bytes = Vec::new();
        f.take(TAIL).read_to_end(&mut bytes)?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    };
    let Ok(tail) = read() else {
        return (None, None);
    };
    let (mut model, mut latest) = (None, None);
    for line in tail.lines().rev() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let msg = v.get("message").unwrap_or(&Value::Null);
        if model.is_none() {
            model = msg.get("model").and_then(Value::as_str).and_then(family);
        }
        if latest.is_none() && msg.get("role").and_then(Value::as_str) == Some("assistant") {
            latest = msg
                .get("content")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|c| c.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|c| c.get("text").and_then(Value::as_str))
                .find_map(|t| t.lines().map(str::trim).find(|l| !l.is_empty()))
                .map(str::to_owned);
        }
        if model.is_some() && latest.is_some() {
            break;
        }
    }
    (model, latest)
}

/// "Opus", "Sonnet", "Haiku" for a Claude model id; the id itself (cut) for any other, `None`
/// for the synthetic marker.
fn family(model: &str) -> Option<String> {
    let m = model.to_lowercase();
    if m.starts_with('<') || m.is_empty() {
        return None;
    }
    Some(
        ["opus", "sonnet", "haiku"]
            .into_iter()
            .find(|f| m.contains(f))
            .map_or_else(
                || model.chars().take(16).collect(),
                |f| {
                    let mut c = f.chars();
                    c.next()
                        .map(|first| first.to_uppercase().chain(c).collect())
                        .unwrap_or_default()
                },
            ),
    )
}

/// Memory and CPU of processes, from `ps` (one call): `412 MB  ·  38% CPU` per pid. Empty when
/// `ps` cannot answer.
pub(super) fn ps_uses(pids: &[u32]) -> HashMap<u32, String> {
    if pids.is_empty() {
        return HashMap::new();
    }
    let list = pids
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let Ok(out) = std::process::Command::new("ps")
        .args(["-o", "pid=,rss=,pcpu=", "-p", &list])
        .output()
    else {
        return HashMap::new();
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let mut f = l.split_whitespace();
            let pid = f.next()?.parse::<u32>().ok()?;
            let rss_kb = f.next()?.parse::<u64>().ok()?;
            let cpu = f.next()?.parse::<f64>().ok()?;
            Some((pid, format!("{} MB  ·  {cpu:.0}% CPU", rss_kb / 1024)))
        })
        .collect()
}

/// Whether a process runs (signal 0; a permission error means it does).
pub(super) fn pid_alive(pid: u32) -> bool {
    let Some(pid) = i32::try_from(pid)
        .ok()
        .and_then(rustix::process::Pid::from_raw)
    else {
        return false;
    };
    match rustix::process::test_kill_process(pid) {
        Ok(()) => true,
        Err(e) => e == rustix::io::Errno::PERM,
    }
}

// ------------------------------------------------------------------ the queue

/// The open jobs of a picture (`GET /v1/coordinator/picture`, with or without the `picture`
/// wrapper). Done jobs are not listed; a stopped one is stuck; one waiting on an owner or an
/// approval needs you; one with a lease is working; the rest wait, idle.
pub(super) fn queue(v: &Value, now: OffsetDateTime) -> Queue {
    let pic = v.get("picture").unwrap_or(v);
    let redacted = pic
        .get("tier")
        .and_then(Value::as_str)
        .is_some_and(|t| t != "full");
    let mut jobs: Vec<Workload> = pic
        .get("jobs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|j| !matches!(text(j, &["stage"]), "done" | "recorded"))
        .map(|j| job(j, now))
        .collect();
    let open = jobs.len();
    jobs.sort_by_key(|j| std::cmp::Reverse(j.state.rank()));
    jobs.truncate(JOBS);
    Queue {
        open,
        jobs,
        redacted,
    }
}

fn job(j: &Value, now: OffsetDateTime) -> Workload {
    let id = text(j, &["id", "job_id"]);
    let stage = text(j, &["stage"]);
    let kind = text(j, &["kind"]);
    let title = text(j, &["title"]).trim();
    let label = if title.is_empty() {
        format!("{} job", if kind.is_empty() { "queue" } else { kind })
    } else {
        title.to_owned()
    };
    let wait = text(j, &["await_kind"]);
    let hold = text(j, &["hold_reason"]);
    let now_state = if stage == "stopped" {
        State::Stuck
    } else if !wait.is_empty() && owner_wait(wait) {
        State::Needs
    } else if stage == "running" || j.get("has_lease").and_then(Value::as_bool) == Some(true) {
        State::Working
    } else {
        State::Idle
    };
    let mut w = Workload::blank(&format!("j:{id}"), Role::Job, &label, "Coordinator service");
    w.state = now_state;
    w.sub = Some(text(j, &["model", "model_tier"]))
        .filter(|m| !m.is_empty())
        .and_then(family)
        .unwrap_or_default();
    w.idle_min = minutes_since(text(j, &["updated_at"]), now);
    w.right = match now_state {
        State::Needs => None,
        _ if stage.is_empty() => None,
        _ => Some(stage.to_owned()),
    };
    w.why = Some(title.to_owned()).filter(|t| !t.is_empty());
    let pct = j.get("progress_pct").and_then(Value::as_u64).unwrap_or(0);
    w.latest = Some(format!(
        "{} stage {stage}, {pct}% of the way",
        if kind.is_empty() { "queue" } else { kind }
    ));
    w.waits = [
        (!wait.is_empty()).then(|| format!("waiting on {wait}")),
        (!hold.is_empty()).then(|| format!("held: {hold}")),
        Some(text(j, &["next_gate"]))
            .filter(|g| !g.is_empty())
            .map(|g| format!("next gate: {g}")),
    ]
    .into_iter()
    .flatten()
    .next();
    w.pr = Some(text(j, &["compliance_pr"]))
        .filter(|p| !p.is_empty())
        .map(str::to_owned);
    w
}

/// Whether a wait is on the owner (the Mac app's test on a job's wait).
fn owner_wait(wait: &str) -> bool {
    let w = wait.to_lowercase();
    ["approv", "owner", "decision", "you"]
        .iter()
        .any(|k| w.contains(k))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn now() -> OffsetDateTime {
        OffsetDateTime::parse("2026-11-07T12:00:00Z", &Rfc3339).expect("time")
    }

    fn session_file(dir: &Path, id: &str, body: &Value) {
        let d = dir.join("agents/sessions");
        std::fs::create_dir_all(&d).expect("dir");
        std::fs::write(d.join(format!("{id}.json")), body.to_string()).expect("file");
    }

    #[test]
    fn a_live_session_is_working_when_seen_lately_else_idle_and_a_dead_one_is_not_listed() {
        let t = tempfile::tempdir().expect("tmp");
        let log = t.path().join("t.jsonl");
        std::fs::write(
            &log,
            [
                json!({"message": {"role": "user", "content": "hi"}}).to_string(),
                json!({"message": {"role": "assistant", "model": "claude-sonnet-5-5",
                    "content": [{"type": "text", "text": "\nrewriting grant_access\nmore"}]}})
                .to_string(),
            ]
            .join("\n"),
        )
        .expect("log");
        session_file(
            t.path(),
            "a",
            &json!({"id": "aaaaaaaa-1", "pid": 1, "name": "Fix reads", "last_seen": "2026-11-07T11:55:00Z",
                "working_on": "clear the hold", "transcript": log, "provider": "claude"}),
        );
        session_file(
            t.path(),
            "b",
            &json!({"id": "bbbbbbbb-2", "pid": 2, "branch": "feat/x", "last_seen": "2026-11-07T11:30:00Z"}),
        );
        session_file(
            t.path(),
            "c",
            &json!({"id": "cccccccc-3", "pid": 3, "last_seen": "2026-11-07T11:59:00Z"}),
        );
        std::fs::write(t.path().join("agents/sessions/junk.json"), "{not json").expect("junk");
        let alive = |p: u32| p != 3;
        let uses = |p: &[u32]| {
            p.iter()
                .map(|p| (*p, "412 MB  ·  38% CPU".to_owned()))
                .collect()
        };
        let list = sessions(t.path(), now(), &alive, &uses);
        assert_eq!(list.len(), 2, "pid 3 is dead, the junk file is skipped");
        let a = &list[0];
        assert_eq!((a.label.as_str(), a.state), ("Fix reads", State::Working));
        assert_eq!(a.sub, "Sonnet");
        assert_eq!(a.latest.as_deref(), Some("rewriting grant_access"));
        assert_eq!(a.why.as_deref(), Some("clear the hold"));
        assert_eq!(a.uses.as_deref(), Some("412 MB  ·  38% CPU"));
        assert_eq!(a.right(), "5 min ago");
        let b = &list[1];
        assert_eq!((b.label.as_str(), b.state), ("feat/x", State::Idle));
        assert_eq!(
            (b.sub.as_str(), b.right().as_str()),
            ("claude", "idle 30 min")
        );
    }

    #[test]
    fn an_ask_naming_a_session_makes_it_need_you_and_the_root_follows_the_worst() {
        let snap = Snapshot {
            sessions: vec![
                {
                    let mut w = Workload::blank("s:aaaaaaaa-1", Role::Session, "Fix reads", "");
                    w.state = State::Working;
                    w.agent_keys = vec!["fix reads".into(), "aaaaaaaa-1".into()];
                    w
                },
                {
                    let mut w = Workload::blank("s:bbbbbbbb-2", Role::Session, "Other", "");
                    w.agent_keys = vec!["bbbbbbbb-2".into()];
                    w
                },
            ],
            queue: Ok(Queue {
                open: 0,
                jobs: vec![],
                redacted: false,
            }),
        };
        let asks = vec![(
            "apr_1".to_owned(),
            "Merge sdk".to_owned(),
            "Fix Reads".to_owned(),
        )];
        let rows = tree(&snap, &asks);
        assert_eq!(
            rows[0].state,
            State::Needs,
            "the root follows the worst session"
        );
        assert_eq!(rows[1].ask.as_deref(), Some("apr_1"));
        assert!(
            rows[1]
                .waits
                .as_deref()
                .is_some_and(|w| w.contains("approve on your phone"))
        );
        assert_eq!(rows[2].state, State::Idle);
        assert!(
            names(&["aaaaaaaa-1".into()], "aaaaaa"),
            "an id prefix of six or more"
        );
        assert!(!names(&["aaaaaaaa-1".into()], "aaaaa"), "five is too short");
        assert!(!names(&["x".into()], ""));
        let none = tree(&snap, &[]);
        assert_eq!(none[0].state, State::Working);
    }

    #[test]
    fn the_queue_lists_open_jobs_worst_first_and_counts_them_all() {
        let jobs: Vec<Value> = (0..9)
            .map(|i| json!({"id": format!("job_{i}"), "kind": "code", "stage": "approved", "title": format!("Job {i}"),
                "updated_at": "2026-11-07T11:00:00Z"}))
            .chain([
                json!({"id": "job_done", "stage": "done"}),
                json!({"id": "job_run", "kind": "code", "stage": "running", "has_lease": true, "model": "claude-opus-4-8",
                    "title": "Roll it", "updated_at": "2026-11-07T11:58:00Z"}),
                json!({"id": "job_ask", "stage": "planned", "await_kind": "owner approval", "title": "Merge sdk",
                    "next_gate": "approve", "compliance_pr": "inorbithr/sdk#448"}),
                json!({"id": "job_stop", "stage": "stopped", "hold_reason": "limit"}),
            ])
            .collect();
        let q = queue(&json!({"picture": {"tier": "full", "jobs": jobs}}), now());
        assert_eq!(q.open, 12, "the done job is not open");
        assert_eq!(q.jobs.len(), JOBS);
        assert!(!q.redacted);
        let states: Vec<State> = q.jobs.iter().map(|j| j.state).collect();
        assert_eq!(&states[..3], [State::Needs, State::Stuck, State::Working]);
        let ask = &q.jobs[0];
        assert_eq!(ask.waits.as_deref(), Some("waiting on owner approval"));
        assert_eq!(ask.pr.as_deref(), Some("inorbithr/sdk#448"));
        assert_eq!(ask.right(), "needs you");
        assert_eq!(q.jobs[1].label, "queue job", "no title: named by kind");
        let run = &q.jobs[2];
        assert_eq!(
            (run.sub.as_str(), run.right().as_str()),
            ("Opus", "running")
        );
        assert!(queue(&json!({"tier": "public", "jobs": []}), now()).redacted);
    }

    #[test]
    fn the_service_row_says_when_the_queue_could_not_be_read() {
        let snap = Snapshot {
            sessions: vec![],
            queue: Err("signed out".into()),
        };
        let rows = tree(&snap, &[]);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].right(), "unavailable");
        assert!(
            rows[1]
                .latest
                .as_deref()
                .is_some_and(|l| l.contains("signed out"))
        );
        assert_eq!(
            rows[0].state,
            State::Idle,
            "no session, nothing running here"
        );
    }

    #[test]
    fn glyphs_close_each_group_and_the_roots_have_none() {
        let snap = Snapshot {
            sessions: vec![
                Workload::blank("s:1", Role::Session, "a", ""),
                Workload::blank("s:2", Role::Session, "b", ""),
            ],
            queue: Ok(Queue {
                open: 1,
                jobs: vec![Workload::blank("j:1", Role::Job, "c", "")],
                redacted: false,
            }),
        };
        let rows = tree(&snap, &[]);
        let g: Vec<&str> = (0..rows.len()).map(|i| glyph(&rows, i)).collect();
        assert_eq!(g, ["", "├ ", "└ ", "", "└ "]);
    }

    #[test]
    fn family_names_the_claude_tiers_and_cuts_the_rest() {
        assert_eq!(family("claude-opus-4-8").as_deref(), Some("Opus"));
        assert_eq!(family("claude-haiku-4-5").as_deref(), Some("Haiku"));
        assert_eq!(family("<synthetic>"), None);
        assert_eq!(
            family("gpt-5-codex-experimental").as_deref(),
            Some("gpt-5-codex-expe")
        );
    }

    #[test]
    fn our_own_process_is_alive_and_an_absurd_pid_is_not() {
        assert!(pid_alive(std::process::id()));
        assert!(!pid_alive(u32::MAX));
        let used = ps_uses(&[std::process::id()]);
        assert!(used.values().all(|u| u.contains("MB") && u.contains("CPU")));
    }
}
