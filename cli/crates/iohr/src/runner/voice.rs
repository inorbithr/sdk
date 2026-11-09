//! Voice leases (core RFC 0215): a realtime call's answers, on this runner.
//!
//! The runner listens on `ServeVoice` beside its turns. When a call starts, the coordinator
//! opens a lease here: the runner starts one warm `claude -p` for it, in stream-json both
//! ways, on the person's own sign-in (ADR 0074; no flag here points it at a key). Each turn
//! of the call is written to that session's stdin with its `seq`; the answer goes back
//! piece by piece as Claude writes it (`ReportVoice`), so the first sentence can be spoken
//! while the rest is still coming. Claude answers queued messages one after the other, one
//! `result` each, so the k-th result answers the k-th turn written. A turn that a newer one
//! replaced (barge-in) is answered to nobody: its pieces are dropped here, and the
//! coordinator drops any that still reach it.
//!
//! A call's first answer would wait on that session's start (core PRD 0204 measured turn 1 at
//! 2.37 s, turn 2 at 1.01 s once warm), so while it serves voice the runner keeps one spare:
//! a session started and warmed up the way the newest lease's was, which the next lease
//! started the same way takes instead of starting its own. A lease that asks for another
//! model or other instructions starts its own, as before, and the spare waits for the next.
//! The spare counts toward [`MAX_LEASES`], dies with the listener, and is replaced when it
//! ends or waits past [`SPARE_TTL`], so a stale sign-in never answers a call. Each spare runs
//! a warm-up turn, so one is kept only within [`SPARE_WINDOW`] of the last lease taken or
//! closed: a runner nobody called for that long lets its spare age out and starts the next
//! only when a lease comes again (that lease starts cold).
//!
//! Nothing a person said or Claude wrote is logged or noted: only counts and labels.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use inorbithr::{EventStream, Method, Operation};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

use super::claude::{LEAN_SESSION, assert_honest};
use super::exec::{ClaudeExec, GroupGuard};
use super::service::{HttpService, ServiceError};
use crate::ask::Agent;
use crate::ask::stream::{self, Event};

/// The model a lease answers with when it names none: the fastest to its first word
/// (core RFC 0215 measured haiku's first piece at p50 0.54 s on a warm session).
pub(crate) const DEFAULT_MODEL: &str = "haiku";
/// The largest piece the coordinator takes.
pub(crate) const MAX_PIECE: usize = 4096;
/// Leases held at once; a call more than this is refused here and the coordinator falls
/// back to the thread. The spare is one of them: sessions held and the spare together are
/// never more than this, and a lease is never refused for the spare's sake.
const MAX_LEASES: usize = 4;
/// How long a spare waits for a call before another replaces it, so the session that
/// answers a call never started on a sign-in older than this.
const SPARE_TTL: Duration = Duration::from_mins(10);
/// A spare is started (or replaced) only this long after the last lease was taken or
/// closed: without calls, a runner stops paying a warm-up turn every [`SPARE_TTL`].
const SPARE_WINDOW: Duration = Duration::from_mins(30);
/// After a spare that would not start or ended on its own, the wait before the next: a
/// runner whose Claude cannot start, or exits once it has answered (signed out), does not
/// start it again and again.
const SPARE_RETRY: Duration = Duration::from_secs(30);
/// How often the listener looks at its spare while no frame comes.
const TEND: Duration = Duration::from_secs(30);
/// Pieces waiting to be posted before the session's reader waits.
const QUEUE: usize = 256;
/// Written to a session as it starts, before the person speaks, so the first request's
/// cold start (several seconds, RFC 0215) is paid while the call is still connecting.
/// Its answer is turn 0, which is never live: nobody hears it.
const WARM_UP: &str = "The call is connecting. Reply with one word.";

/// One piece of an answer, as `ReportVoice` takes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Piece {
    pub(crate) seq: u64,
    pub(crate) text: String,
    pub(crate) done: bool,
    /// Empty, or why the answer failed: a label, never the text.
    pub(crate) failed: &'static str,
}

/// What `ServeVoice` sent.
#[derive(Debug, PartialEq, Eq)]
enum Frame {
    Open {
        lease_id: String,
        instructions: String,
        model: String,
    },
    Turn {
        lease_id: String,
        seq: u64,
        text: String,
    },
    Close {
        lease_id: String,
    },
    /// A heartbeat, or an event this runner does not know.
    Other,
}

fn str_of<'v>(v: &'v Value, key: &str) -> &'v str {
    v.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// A `uint64` as the gateway writes it: a string, or a number.
fn u64_of(v: &Value, key: &str) -> u64 {
    match v.get(key) {
        Some(Value::String(s)) => s.parse().unwrap_or(0),
        Some(n) => n.as_u64().unwrap_or(0),
        None => 0,
    }
}

fn frame(v: &Value) -> Frame {
    if let Some(o) = v.get("open") {
        return Frame::Open {
            lease_id: str_of(o, "lease_id").to_owned(),
            instructions: str_of(o, "instructions").to_owned(),
            model: str_of(o, "model").to_owned(),
        };
    }
    if let Some(t) = v.get("turn") {
        return Frame::Turn {
            lease_id: str_of(t, "lease_id").to_owned(),
            seq: u64_of(t, "seq"),
            text: str_of(t, "text").to_owned(),
        };
    }
    if let Some(c) = v.get("close") {
        return Frame::Close {
            lease_id: str_of(c, "lease_id").to_owned(),
        };
    }
    Frame::Other
}

/// An id the coordinator minted: safe in a path.
fn is_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

impl HttpService<'_> {
    /// `ServeVoice`: this runner's voice leases, as they open, take turns and close.
    pub(crate) async fn voice_events(
        &self,
        runner_id: &str,
    ) -> Result<EventStream<Value>, ServiceError> {
        if !is_id(runner_id) {
            return Err(ServiceError::Refused("a runner id that is not one"));
        }
        let op = Operation::new(
            Method::Get,
            format!("/v1/coordinator/runners/{runner_id}/voice/events"),
        )
        .query("account_id", self.account.as_str());
        Ok(self.api.client().stream(op).await?)
    }

    /// `ReportVoice`: one piece of a lease's answer. `false` when the turn was stale and
    /// the coordinator dropped it.
    pub(crate) async fn report_voice(
        &self,
        lease_id: &str,
        piece: &Piece,
    ) -> Result<bool, ServiceError> {
        if !is_id(lease_id) {
            return Err(ServiceError::Refused("a lease id that is not one"));
        }
        let body = json!({
            "seq": piece.seq.to_string(),
            "text": piece.text,
            "done": piece.done,
            "failed": piece.failed,
        });
        let v: Value = self
            .api
            .send_body(
                Method::Post,
                &format!("/v1/coordinator/voice/{lease_id}/deltas"),
                &self.scoped(body),
            )
            .await?
            .json()?;
        Ok(v.get("accepted").and_then(Value::as_bool).unwrap_or(false))
    }
}

