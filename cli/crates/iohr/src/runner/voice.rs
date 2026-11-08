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
//! Nothing a person said or Claude wrote is logged or noted: only counts and labels.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

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
/// back to the thread.
const MAX_LEASES: usize = 4;
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

/// The turns written to a session, oldest first, and the newest turn of the lease.
#[derive(Debug, Default)]
struct Turns {
    written: Mutex<VecDeque<u64>>,
    latest: AtomicU64,
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

    /// Whether `seq` is the turn the person is waiting on. Turn 0 is the warm-up.
    fn live(&self, seq: u64) -> bool {
        seq != 0 && seq == self.latest.load(Ordering::Acquire)
    }
}

/// One lease's warm Claude session. Dropping it kills the session's whole process group
/// and stops its reader.
struct Session {
    /// What it was started with, to start it again the same way.
    args: Vec<String>,
    stdin: ChildStdin,
    turns: Arc<Turns>,
    reader: JoinHandle<()>,
    _group: GroupGuard,
    _child: Child,
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

/// Starts a lease's session on `args` (from [`session_args`]).
async fn open(
    exec: &ClaudeExec,
    lease_id: &str,
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
    let reader = tokio::spawn(read(lease_id.to_owned(), stdout, turns.clone(), tx));
    let mut s = Session {
        args: args.to_vec(),
        stdin,
        turns,
        reader,
        _group: group,
        _child: child,
    };
    s.send(0, WARM_UP).await.map_err(|_| "write")?;
    Ok(s)
}

/// Reads a session's stdout, turns it into pieces of the turn it answers, and fails each
/// turn still waiting when the session ends.
async fn read(
    lease_id: String,
    stdout: ChildStdout,
    turns: Arc<Turns>,
    tx: mpsc::Sender<(String, Piece)>,
) {
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
                            .send((lease_id.clone(), piece(seq, text, false, "")))
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
                        .send((lease_id.clone(), piece(seq, text, true, failed)))
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
                .send((lease_id.clone(), piece(seq, String::new(), true, "model")))
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

/// The leases this runner holds, each with its warm session.
struct Leases<'e> {
    exec: &'e ClaudeExec,
    sessions: HashMap<String, Session>,
    tx: mpsc::Sender<(String, Piece)>,
}

impl<'e> Leases<'e> {
    fn new(exec: &'e ClaudeExec, tx: mpsc::Sender<(String, Piece)>) -> Self {
        Self {
            exec,
            sessions: HashMap::new(),
            tx,
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
                    note(&format!("voice: lease {lease_id} closed"));
                }
            }
            Frame::Other => {}
        }
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
        let opened = match session_args(instructions, model) {
            Ok(args) => open(self.exec, &lease_id, &args, self.tx.clone()).await,
            Err(_) => Err("refused"),
        };
        match opened {
            Ok(s) => {
                note(&format!("voice: lease {lease_id} open"));
                self.sessions.insert(lease_id, s);
            }
            Err(why) => note(&format!("voice: lease {lease_id} not opened ({why})")),
        }
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
        // The session died since the last turn: start it again (it starts with no memory
        // of the call) and write the turn there.
        note(&format!(
            "voice: lease {lease_id} session ended, starting again"
        ));
        let args = std::mem::take(&mut s.args);
        let again = if let Ok(mut fresh) = open(self.exec, &lease_id, &args, self.tx.clone()).await
        {
            let wrote = fresh.write(seq, text).await;
            self.sessions.insert(lease_id.clone(), fresh);
            wrote.is_ok()
        } else {
            self.sessions.remove(&lease_id);
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
            Ok(mut events) => loop {
                let next = tokio::select! {
                    _ = stop.changed() => return,
                    n = events.next() => n,
                };
                let Some(Ok(v)) = next else { break };
                leases.on(frame(&v), note).await;
            },
            Err(e) if e.is_missing() || e.is_permanent() => {
                note(&format!(
                    "voice: not served here ({}); turns go on",
                    status_of(&e)
                ));
                return;
            }
            Err(_) => {}
        }
        // The stream ended, and its leases with it on the coordinator (`runner_gone`).
        leases.sessions.clear();
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
    /// `claude -p --include-partial-messages` does). It never echoes what it was told.
    fn fake_claude(dir: &std::path::Path, body: &str) -> ClaudeExec {
        let p = dir.join("claude");
        let script = format!(
            "#!/bin/sh\n[ \"$1\" = --version ] && {{ echo 1; exit 0; }}\nd=$(dirname \"$0\")\nprintf '%s\\n' \"$@\" > \"$d/args.txt\"\nenv > \"$d/env.txt\"\npwd > \"$d/pwd.txt\"\n{body}\n"
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
        Frame::Open {
            lease_id: "vl_1".into(),
            instructions: "be Io".into(),
            model: String::new(),
        }
    }

    fn turn(seq: u64, text: &str) -> Frame {
        Frame::Turn {
            lease_id: "vl_1".into(),
            seq,
            text: text.into(),
        }
    }

    /// Pieces until turn `seq` is done.
    async fn until_done(rx: &mut mpsc::Receiver<(String, Piece)>, seq: u64) -> Vec<Piece> {
        let mut got = Vec::new();
        tokio::time::timeout(Duration::from_secs(10), async {
            while let Some((lease, p)) = rx.recv().await {
                assert_eq!(lease, "vl_1");
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
        leases
            .on(
                Frame::Close {
                    lease_id: "vl_1".into(),
                },
                &note,
            )
            .await;
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
            "echo 1 >> \"$d/starts.txt\"\nread -r w\nread -r a\nexit 1",
        );
        let (tx, mut rx) = mpsc::channel(QUEUE);
        let mut leases = Leases::new(&exec, tx);
        leases.on(open_frame(), &|_: &str| {}).await;
        leases.on(turn(1, "a"), &|_: &str| {}).await;
        let got = until_done(&mut rx, 1).await;
        assert_eq!(got.last().unwrap().failed, "model");
        // The next turn finds the session gone and starts it again.
        leases.on(turn(2, "b"), &|_: &str| {}).await;
        let got = until_done(&mut rx, 2).await;
        assert!(got.last().unwrap().done);
        let starts = std::fs::read_to_string(dir.path().join("starts.txt")).unwrap();
        assert_eq!(starts.lines().count(), 2, "{starts}");
        assert!(leases.sessions.contains_key("vl_1"));
    }

    fn api(server: &MockServer) -> Api {
        let person = Arc::new(AnyCredential::Token(StaticToken::new(Redacted::new(
            "person-token".to_owned(),
        ))));
        Api::new(&server.uri(), person, false).unwrap()
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
        for piece in [p(1, "Hel", false), p(1, "lo.", true), p(2, "Hi", true)] {
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
            json!({"seq": "1", "text": "Hello.", "done": true, "failed": "", "account_id": "acc_1"})
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
