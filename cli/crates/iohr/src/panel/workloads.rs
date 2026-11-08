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

#[cfg(unix)]
mod read;
#[cfg(unix)]
pub(super) use read::{
    SessionRef, home_root, pid_alive, ps_uses, queue, session_ref, sessions, transcript_lines,
};

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