/// The turns written to a session, oldest first, the newest turn of the lease, and the
/// lease: set once, when a lease takes the session (a spare has none until then).
#[derive(Debug, Default)]
struct Turns {
    written: Mutex<VecDeque<u64>>,
    latest: AtomicU64,
    lease: OnceLock<String>,
}

impl Turns {
    fn front(&self) -> Option<u64> {
        self.written
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .front()
            .copied()
    }

    fn pop(&self) -> Option<u64> {
        self.written
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
    }

    /// Whether `seq` is the turn the person is waiting on. Turn 0 is the warm-up, and a
    /// session no lease took has nobody waiting.
    fn live(&self, seq: u64) -> bool {
        seq != 0 && seq == self.latest.load(Ordering::Acquire) && self.lease.get().is_some()
    }

    fn lease(&self) -> String {
        self.lease.get().cloned().unwrap_or_default()
    }

    /// Whether every turn written was answered: for a spare, that its warm-up was.
    fn answered(&self) -> bool {
        self.written
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty()
    }
}

/// One lease's warm Claude session, or the spare. Dropping it kills the session's whole
/// process group and stops its reader.
struct Session {
    /// What it was started with, to start it again the same way. A spare is taken only by
    /// a lease whose arguments are these, so they are its key: the model and the
    /// instructions, compared whole.
    args: Vec<String>,
    stdin: ChildStdin,
    turns: Arc<Turns>,
    reader: JoinHandle<()>,
    born: Instant,
    group: GroupGuard,
    child: Child,
}

impl Drop for Session {
    fn drop(&mut self) {
        self.reader.abort();
    }
}

/// The arguments of a lease's session.
pub(crate) fn session_args(instructions: &str, model: &str) -> Result<Vec<String>, String> {
    let model = if model.trim().is_empty() {
        DEFAULT_MODEL
    } else {
        model.trim()
    };
    // A model name, never a flag or anything else.
    if model.starts_with('-')
        || !model
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_' | b'[' | b']'))
    {
        return Err("a model that is not a model name".into());
    }
    let mut a: Vec<String> = [
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--include-partial-messages",
        "--verbose",
        "--model",
        model,
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect();
    a.extend(LEAN_SESSION.iter().map(|s| (*s).to_owned()));
    a.extend([
        "--system-prompt".to_owned(),
        instructions.to_owned(),
        // A spoken answer uses no tools.
        "--tools".to_owned(),
        String::new(),
    ]);
    assert_honest(&a).map_err(|e| e.to_string())?;
    Ok(a)
}

/// One turn as a stream-json user message.
fn user_line(text: &str) -> String {
    let mut line = json!({
        "type": "user",
        "message": {"role": "user", "content": [{"type": "text", "text": text}]},
    })
    .to_string();
    line.push('\n');
    line
}

/// Where the voice sessions run: beside the turns' working directory, not in it, so no
/// checkout's settings apply and a turn's session is never resumed by a call.
fn voice_dir(exec: &ClaudeExec) -> PathBuf {
    exec.workdir.parent().unwrap_or(&exec.workdir).join("voice")
}

/// Starts a session on `args` (from [`session_args`]) and writes its warm-up; a lease takes
/// it with [`Session::take`].
async fn open(
    exec: &ClaudeExec,
    args: &[String],
    tx: mpsc::Sender<(String, Piece)>,
) -> Result<Session, &'static str> {
    let dir = voice_dir(exec);
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|_| "workdir")?;
    let mut cmd = exec.session_command(args, &dir);
    // A spoken answer starts at once: no thinking before the first word.
    cmd.env("MAX_THINKING_TOKENS", "0");
    let mut child = cmd.spawn().map_err(|_| "spawn")?;
    let group = GroupGuard::new(child.id());
    let stdin = child.stdin.take().ok_or("spawn")?;
    let stdout = child.stdout.take().ok_or("spawn")?;
    let turns = Arc::new(Turns::default());
    let reader = tokio::spawn(read(stdout, turns.clone(), tx));
    let mut s = Session {
        args: args.to_vec(),
        stdin,
        turns,
        reader,
        born: Instant::now(),
        group,
        child,
    };
    s.send(0, WARM_UP).await.map_err(|_| "write")?;
    Ok(s)
}

/// Reads a session's stdout, turns it into pieces of the turn it answers, and fails each
/// turn still waiting when the session ends.
async fn read(stdout: ChildStdout, turns: Arc<Turns>, tx: mpsc::Sender<(String, Piece)>) {
    let mut lines = BufReader::new(stdout).lines();
    let mut texted = false;
    let piece = |seq, text: String, done, failed| Piece {
        seq,
        text,
        done,
        failed,
    };
    while let Ok(Some(line)) = lines.next_line().await {
        for ev in stream::parse(Agent::Claude, &line) {
            match ev {
                Event::Text(text) => {
                    if let Some(seq) = turns.front()
                        && turns.live(seq)
                    {
                        texted = true;
                        if tx
                            .send((turns.lease(), piece(seq, text, false, "")))
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                }
                Event::Done(d) => {
                    let Some(seq) = turns.pop() else { continue };
                    let was_texted = std::mem::take(&mut texted);
                    if !turns.live(seq) {
                        continue;
                    }
                    let (text, failed) = match (d.error, d.answer) {
                        (Some(_), _) => (String::new(), "model"),
                        // An answer that came whole, with no pieces before it.
                        (None, Some(a)) if !was_texted => (a, ""),
                        (None, _) => (String::new(), ""),
                    };
                    if tx
                        .send((turns.lease(), piece(seq, text, true, failed)))
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
                _ => {}
            }
        }
    }
    // The session ended: whatever it still owed fails.
    while let Some(seq) = turns.pop() {
        if turns.live(seq) {
            let _ = tx
                .send((turns.lease(), piece(seq, String::new(), true, "model")))
                .await;
        }
    }
}

/// `text` cut into pieces of at most [`MAX_PIECE`] bytes, on character boundaries.
fn cut(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = text;
    while rest.len() > MAX_PIECE {
        let mut at = MAX_PIECE;
        while !rest.is_char_boundary(at) {
            at -= 1;
        }
        let (head, tail) = rest.split_at(at);
        out.push(head);
        rest = tail;
    }
    out.push(rest);
    out
}

/// Posts the pieces in order. Pieces of one turn that queued up while a post was on the
/// wire go together in the next one.
async fn post(
    svc: &HttpService<'_>,
    mut rx: mpsc::Receiver<(String, Piece)>,
    note: &impl Fn(&str),
) {
    let mut held: Option<(String, Piece)> = None;
    loop {
        let (lease, mut piece) = match held.take() {
            Some(p) => p,
            None => match rx.recv().await {
                Some(p) => p,
                None => return,
            },
        };
        while !piece.done {
            match rx.try_recv() {
                Ok((l, p)) if l == lease && p.seq == piece.seq => {
                    piece.text.push_str(&p.text);
                    piece.done = p.done;
                    piece.failed = p.failed;
                }
                Ok(other) => {
                    held = Some(other);
                    break;
                }
                Err(_) => break,
            }
        }
        let parts = cut(&piece.text);
        let last = parts.len().saturating_sub(1);
        for (i, text) in parts.into_iter().enumerate() {
            let one = Piece {
                seq: piece.seq,
                text: text.to_owned(),
                done: piece.done && i == last,
                failed: if i == last { piece.failed } else { "" },
            };
            match svc.report_voice(&lease, &one).await {
                // `false` is stale: a newer turn took over, and the coordinator drops the
                // rest of this one too.
                Ok(_) => {}
                Err(e) => {
                    note(&format!(
                        "voice: a piece of lease {lease} was not delivered ({})",
                        status_of(&e)
                    ));
                    break;
                }
            }
        }
    }
}

/// A refusal's HTTP status, never its message.
fn status_of(e: &ServiceError) -> String {
    match e {
        ServiceError::Api(inorbithr::Error::Api(a)) => a.status.to_string(),
        ServiceError::Api(_) => "network".into(),
        ServiceError::Refused(what) => (*what).to_owned(),
    }
}

/// How long the listener waits before it opens `ServeVoice` again.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Pace {
    pub(crate) backoff: Duration,
    /// Stop after this many streams (tests); `None` is never.
    pub(crate) rounds: Option<usize>,
}

impl Default for Pace {
    fn default() -> Self {
        Self {
            backoff: Duration::from_secs(5),
            rounds: None,
        }
    }
}

/// Serves this runner's voice leases until `stop`. A coordinator without voice leases
/// (an older one), or one that refuses this runner for good, ends it quietly: the runner's
/// turns go on as before.
pub(crate) async fn serve(
    svc: &HttpService<'_>,
    exec: &ClaudeExec,
    runner_id: &str,
    stop: watch::Receiver<bool>,
    pace: Pace,
    note: &impl Fn(&str),
) {
    let (tx, rx) = mpsc::channel(QUEUE);
    tokio::join!(
        listen(svc, Leases::new(exec, tx), runner_id, stop, pace, note),
        post(svc, rx, note)
    );
}

/// The leases this runner holds, each with its warm session, and the spare.
struct Leases<'e> {
    exec: &'e ClaudeExec,
    sessions: HashMap<String, Session>,
    tx: mpsc::Sender<(String, Piece)>,
    /// A session started and warmed up for the next lease, which no lease holds yet.
    spare: Option<Session>,
    /// What the spare starts with: the newest lease's arguments. None before the first
    /// lease, as the instructions come only with a lease.
    want: Option<Vec<String>>,
    /// No spare starts before this.
    spare_after: Option<Instant>,
    /// When a lease was last taken or closed; None before the first.
    last_lease: Option<Instant>,
    spare_ttl: Duration,
    spare_window: Duration,
    /// How often the listener tends the spare while no frame comes.
    tend_every: Duration,
}

impl<'e> Leases<'e> {
    fn new(exec: &'e ClaudeExec, tx: mpsc::Sender<(String, Piece)>) -> Self {
        Self {
            exec,
            sessions: HashMap::new(),
            tx,
            spare: None,
            want: None,
            spare_after: None,
            last_lease: None,
            spare_ttl: SPARE_TTL,
            spare_window: SPARE_WINDOW,
            tend_every: TEND,
        }
    }

    async fn on(&mut self, frame: Frame, note: &impl Fn(&str)) {
        match frame {
            Frame::Open {
                lease_id,
                instructions,
                model,
            } => self.open(lease_id, &instructions, &model, note).await,
            Frame::Turn {
                lease_id,
                seq,
                text,
            } => self.turn(lease_id, seq, &text, note).await,
            Frame::Close { lease_id } => {
                if self.sessions.remove(&lease_id).is_some() {
                    self.last_lease = Some(Instant::now());
                    note(&format!("voice: lease {lease_id} closed"));
                }
            }
            Frame::Other => {}
        }
        self.tend(note).await;
    }

    async fn open(
        &mut self,
        lease_id: String,
        instructions: &str,
        model: &str,
        note: &impl Fn(&str),
    ) {
        if self.sessions.contains_key(&lease_id) || !is_id(&lease_id) {
            return;
        }
        if self.sessions.len() >= MAX_LEASES {
            note(&format!(
                "voice: lease {lease_id} refused, {MAX_LEASES} held"
            ));
            return;
        }
        let Ok(args) = session_args(instructions, model) else {
            note(&format!("voice: lease {lease_id} not opened (refused)"));
            return;
        };
        match self.session(&lease_id, &args, 1, note).await {
            Ok((s, warm)) => {
                let on = if warm { ", on the spare" } else { "" };
                note(&format!("voice: lease {lease_id} open{on}"));
                self.sessions.insert(lease_id, s);
            }
            Err(why) => note(&format!("voice: lease {lease_id} not opened ({why})")),
        }
        self.want = Some(args);
        self.last_lease = Some(Instant::now());
    }

    /// A session on `args` for `lease_id`: the spare when it was started on the same
    /// arguments and is still fit to answer, else a new one (`true` when it was the
    /// spare). `adds` is how many sessions the lease adds to those held: a spare that is
    /// not taken is stopped only when keeping it would go past [`MAX_LEASES`].
    async fn session(
        &mut self,
        lease_id: &str,
        args: &[String],
        adds: usize,
        note: &impl Fn(&str),
    ) -> Result<(Session, bool), &'static str> {
        self.check_spare(note);
        if let Some(s) = self.spare.take_if(|s| s.args == args) {
            s.take(lease_id);
            return Ok((s, true));
        }
        if self.spare.is_some() && self.sessions.len() + adds + 1 > MAX_LEASES {
            self.spare = None;
        }
        let s = open(self.exec, args, self.tx.clone()).await?;
        s.take(lease_id);
        Ok((s, false))
    }

    /// Whether a spare is kept now: a lease was taken or closed within the window.
    fn calling(&self) -> bool {
        self.last_lease
            .is_some_and(|t| t.elapsed() < self.spare_window)
    }

    /// Stops a spare that ended or waited too long. A spare ends only on its own (it
    /// would not start, or it exited after its warm-up, as a signed-out Claude does), so
    /// the next waits [`SPARE_RETRY`].
    fn check_spare(&mut self, note: &impl Fn(&str)) {
        let Some(s) = self.spare.as_mut() else { return };
        if s.ended() {
            let when = if s.turns.answered() {
                "after its warm-up"
            } else {
                "before its warm-up"
            };
            let next = if self.calling() {
                format!("another in {} s at the soonest", SPARE_RETRY.as_secs())
            } else {
                "none until the next lease".to_owned()
            };
            note(&format!("voice: the spare session ended ({when}), {next}"));
            self.spare_after = Some(Instant::now() + SPARE_RETRY);
            self.spare = None;
        } else if s.born.elapsed() >= self.spare_ttl {
            self.spare = None;
        }
    }

    /// Keeps one spare ready while calls come, there is room for it and a lease to model
    /// it on.
    async fn tend(&mut self, note: &impl Fn(&str)) {
        self.check_spare(note);
        if self.spare.is_some()
            || !self.calling()
            || self.sessions.len() >= MAX_LEASES
            || self.spare_after.is_some_and(|t| Instant::now() < t)
        {
            return;
        }
        let Some(args) = self.want.as_deref() else {
            return;
        };
        match open(self.exec, args, self.tx.clone()).await {
            Ok(s) => {
                self.spare = Some(s);
                self.spare_after = None;
            }
            Err(why) => {
                note(&format!("voice: no spare session ({why})"));
                self.spare_after = Some(Instant::now() + SPARE_RETRY);
            }
        }
    }

    /// Stops every session and the spare: the listener is not listening.
    fn clear(&mut self) {
        self.sessions.clear();
        self.spare = None;
    }

    async fn turn(&mut self, lease_id: String, seq: u64, text: &str, note: &impl Fn(&str)) {
        let Some(s) = self.sessions.get_mut(&lease_id) else {
            return;
        };
        // An old or repeated turn: the coordinator sends each seq once, newest last.
        if seq <= s.turns.latest.load(Ordering::Acquire) {
            return;
        }
        if s.write(seq, text).await.is_ok() {
            return;
        }
        // The session died since the last turn: start it again, or take the spare (either
        // starts with no memory of the call), and write the turn there.
        note(&format!(
            "voice: lease {lease_id} session ended, starting again"
        ));
        let args = std::mem::take(&mut s.args);
        self.sessions.remove(&lease_id);
        let again = if let Ok((mut fresh, _)) = self.session(&lease_id, &args, 1, note).await {
            let wrote = fresh.write(seq, text).await;
            self.sessions.insert(lease_id.clone(), fresh);
            wrote.is_ok()
        } else {
            false
        };
        if !again {
            let _ = self
                .tx
                .send((
                    lease_id,
                    Piece {
                        seq,
                        text: String::new(),
                        done: true,
                        failed: "model",
                    },
                ))
                .await;
        }
    }
}

impl Session {
    /// A lease takes the session: from here on its live turns' pieces go to that lease.
    fn take(&self, lease_id: &str) {
        let _ = self.turns.lease.set(lease_id.to_owned());
    }

    /// Whether the session's process has ended.
    fn ended(&mut self) -> bool {
        match self.child.try_wait() {
            Ok(None) => false,
            Ok(Some(_)) => {
                // Reaped: its pid may be reused, so its group is not killed on drop.
                self.group.disarm();
                true
            }
            Err(_) => true,
        }
    }

    /// Writes turn `seq`; it is the lease's newest from here on.
    async fn write(&mut self, seq: u64, text: &str) -> std::io::Result<()> {
        self.turns.latest.store(seq, Ordering::Release);
        self.send(seq, text).await
    }

    /// Writes `text` as turn `seq`, whose answer is the next one not yet read.
    async fn send(&mut self, seq: u64, text: &str) -> std::io::Result<()> {
        self.turns
            .written
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(seq);
        self.stdin.write_all(user_line(text).as_bytes()).await?;
        self.stdin.flush().await
    }
}

async fn listen(
    svc: &HttpService<'_>,
    mut leases: Leases<'_>,
    runner_id: &str,
    mut stop: watch::Receiver<bool>,
    pace: Pace,
    note: &impl Fn(&str),
) {
    let mut round = 0_usize;
    while !*stop.borrow() {
        match svc.voice_events(runner_id).await {
            Ok(mut events) => {
                // Listening: a spare is kept from here until the stream ends.
                let mut tend = tokio::time::interval(leases.tend_every);
                tend.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                loop {
                    let next = tokio::select! {
                        _ = stop.changed() => return,
                        _ = tend.tick() => {
                            leases.tend(note).await;
                            continue;
                        }
                        n = events.next() => n,
                    };
                    let Some(Ok(v)) = next else { break };
                    leases.on(frame(&v), note).await;
                }
            }
            Err(e) if e.is_missing() || e.is_permanent() => {
                note(&format!(
                    "voice: not served here ({}); turns go on",
                    status_of(&e)
                ));
                return;
            }
            Err(_) => {}
        }
        // The stream ended, and its leases with it on the coordinator (`runner_gone`); the
        // spare too, as nothing is served until the stream opens again.
        leases.clear();
        round += 1;
        if pace.rounds.is_some_and(|m| round >= m) {
            return;
        }
        tokio::select! {
            _ = stop.changed() => return,
            () = tokio::time::sleep(pace.backoff) => {}
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use std::sync::Arc;

    use iohr_auth::{Redacted, StaticToken};
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::context::{AnyCredential, Api};
    use crate::runner::exec::settle_script;

    #[test]
    fn frames_read_as_the_gateway_writes_them() {
        assert_eq!(
            frame(
                &json!({"open": {"lease_id": "vl_1", "thread_id": "t", "instructions": "be Io", "model": ""}})
            ),
            Frame::Open {
                lease_id: "vl_1".into(),
                instructions: "be Io".into(),
                model: String::new()
            }
        );
        assert_eq!(
            frame(&json!({"turn": {"lease_id": "vl_1", "seq": "7", "text": "hi"}})),
            Frame::Turn {
                lease_id: "vl_1".into(),
                seq: 7,
                text: "hi".into()
            }
        );
        assert_eq!(
            frame(&json!({"turn": {"lease_id": "vl_1", "seq": 8}})),
            Frame::Turn {
                lease_id: "vl_1".into(),
                seq: 8,
                text: String::new()
            }
        );
        assert_eq!(
            frame(&json!({"close": {"lease_id": "vl_1", "reason": "hung_up"}})),
            Frame::Close {
                lease_id: "vl_1".into()
            }
        );
        assert_eq!(frame(&json!({"heartbeat": {}})), Frame::Other);
    }

    #[test]
    fn the_session_is_lean_honest_toolless_and_haiku_by_default() {
        let a = session_args("be Io", "").unwrap();
        let at = |f: &str| a.iter().position(|x| x == f).unwrap();
        assert_eq!(a[at("--model") + 1], DEFAULT_MODEL);
        assert_eq!(a[at("--system-prompt") + 1], "be Io");
        assert_eq!(a[at("--tools") + 1], "");
        assert_eq!(a[at("--input-format") + 1], "stream-json");
        assert!(a.iter().any(|x| x == "--strict-mcp-config"));
        assert_eq!(
            session_args("x", "sonnet").unwrap()[at("--model") + 1],
            "sonnet"
        );
        // A model that is not a model name never reaches claude.
        assert!(session_args("x", "--bare").is_err());
        assert!(session_args("x", "a b").is_err());
    }

    #[test]
    fn a_long_piece_is_cut_on_character_boundaries() {
        let text = "é".repeat(MAX_PIECE);
        let parts = cut(&text);
        assert!(parts.len() > 1);
        assert!(parts.iter().all(|p| p.len() <= MAX_PIECE));
        assert_eq!(parts.concat(), text);
        assert_eq!(cut(""), vec![""]);
    }

    const ANSWER: &str = concat!(
        r#"echo '{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello "}}}'"#,
        "\n",
        r#"echo '{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"there."}}}'"#,
        "\n",
        r#"echo '{"type":"result","subtype":"success","is_error":false,"result":"Hello there.","session_id":"s1"}'"#,
        "\n",
    );

    /// A `claude` that records its arguments, environment and directory, then runs
    /// `body` (which reads stream-json lines and answers them the way
    /// `claude -p --include-partial-messages` does). It never echoes what it was told. Each
    /// record is written whole (a rename), as the spare runs the same script beside it.
    fn fake_claude(dir: &std::path::Path, body: &str) -> ClaudeExec {
        let p = dir.join("claude");
        let script = format!(
            "#!/bin/sh\n[ \"$1\" = --version ] && {{ echo 1; exit 0; }}\nd=$(dirname \"$0\")\nprintf '%s\\n' \"$@\" > \"$d/args.$$\"\nenv > \"$d/env.$$\"\npwd > \"$d/pwd.$$\"\nfor f in args env pwd; do mv \"$d/$f.$$\" \"$d/$f.txt\"; done\n{body}\n"
        );
        std::fs::write(&p, script).unwrap();
        settle_script(&p);
        ClaudeExec::new(
            p,
            dir.join("work"),
            Vec::<(std::ffi::OsString, std::ffi::OsString)>::new(),
            None,
            "run_1".into(),
            "usr_1".into(),
        )
    }

    fn open_frame() -> Frame {
        open_as("vl_1", "")
    }

    fn open_as(lease_id: &str, model: &str) -> Frame {
        Frame::Open {
            lease_id: lease_id.into(),
            instructions: "be Io".into(),
            model: model.into(),
        }
    }

    fn close(lease_id: &str) -> Frame {
        Frame::Close {
            lease_id: lease_id.into(),
        }
    }

    /// Counts its starts by pid (`$$` is the session's pid: `claude` runs unwrapped), then
    /// answers every line it reads.
    const COUNTED: &str = "echo $$ >> \"$d/starts.txt\"\nwhile read -r line; do\n";

    fn counted(dir: &std::path::Path) -> ClaudeExec {
        fake_claude(dir, &format!("{COUNTED}{ANSWER}done"))
    }

    /// The pids `fake_claude` started, sorted, once there are `n` (and no more).
    async fn started(dir: &std::path::Path, n: usize) -> Vec<u32> {
        let read = || {
            std::fs::read_to_string(dir.join("starts.txt"))
                .unwrap_or_default()
                .lines()
                .map(|l| l.parse().unwrap())
                .collect::<Vec<u32>>()
        };
        let waited = tokio::time::timeout(Duration::from_secs(5), async {
            while read().len() < n {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await;
        let mut got = read();
        got.sort_unstable();
        assert!(waited.is_ok(), "{n} starts expected, {} seen", got.len());
        assert_eq!(got.len(), n, "{got:?}");
        got
    }

    fn pid(s: &Session) -> u32 {
        s.child.id().unwrap()
    }

    fn spare_pid(leases: &Leases<'_>) -> u32 {
        pid(leases.spare.as_ref().unwrap())
    }

    fn turn(seq: u64, text: &str) -> Frame {
        turn_on("vl_1", seq, text)
    }

    fn turn_on(lease_id: &str, seq: u64, text: &str) -> Frame {
        Frame::Turn {
            lease_id: lease_id.into(),
            seq,
            text: text.into(),
        }
    }

    /// Pieces until turn `seq` is done.
    async fn until_done(rx: &mut mpsc::Receiver<(String, Piece)>, seq: u64) -> Vec<Piece> {
        until_done_on(rx, "vl_1", seq).await
    }

    /// Pieces until turn `seq` of `lease_id` is done; every piece is that lease's.
    async fn until_done_on(
        rx: &mut mpsc::Receiver<(String, Piece)>,
        lease_id: &str,
        seq: u64,
    ) -> Vec<Piece> {
        let mut got = Vec::new();
        tokio::time::timeout(Duration::from_secs(10), async {
            while let Some((lease, p)) = rx.recv().await {
                assert_eq!(lease, lease_id);
                let last = p.done && p.seq == seq;
                got.push(p);
                if last {
                    break;
                }
            }
        })
        .await
        .unwrap();
        got
    }

    #[tokio::test]
    async fn a_lease_answers_each_turn_in_pieces_on_one_warm_session() {
        let dir = tempfile::tempdir().unwrap();
        let exec = fake_claude(dir.path(), &format!("while read -r line; do\n{ANSWER}done"));
        let (tx, mut rx) = mpsc::channel(QUEUE);
        let mut leases = Leases::new(&exec, tx);
        let notes = Mutex::new(Vec::<String>::new());
        // (The warm-up's answer, turn 0, must not come out below.)
        let note = |l: &str| notes.lock().unwrap().push(l.to_owned());
        leases.on(open_frame(), &note).await;
        leases.on(turn(1, "first"), &note).await;
        let one = until_done(&mut rx, 1).await;
        leases.on(turn(2, "second"), &note).await;
        let two = until_done(&mut rx, 2).await;
        for (seq, got) in [(1, one), (2, two)] {
            let text: String = got.iter().map(|p| p.text.as_str()).collect();
            assert_eq!(text, "Hello there.", "turn {seq}: {got:?}");
            assert!(got.iter().all(|p| p.seq == seq && p.failed.is_empty()));
            assert!(got.len() > 1, "the answer came in pieces");
            assert_eq!(got.iter().filter(|p| p.done).count(), 1);
        }
        let args = std::fs::read_to_string(dir.path().join("args.txt")).unwrap();
        assert!(args.contains("--input-format\nstream-json\n"), "{args}");
        assert!(args.contains("--model\nhaiku\n"), "{args}");
        let env = std::fs::read_to_string(dir.path().join("env.txt")).unwrap();
        assert!(env.contains("MAX_THINKING_TOKENS=0"));
        let pwd = std::fs::read_to_string(dir.path().join("pwd.txt")).unwrap();
        assert!(pwd.trim_end().ends_with("/voice"), "{pwd}");
        leases.on(close("vl_1"), &note).await;
        assert!(leases.sessions.is_empty());
        // Notes name the lease, never what was said or answered.
        let notes = notes.into_inner().unwrap().join("\n");
        assert!(notes.contains("vl_1"));
        for said in ["first", "second", "Hello", "be Io"] {
            assert!(!notes.contains(said), "{notes}");
        }
    }

    #[tokio::test]
    async fn a_replaced_turn_gets_no_pieces() {
        let dir = tempfile::tempdir().unwrap();
        // Reads both turns before it answers either, as when the person speaks again
        // before the first answer starts.
        let exec = fake_claude(
            dir.path(),
            &format!("read -r w\nread -r a\nread -r b\n{ANSWER}{ANSWER}{ANSWER}cat > /dev/null"),
        );
        let (tx, mut rx) = mpsc::channel(QUEUE);
        let mut leases = Leases::new(&exec, tx);
        leases.on(open_frame(), &|_: &str| {}).await;
        leases.on(turn(1, "a"), &|_: &str| {}).await;
        leases.on(turn(2, "b"), &|_: &str| {}).await;
        // A repeated or older turn is not written again.
        leases.on(turn(2, "b"), &|_: &str| {}).await;
        leases.on(turn(1, "a"), &|_: &str| {}).await;
        let got = until_done(&mut rx, 2).await;
        assert!(got.iter().all(|p| p.seq == 2), "{got:?}");
        let text: String = got.iter().map(|p| p.text.as_str()).collect();
        assert_eq!(text, "Hello there.");
    }

    #[tokio::test]
    async fn a_session_that_dies_fails_its_turn_and_starts_again() {
        let dir = tempfile::tempdir().unwrap();
        // Counts its starts, then exits after reading one turn without answering it.
        let exec = fake_claude(
            dir.path(),
            "echo $$ >> \"$d/starts.txt\"\nread -r w\nread -r a\nexit 1",
        );
        let (tx, mut rx) = mpsc::channel(QUEUE);
        let mut leases = Leases::new(&exec, tx);
        leases.on(open_frame(), &|_: &str| {}).await;
        leases.on(turn(1, "a"), &|_: &str| {}).await;
        let got = until_done(&mut rx, 1).await;
        assert_eq!(got.last().unwrap().failed, "model");
        // The next turn finds the session gone and takes the spare, started the same way,
        // in its place; another spare is started for the next call.
        leases.on(turn(2, "b"), &|_: &str| {}).await;
        let got = until_done(&mut rx, 2).await;
        assert!(got.last().unwrap().done);
        started(dir.path(), 3).await;
        assert!(leases.sessions.contains_key("vl_1"));
    }

    #[tokio::test]
    async fn a_lease_takes_the_ready_spare_instead_of_starting_claude() {
        let dir = tempfile::tempdir().unwrap();
        let exec = counted(dir.path());
        let (tx, mut rx) = mpsc::channel(QUEUE);
        let mut leases = Leases::new(&exec, tx);
        let notes = Mutex::new(Vec::<String>::new());
        let note = |l: &str| notes.lock().unwrap().push(l.to_owned());
        // The first lease starts its own, and a spare is started the same way beside it.
        leases.on(open_frame(), &note).await;
        let two = started(dir.path(), 2).await;
        let spare = spare_pid(&leases);
        assert!(two.contains(&spare) && two.contains(&pid(&leases.sessions["vl_1"])));
        leases.on(close("vl_1"), &note).await;
        // The next call is answered by that spare: nothing was started for it.
        leases.on(open_as("vl_2", "haiku"), &note).await;
        assert_eq!(pid(&leases.sessions["vl_2"]), spare);
        leases.on(turn_on("vl_2", 1, "hi"), &note).await;
        let got = until_done_on(&mut rx, "vl_2", 1).await;
        let text: String = got.iter().map(|p| p.text.as_str()).collect();
        assert_eq!(text, "Hello there.", "{got:?}");
        // The one start since is the next spare, not the lease's.
        let three = started(dir.path(), 3).await;
        let next = spare_pid(&leases);
        assert!(three.contains(&next) && !two.contains(&next));
        let notes = notes.into_inner().unwrap();
        assert!(
            notes
                .iter()
                .any(|n| n == "voice: lease vl_2 open, on the spare"),
            "{notes:?}"
        );
        assert!(
            notes
                .iter()
                .all(|n| !n.contains("be Io") && !n.contains("Hello") && !n.contains("hi")),
            "{notes:?}"
        );
    }

    #[tokio::test]
    async fn a_lease_for_another_model_starts_its_own_and_keeps_the_spare() {
        let dir = tempfile::tempdir().unwrap();
        let exec = counted(dir.path());
        let (tx, _rx) = mpsc::channel(QUEUE);
        let mut leases = Leases::new(&exec, tx);
        leases.on(open_frame(), &|_: &str| {}).await;
        let two = started(dir.path(), 2).await;
        let spare = spare_pid(&leases);
        leases.on(open_as("vl_2", "sonnet"), &|_: &str| {}).await;
        let three = started(dir.path(), 3).await;
        let s = &leases.sessions["vl_2"];
        assert!(three.contains(&pid(s)) && !two.contains(&pid(s)), "its own");
        assert!(s.args.windows(2).any(|w| w == ["--model", "sonnet"]));
        // The spare was not spent on it, and the next haiku call takes it.
        assert_eq!(spare_pid(&leases), spare);
        leases.on(open_as("vl_3", ""), &|_: &str| {}).await;
        assert_eq!(pid(&leases.sessions["vl_3"]), spare);
    }

    #[tokio::test]
    async fn a_spare_that_ended_or_aged_is_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let exec = counted(dir.path());
        let (tx, _rx) = mpsc::channel(QUEUE);
        let mut leases = Leases::new(&exec, tx);
        let notes = Mutex::new(Vec::<String>::new());
        let note = |l: &str| notes.lock().unwrap().push(l.to_owned());
        leases.on(open_frame(), &note).await;
        started(dir.path(), 2).await;
        leases.on(close("vl_1"), &note).await;
        let dead = spare_pid(&leases);
        // Once its warm-up is answered, the spare ends.
        tokio::time::timeout(Duration::from_secs(5), async {
            while !leases.spare.as_ref().unwrap().turns.answered() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        let killed = std::process::Command::new("kill")
            .args(["-KILL", &dead.to_string()])
            .status()
            .unwrap();
        assert!(killed.success());
        // The listener's next look (a heartbeat here) sees it ended and waits before the
        // next: a Claude that exits once it has answered is not started on every frame.
        tokio::time::timeout(Duration::from_secs(5), async {
            while leases.spare.is_some() {
                leases.on(Frame::Other, &note).await;
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        for _ in 0..5 {
            leases.on(Frame::Other, &note).await;
        }
        assert!(leases.spare.is_none());
        assert!(leases.spare_after.is_some_and(|t| t > Instant::now()));
        started(dir.path(), 2).await;
        // Once the wait is over, the next look replaces it.
        leases.spare_after = None;
        let fresh = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                leases.on(Frame::Other, &note).await;
                if leases.spare.as_ref().is_some_and(|s| pid(s) != dead) {
                    return spare_pid(&leases);
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        assert!(started(dir.path(), 3).await.contains(&fresh));
        assert!(
            notes.lock().unwrap().iter().any(
                |n| n == "voice: the spare session ended (after its warm-up), another in 30 s at the soonest"
            ),
            "{:?}",
            notes.lock().unwrap()
        );
        // A spare older than its bound is replaced too: a call never gets it.
        leases.spare_ttl = Duration::ZERO;
        leases.on(open_as("vl_2", ""), &note).await;
        let five = started(dir.path(), 5).await;
        let call = pid(&leases.sessions["vl_2"]);
        assert!(five.contains(&call) && call != fresh && call != dead);
        leases.spare_ttl = SPARE_TTL;
        let next = spare_pid(&leases);
        leases.on(Frame::Other, &note).await;
        assert_eq!(spare_pid(&leases), next, "a young spare stays");
        assert!(five.contains(&next) && next != call);
    }

    #[tokio::test]
    async fn the_spare_counts_toward_the_lease_cap_and_never_refuses_a_lease() {
        let dir = tempfile::tempdir().unwrap();
        let exec = counted(dir.path());
        let (tx, _rx) = mpsc::channel(QUEUE);
        let mut leases = Leases::new(&exec, tx);
        let held = |l: &Leases<'_>| l.sessions.len() + usize::from(l.spare.is_some());
        // The first lease and its spare, then each next lease takes the spare and another
        // is started.
        for i in 1..MAX_LEASES {
            leases
                .on(open_as(&format!("vl_{i}"), ""), &|_: &str| {})
                .await;
            started(dir.path(), i + 1).await;
            assert!(held(&leases) <= MAX_LEASES);
        }
        // Three leases and the spare: the cap. A fourth for another model is opened, and
        // the spare stops to make room for it.
        assert_eq!(held(&leases), MAX_LEASES);
        leases.on(open_as("vl_x", "sonnet"), &|_: &str| {}).await;
        started(dir.path(), MAX_LEASES + 1).await;
        assert!(leases.sessions.contains_key("vl_x"));
        assert!(leases.spare.is_none());
        assert_eq!(held(&leases), MAX_LEASES);
        // A lease closes: there is room for a spare again, on the newest lease's model.
        leases.on(close("vl_1"), &|_: &str| {}).await;
        started(dir.path(), MAX_LEASES + 2).await;
        assert_eq!(held(&leases), MAX_LEASES);
        let spare = leases.spare.as_ref().unwrap();
        assert!(spare.args.windows(2).any(|w| w == ["--model", "sonnet"]));
        // A lease on that model takes it, and none is started past the cap.
        leases.on(open_as("vl_y", "sonnet"), &|_: &str| {}).await;
        assert!(leases.spare.is_none());
        assert_eq!(held(&leases), MAX_LEASES);
        // A fifth lease is refused, as before the spare.
        leases.on(open_as("vl_z", ""), &|_: &str| {}).await;
        assert!(!leases.sessions.contains_key("vl_z"));
        tokio::time::sleep(Duration::from_millis(200)).await;
        started(dir.path(), MAX_LEASES + 2).await;
    }

    #[tokio::test]
    async fn no_spare_is_started_once_no_lease_came_for_the_window() {
        let dir = tempfile::tempdir().unwrap();
        let exec = counted(dir.path());
        let (tx, _rx) = mpsc::channel(QUEUE);
        let mut leases = Leases::new(&exec, tx);
        let notes = Mutex::new(Vec::<String>::new());
        let note = |l: &str| notes.lock().unwrap().push(l.to_owned());
        leases.on(open_frame(), &note).await;
        started(dir.path(), 2).await;
        leases.on(close("vl_1"), &note).await;
        // No lease for longer than the window: the spare ages out and is not replaced, on a
        // frame or on the listener's tick.
        leases.spare_window = Duration::ZERO;
        leases.spare_ttl = Duration::ZERO;
        leases.on(Frame::Other, &note).await;
        leases.tend(&note).await;
        assert!(leases.spare.is_none());
        tokio::time::sleep(Duration::from_millis(200)).await;
        started(dir.path(), 2).await;
        // The next lease starts cold, and the spare is kept again from there.
        leases.spare_window = SPARE_WINDOW;
        leases.spare_ttl = SPARE_TTL;
        leases.on(open_as("vl_2", ""), &note).await;
        let three = started(dir.path(), 4).await;
        assert!(leases.spare.is_some());
        assert!(three.contains(&spare_pid(&leases)));
        assert!(
            notes
                .lock()
                .unwrap()
                .iter()
                .any(|n| n == "voice: lease vl_2 open"),
            "a cold start: {:?}",
            notes.lock().unwrap()
        );
    }

    /// Counts its starts like [`COUNTED`] and leaves a child in its group (as `claude`
    /// leaves its tools), recording `pid child`, so a test sees whether the group died.
    fn grouped(dir: &std::path::Path) -> ClaudeExec {
        fake_claude(
            dir,
            &format!(
                "sleep 60 </dev/null >/dev/null 2>&1 &\necho \"$$ $!\" >> \"$d/kids.txt\"\n{COUNTED}{ANSWER}done"
            ),
        )
    }

    /// Every pid and child pid `grouped` recorded, once `n` sessions recorded theirs.
    async fn kids(dir: &std::path::Path, n: usize) -> Vec<i32> {
        let read = || std::fs::read_to_string(dir.join("kids.txt")).unwrap_or_default();
        tokio::time::timeout(Duration::from_secs(5), async {
            while read().lines().count() < n {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        read()
            .split_whitespace()
            .map(|p| p.parse().unwrap())
            .collect()
    }

    /// A lease stream on loopback that sends `frames`, then stays open until `end` is sent
    /// (or dropped), as the coordinator's does between frames.
    async fn held_stream(frames: &[Value]) -> (String, tokio::sync::oneshot::Sender<()>) {
        use std::fmt::Write as _;

        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let uri = format!("http://{}", listener.local_addr().unwrap());
        let (end, ended) = tokio::sync::oneshot::channel::<()>();
        let mut body = String::new();
        for f in frames {
            writeln!(body, "data: {f}\n").unwrap();
        }
        tokio::spawn(async move {
            let (mut conn, _) = listener.accept().await.unwrap();
            let mut head = Vec::new();
            let mut buf = [0_u8; 1024];
            while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                let n = conn.read(&mut buf).await.unwrap();
                assert!(n > 0);
                head.extend_from_slice(&buf[..n]);
            }
            let reply = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n{body}"
            );
            conn.write_all(reply.as_bytes()).await.unwrap();
            conn.flush().await.unwrap();
            let _ = ended.await;
        });
        (uri, end)
    }

    fn api_at(uri: &str) -> Api {
        let person = Arc::new(AnyCredential::Token(StaticToken::new(Redacted::new(
            "person-token".to_owned(),
        ))));
        Api::new(uri, person, false).unwrap()
    }

    #[tokio::test]
    async fn the_spare_stops_when_the_stream_ends() {
        let dir = tempfile::tempdir().unwrap();
        let exec = grouped(dir.path());
        let (uri, end) =
            held_stream(&[json!({"open": {"lease_id": "vl_1", "instructions": "be Io"}})]).await;
        let api = api_at(&uri);
        let svc = HttpService::new(&api, "acc_1".into(), None);
        let (stop, stopped) = watch::channel(false);
        // A long wait before the stream is opened again: what is stopped below, the
        // stream's end stopped, not the listener's return.
        let pace = Pace {
            backoff: Duration::from_secs(60),
            rounds: None,
        };
        let check = async {
            // The lease's session and its spare, each with a child in its group.
            let pids = kids(dir.path(), 2).await;
            assert_eq!(pids.len(), 4, "{pids:?}");
            drop(end);
            let mut alive = Vec::new();
            for p in pids {
                if !crate::runner::exec::process_gone(p).await {
                    alive.push(p);
                }
            }
            stop.send(true).unwrap();
            alive
        };
        let (alive, ()) = tokio::time::timeout(Duration::from_secs(30), async {
            tokio::join!(
                check,
                serve(&svc, &exec, "run_1", stopped, pace, &|_: &str| {})
            )
        })
        .await
        .unwrap();
        assert!(
            alive.is_empty(),
            "still running after the stream ended: {alive:?}"
        );
    }

    #[tokio::test]
    async fn an_aged_spare_is_stopped_with_its_group() {
        let dir = tempfile::tempdir().unwrap();
        let exec = grouped(dir.path());
        // A lease opens and closes, which leaves the spare; the stream stays open.
        let (uri, _end) = held_stream(&[
            json!({"open": {"lease_id": "vl_1", "instructions": "be Io"}}),
            json!({"close": {"lease_id": "vl_1"}}),
        ])
        .await;
        let api = api_at(&uri);
        let svc = HttpService::new(&api, "acc_1".into(), None);
        let (stop, stopped) = watch::channel(false);
        let (tx, _rx) = mpsc::channel(QUEUE);
        let mut leases = Leases::new(&exec, tx);
        leases.spare_ttl = Duration::from_secs(2);
        leases.tend_every = Duration::from_millis(100);
        let check = async {
            // The spare ages on the listener's tick and the next replaces it: three started.
            started(dir.path(), 3).await;
            stop.send(true).unwrap();
        };
        tokio::time::timeout(Duration::from_secs(30), async {
            tokio::join!(
                check,
                listen(
                    &svc,
                    leases,
                    "run_1",
                    stopped,
                    Pace::default(),
                    &|_: &str| {}
                )
            )
        })
        .await
        .unwrap();
        // Everything the aged spare left in its group was stopped with it, as were the
        // sessions the listener dropped on its way out.
        for p in kids(dir.path(), 3).await {
            assert!(crate::runner::exec::process_gone(p).await, "{p} still runs");
        }
    }

    fn api(server: &MockServer) -> Api {
        api_at(&server.uri())
    }

    #[tokio::test]
    async fn pieces_are_posted_in_order_coalesced_and_scoped() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/coordinator/voice/vl_1/deltas"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"accepted": true})))
            .mount(&server)
            .await;
        let api = api(&server);
        let svc = HttpService::new(&api, "acc_1".into(), None);
        let (tx, rx) = mpsc::channel(QUEUE);
        let p = |seq, text: &str, done| Piece {
            seq,
            text: text.into(),
            done,
            failed: "",
        };
        for piece in [p(1, "Hi th", false), p(1, "ere.", true), p(2, "Hi", true)] {
            tx.send(("vl_1".into(), piece)).await.unwrap();
        }
        drop(tx);
        post(&svc, rx, &|_: &str| {}).await;
        let sent: Vec<Value> = server
            .received_requests()
            .await
            .unwrap_or_default()
            .iter()
            .map(|r| serde_json::from_slice(&r.body).unwrap())
            .collect();
        assert_eq!(sent.len(), 2, "{sent:?}");
        assert_eq!(
            sent[0],
            json!({"seq": "1", "text": "Hi there.", "done": true, "failed": "", "account_id": "acc_1"})
        );
        assert_eq!(sent[1]["seq"], "2");
    }

    #[tokio::test]
    async fn the_runner_serves_its_own_lease_stream() {
        let dir = tempfile::tempdir().unwrap();
        let exec = fake_claude(dir.path(), "cat > /dev/null");
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/coordinator/runners/run_1/voice/events"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(format!(
                        "data: {}\n\ndata: {}\n\n",
                        json!({"heartbeat": {}}),
                        json!({"open": {"lease_id": "vl_1", "instructions": "be Io"}}),
                    )),
            )
            .expect(1)
            .mount(&server)
            .await;
        let api = api(&server);
        let svc = HttpService::new(&api, "acc_1".into(), None);
        let (_stop, stopped) = watch::channel(false);
        let notes = Mutex::new(Vec::<String>::new());
        let pace = Pace {
            backoff: Duration::from_millis(10),
            rounds: Some(1),
        };
        tokio::time::timeout(
            Duration::from_secs(10),
            serve(&svc, &exec, "run_1", stopped, pace, &|l: &str| {
                notes.lock().unwrap().push(l.to_owned());
            }),
        )
        .await
        .unwrap();
        let notes = notes.into_inner().unwrap();
        assert!(
            notes.iter().any(|n| n == "voice: lease vl_1 open"),
            "{notes:?}"
        );
        let sent = server.received_requests().await.unwrap_or_default();
        assert!(
            sent[0]
                .url
                .query()
                .unwrap_or_default()
                .contains("account_id=acc_1"),
            "{:?}",
            sent[0].url
        );
    }

    #[tokio::test]
    async fn an_older_coordinator_ends_voice_quietly() {
        let dir = tempfile::tempdir().unwrap();
        let exec = fake_claude(dir.path(), "cat > /dev/null");
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(404).set_body_json(json!({"message": "no such route"})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let api = api(&server);
        let svc = HttpService::new(&api, "acc_1".into(), None);
        let (_stop, stopped) = watch::channel(false);
        let notes = Mutex::new(Vec::<String>::new());
        tokio::time::timeout(
            Duration::from_secs(10),
            serve(
                &svc,
                &exec,
                "run_1",
                stopped,
                Pace::default(),
                &|l: &str| notes.lock().unwrap().push(l.to_owned()),
            ),
        )
        .await
        .unwrap();
        assert!(notes.into_inner().unwrap()[0].contains("not served here"));
        assert!(!dir.path().join("args.txt").exists(), "no session started");
    }
}
