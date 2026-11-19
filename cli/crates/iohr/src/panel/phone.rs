//! The panel's link to the Mac app's terminal host (`inorbit-termd`, RFC 0193), so the panel's
//! shell can be shown on the phone and, when the person allows it, typed into from there.
//!
//! The panel does not serve the phone itself. `ServeTerminals` is one stream per person and a
//! later stream replaces the earlier one, so a second server here would knock the Mac app's
//! share off. Instead the panel hands its shell to the host as an *external session* (host
//! protocol 2): it `adopt`s a session, sends what the shell prints as `output`, and `offer`s
//! "show it" and "let the phone type". The app's own relay, pairing and native questions do the
//! rest and answer with `flags`; the phone's keys come back as `write`. No crypto lives here.
//!
//! States of the link: `Connecting` (hello, adopt, the first screen) -> `Looking` (an app is not
//! known yet: the app's watcher sends `flags` every few seconds, so the first one proves it
//! runs; none within [`LOOK`] means it does not) -> `Linked` (flags flow, writes may arrive) ->
//! `Lost` (the host closed the link, an upgrade included). A lost link is final for this
//! session: the person turns the option on again.
//!
//! Typing has two locks, both must be open: the person asked for it in the panel
//! (`Cmd::Typing`), and the Mac app said so in `flags` after its own question. A `write` that
//! arrives while either is shut is dropped ([`may_type`]). The desktop's own rules for what a
//! phone may type (`input.rs`: idle, Claude in front, one clean line, rate limits) apply there,
//! before a `write` is ever sent.
//!
//! Generic over the stream, so the tests run it over an in-memory pipe.

use std::path::{Path, PathBuf};
use std::time::Duration;

use base64::Engine;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::model::Told;

/// The host's protocol that has external sessions.
const NEEDS: u32 = 2;
/// How long the host may take to answer hello and adopt.
const ANSWER: Duration = Duration::from_secs(3);
/// How long to wait for the app's first `flags` before saying it is not running.
pub(super) const LOOK: Duration = Duration::from_secs(6);

pub(super) const NO_APP: &str = "The InOrbit Mac app is not running. Open it, then try again.";
pub(super) const OLD_APP: &str =
    "The InOrbit Mac app is too old to show a terminal on the phone. Update it first.";
pub(super) const LOST: &str = "The link to the Mac app was lost. Turn Share to phone on again.";

/// What the panel tells the host. The host's own names, a copy of the request shapes it reads.
#[derive(Serialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum Req<'a> {
    Hello,
    Adopt {
        session: &'a str,
        cwd: &'a str,
        cols: u16,
        rows: u16,
        pid: u32,
    },
    Output {
        session: &'a str,
        data: String,
    },
    Resize {
        session: &'a str,
        cols: u16,
        rows: u16,
    },
    Offer {
        session: &'a str,
        share: bool,
        typing: bool,
    },
}

#[derive(Deserialize)]
struct Refused {
    what: String,
    why: String,
}

/// What the host tells the panel; anything else it may say later is ignored.
#[derive(Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
enum Ev {
    Hello {
        protocol: u32,
    },
    Ok,
    Error {
        message: String,
    },
    Write {
        data: String,
    },
    Flags {
        shared: bool,
        typing: bool,
        #[serde(default)]
        refused: Option<Refused>,
    },
    #[serde(other)]
    Other,
}

/// What the panel gives the link to start with.
#[derive(Debug, Clone)]
pub(super) struct Start {
    pub(super) session: String,
    pub(super) cwd: String,
    pub(super) cols: u16,
    pub(super) rows: u16,
    /// The shell's process id (the app reads whether Claude runs in it from it).
    pub(super) pid: u32,
    /// The screen as it is now, so the phone starts from it.
    pub(super) screen: Vec<u8>,
    /// Whether the person already turned typing on.
    pub(super) typing: bool,
}

/// What the panel sends while linked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Cmd {
    /// What the shell printed (already free of the panel's own drawing).
    Output(Vec<u8>),
    Resize(u16, u16),
    /// The person turned typing on or off in the panel.
    Typing(bool),
}

/// What the link says back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Report {
    Told(Told),
    /// Bytes the phone typed, allowed by both locks: write them to the shell.
    Write(Vec<u8>),
}

/// Both locks: the person asked in the panel, and the Mac app said yes.
pub(super) fn may_type(asked_here: bool, allowed_by_app: bool) -> bool {
    asked_here && allowed_by_app
}

/// A running link. Dropping it ends the link (the host then ends the session and the phone's
/// view of it).
pub(super) struct Phone {
    cmds: mpsc::UnboundedSender<Cmd>,
    pub(super) reports: mpsc::UnboundedReceiver<Report>,
    task: JoinHandle<()>,
}

impl Phone {
    /// Opens the link to the host in `dir` (`~/.inorbit/term`) in the background. What it finds
    /// arrives as reports: the app answering, the Mac's decisions, one `Lost` with the reason.
    pub(super) fn start(dir: PathBuf, start: Start, look: Duration) -> Self {
        let (cmds, cmd_rx) = mpsc::unbounded_channel();
        let (report_tx, reports) = mpsc::unbounded_channel();
        let task = tokio::spawn(async move {
            let lost = match open(&dir).await {
                Ok(stream) => serve(stream, start, look, cmd_rx, &report_tx).await,
                Err(why) => why,
            };
            let _ = report_tx.send(Report::Told(Told::Lost(lost)));
        });
        Self {
            cmds,
            reports,
            task,
        }
    }

    pub(super) fn send(&self, cmd: Cmd) {
        // A closed channel means the link ended; its `Lost` report says so.
        let _ = self.cmds.send(cmd);
    }
}

impl Drop for Phone {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// The host's socket: the usual one, else the second one a newer host serves on when the first
/// runs an older build. Says in one line why there is none.
async fn open(dir: &Path) -> Result<tokio::net::UnixStream, String> {
    let mut old = false;
    for name in ["termd.sock", "termd-v2.sock"] {
        let Ok(mut stream) = tokio::net::UnixStream::connect(dir.join(name)).await else {
            continue;
        };
        match hello(&mut stream).await {
            Ok(protocol) if protocol >= NEEDS => return Ok(stream),
            Ok(_) => old = true,
            Err(_) => {}
        }
    }
    Err(if old { OLD_APP } else { NO_APP }.to_owned())
}

/// Asks a host which protocol it speaks (one line out, one line in), on its own connection, so
/// an old host's answer to anything else never reaches the session's stream.
async fn hello<S: AsyncRead + AsyncWrite + Unpin>(stream: &mut S) -> Result<u32, String> {
    send(stream, &Req::Hello).await?;
    let mut line = Vec::new();
    // Read byte by byte: nothing after the answer may be swallowed by a buffer, the same stream
    // goes on to the session.
    let read = async {
        let mut b = [0u8; 1];
        loop {
            if stream.read(&mut b).await.map_err(|e| e.to_string())? == 0 {
                return Err("closed".to_owned());
            }
            if b[0] == b'\n' {
                return Ok(());
            }
            line.push(b[0]);
        }
    };
    tokio::time::timeout(ANSWER, read)
        .await
        .map_err(|_| "no answer".to_owned())??;
    match serde_json::from_slice::<Ev>(&line) {
        Ok(Ev::Hello { protocol }) => Ok(protocol),
        // A host that predates hello answers with an error: protocol 0.
        Ok(Ev::Error { .. }) => Ok(0),
        _ => Err("not a host".to_owned()),
    }
}

async fn send<S: AsyncWrite + Unpin>(stream: &mut S, req: &Req<'_>) -> Result<(), String> {
    let mut line = serde_json::to_string(req).map_err(|e| e.to_string())?;
    line.push('\n');
    stream
        .write_all(line.as_bytes())
        .await
        .map_err(|e| e.to_string())
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// Runs the link on a stream whose hello already passed, until the host closes it. The return is
/// why it ended, in one line.
#[allow(clippy::too_many_lines, reason = "the link's steps, in order")]
async fn serve<S: AsyncRead + AsyncWrite + Unpin>(
    stream: S,
    start: Start,
    look: Duration,
    mut cmds: mpsc::UnboundedReceiver<Cmd>,
    reports: &mpsc::UnboundedSender<Report>,
) -> String {
    let (read, mut write) = tokio::io::split(stream);
    let mut lines = BufReader::new(read).lines();
    let id = start.session.as_str();
    let adopt = Req::Adopt {
        session: id,
        cwd: &start.cwd,
        cols: start.cols,
        rows: start.rows,
        pid: start.pid,
    };
    if send(&mut write, &adopt).await.is_err() {
        return LOST.to_owned();
    }
    // The host answers adopt with ok or an error; anything before it is not about this.
    let answered = tokio::time::timeout(ANSWER, async {
        loop {
            match lines.next_line().await {
                Ok(Some(l)) => match serde_json::from_str::<Ev>(&l) {
                    Ok(Ev::Ok) => return Ok(()),
                    Ok(Ev::Error { message }) => return Err(message),
                    _ => {}
                },
                _ => return Err(LOST.to_owned()),
            }
        }
    })
    .await;
    match answered {
        Ok(Ok(())) => {}
        Ok(Err(why)) => return plain(&why),
        Err(_) => return NO_APP.to_owned(),
    }
    if !start.screen.is_empty() {
        let out = Req::Output {
            session: id,
            data: b64(&start.screen),
        };
        if send(&mut write, &out).await.is_err() {
            return LOST.to_owned();
        }
    }
    // A probe that asks for nothing: the app's watcher answers every session it sees with flags.
    let probe = Req::Offer {
        session: id,
        share: false,
        typing: false,
    };
    if send(&mut write, &probe).await.is_err() {
        return LOST.to_owned();
    }

    let mut asked_here = start.typing;
    let mut app_allows = false;
    let mut app_seen = false;
    let deadline = tokio::time::sleep(look);
    tokio::pin!(deadline);
    loop {
        // Lines first: what the host says (a write, a refusal) is handled before what the panel
        // has queued, so a decision is never applied after the output it came before.
        tokio::select! {
            biased;
            line = lines.next_line() => {
                let Ok(Some(line)) = line else { return LOST.to_owned() };
                match serde_json::from_str::<Ev>(&line) {
                    Ok(Ev::Flags { shared, typing, refused }) => {
                        if !app_seen {
                            app_seen = true;
                            let _ = reports.send(Report::Told(Told::App));
                            // The app is there: now ask for the share (it asks the person).
                            let offer = Req::Offer { session: id, share: true, typing: asked_here };
                            if send(&mut write, &offer).await.is_err() {
                                return LOST.to_owned();
                            }
                        }
                        app_allows = typing;
                        let refused = refused.map(|r| (r.what, r.why));
                        let _ = reports.send(Report::Told(Told::Flags { shared, typing, refused }));
                    }
                    Ok(Ev::Write { data }) => {
                        // Both locks, every time: a write is dropped unless the person asked in
                        // the panel and the Mac said yes.
                        if may_type(asked_here, app_allows)
                            && let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(data)
                        {
                            let _ = reports.send(Report::Write(bytes));
                        }
                    }
                    _ => {}
                }
            }
            () = &mut deadline, if !app_seen => return NO_APP.to_owned(),
            cmd = cmds.recv() => {
                let Some(cmd) = cmd else { return LOST.to_owned() };
                let sent = match cmd {
                    Cmd::Output(bytes) if bytes.is_empty() => Ok(()),
                    Cmd::Output(bytes) => {
                        send(&mut write, &Req::Output { session: id, data: b64(&bytes) }).await
                    }
                    Cmd::Resize(cols, rows) => {
                        send(&mut write, &Req::Resize { session: id, cols, rows }).await
                    }
                    Cmd::Typing(on) => {
                        asked_here = on;
                        if app_seen {
                            send(&mut write, &Req::Offer { session: id, share: true, typing: on }).await
                        } else {
                            Ok(())
                        }
                    }
                };
                if sent.is_err() {
                    return LOST.to_owned();
                }
            }
        }
    }
}

/// A host's error text as one line a person can read.
fn plain(why: &str) -> String {
    format!("The Mac app refused: {}", why.trim())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use tokio::io::{DuplexStream, ReadHalf, WriteHalf};

    fn start(typing: bool) -> Start {
        Start {
            session: "panel-1".into(),
            cwd: "/work".into(),
            cols: 80,
            rows: 24,
            pid: 4242,
            screen: b"$ ".to_vec(),
            typing,
        }
    }

    /// The host's side of a link: reads the panel's lines, says things back.
    struct Host {
        lines: tokio::io::Lines<BufReader<ReadHalf<DuplexStream>>>,
        write: WriteHalf<DuplexStream>,
    }

    impl Host {
        async fn next(&mut self) -> Value {
            let line = self.lines.next_line().await.unwrap().expect("a line");
            serde_json::from_str(&line).unwrap()
        }

        async fn say(&mut self, v: Value) {
            let mut s = v.to_string();
            s.push('\n');
            self.write.write_all(s.as_bytes()).await.unwrap();
        }

        /// Reads until the panel's offer, skipping nothing: every line is checked by the caller.
        async fn offer(&mut self) -> (bool, bool) {
            let v = self.next().await;
            assert_eq!(v["op"], "offer", "{v}");
            (
                v["share"].as_bool().unwrap(),
                v["typing"].as_bool().unwrap(),
            )
        }
    }

    struct Link {
        host: Host,
        cmds: mpsc::UnboundedSender<Cmd>,
        reports: mpsc::UnboundedReceiver<Report>,
        task: JoinHandle<String>,
    }

    fn link(typing: bool, look: Duration) -> Link {
        let (mine, theirs) = tokio::io::duplex(1 << 16);
        let (read, write) = tokio::io::split(theirs);
        let (cmds, cmd_rx) = mpsc::unbounded_channel();
        let (tx, reports) = mpsc::unbounded_channel();
        let task = tokio::spawn(async move { serve(mine, start(typing), look, cmd_rx, &tx).await });
        Link {
            host: Host {
                lines: BufReader::new(read).lines(),
                write,
            },
            cmds,
            reports,
            task,
        }
    }

    /// Takes the handshake up to the probe, answering adopt ok.
    async fn adopted(l: &mut Link) {
        let v = l.host.next().await;
        assert_eq!(v["op"], "adopt");
        assert_eq!(v["session"], "panel-1");
        assert_eq!(v["pid"], 4242);
        l.host.say(json!({"t": "ok"})).await;
        let v = l.host.next().await;
        assert_eq!(v["op"], "output", "the screen as it is now comes first");
        assert_eq!(v["data"], b64(b"$ "));
        assert_eq!(
            l.host.offer().await,
            (false, false),
            "the probe asks for nothing"
        );
    }

    fn flags(shared: bool, typing: bool) -> Value {
        json!({"t": "flags", "session": "panel-1", "shared": shared, "typing": typing})
    }

    async fn report(l: &mut Link) -> Report {
        l.reports.recv().await.expect("a report")
    }

    #[test]
    fn both_locks_must_be_open_to_type() {
        assert!(may_type(true, true));
        assert!(!may_type(true, false), "the Mac has not said yes");
        assert!(
            !may_type(false, true),
            "the person did not ask in the panel"
        );
        assert!(!may_type(false, false));
    }

    #[tokio::test(start_paused = true)]
    async fn no_flags_in_time_says_the_mac_app_is_not_running() {
        let mut l = link(false, LOOK);
        adopted(&mut l).await;
        // The host is there but no app answers: nothing says flags.
        let why = l.task.await.unwrap();
        assert_eq!(why, NO_APP);
    }

    #[tokio::test(start_paused = true)]
    async fn the_first_flags_prove_the_app_and_only_then_the_share_is_offered() {
        let mut l = link(false, LOOK);
        adopted(&mut l).await;
        l.host.say(flags(false, false)).await;
        assert_eq!(report(&mut l).await, Report::Told(Told::App));
        assert_eq!(l.host.offer().await, (true, false), "now it asks to share");
        assert_eq!(
            report(&mut l).await,
            Report::Told(Told::Flags {
                shared: false,
                typing: false,
                refused: None
            })
        );
        l.host.say(flags(true, false)).await;
        assert_eq!(
            report(&mut l).await,
            Report::Told(Told::Flags {
                shared: true,
                typing: false,
                refused: None
            })
        );
        // The shell's output and size go to the host.
        l.cmds.send(Cmd::Output(b"hi".to_vec())).unwrap();
        l.cmds.send(Cmd::Resize(100, 30)).unwrap();
        let v = l.host.next().await;
        assert_eq!(
            (v["op"].as_str(), v["data"].as_str()),
            (Some("output"), Some(b64(b"hi").as_str()))
        );
        let v = l.host.next().await;
        assert_eq!(
            (v["op"].as_str(), v["cols"].as_u64(), v["rows"].as_u64()),
            (Some("resize"), Some(100), Some(30))
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_refused_share_reaches_the_panel_with_the_reason() {
        let mut l = link(false, LOOK);
        adopted(&mut l).await;
        l.host.say(flags(false, false)).await;
        report(&mut l).await;
        l.host.offer().await;
        report(&mut l).await;
        l.host
            .say(
                json!({"t": "flags", "session": "panel-1", "shared": false, "typing": false,
                "refused": {"what": "share", "why": "Not shared."}}),
            )
            .await;
        assert_eq!(
            report(&mut l).await,
            Report::Told(Told::Flags {
                shared: false,
                typing: false,
                refused: Some(("share".into(), "Not shared.".into()))
            })
        );
    }

    #[tokio::test(start_paused = true)]
    async fn the_phone_types_only_when_the_person_asked_and_the_mac_said_yes() {
        let mut l = link(false, LOOK);
        adopted(&mut l).await;
        l.host.say(flags(true, true)).await;
        report(&mut l).await;
        l.host.offer().await;
        report(&mut l).await;
        let typed =
            |s: &str| json!({"t": "write", "session": "panel-1", "data": b64(s.as_bytes())});

        // The Mac says typing is fine, but the person never asked in the panel: dropped.
        l.host.say(typed("ls\n")).await;
        // The person asks; the offer carries it. Then the same write goes through.
        l.cmds.send(Cmd::Typing(true)).unwrap();
        assert_eq!(l.host.offer().await, (true, true));
        l.host.say(typed("pwd\n")).await;
        assert_eq!(
            report(&mut l).await,
            Report::Write(b"pwd\n".to_vec()),
            "the first write was dropped"
        );
        // The Mac takes it back: dropped again.
        l.host.say(flags(true, false)).await;
        assert!(matches!(
            report(&mut l).await,
            Report::Told(Told::Flags { typing: false, .. })
        ));
        l.host.say(typed("rm -rf x\n")).await;
        // The person turns it off, the Mac says yes again: still dropped.
        l.cmds.send(Cmd::Typing(false)).unwrap();
        assert_eq!(l.host.offer().await, (true, false));
        l.host.say(flags(true, true)).await;
        assert!(matches!(
            report(&mut l).await,
            Report::Told(Told::Flags { typing: true, .. })
        ));
        l.host.say(typed("whoami\n")).await;
        // End the link: the last report is the loss, with no write in between.
        drop(l.host);
        assert_eq!(l.task.await.unwrap(), LOST);
        assert!(
            l.reports.recv().await.is_none(),
            "nothing typed went through"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn typing_asked_before_the_app_answers_is_offered_with_the_share() {
        let mut l = link(true, LOOK);
        adopted(&mut l).await;
        l.host.say(flags(false, false)).await;
        assert_eq!(l.host.offer().await, (true, true));
    }

    #[tokio::test]
    async fn a_closed_link_is_one_plain_line() {
        let mut l = link(false, LOOK);
        adopted(&mut l).await;
        drop(l.host);
        assert_eq!(l.task.await.unwrap(), LOST);
    }

    #[tokio::test]
    async fn the_host_refusing_adopt_says_so() {
        let mut l = link(false, LOOK);
        let v = l.host.next().await;
        assert_eq!(v["op"], "adopt");
        l.host
            .say(json!({"t": "error", "message": "session id taken"}))
            .await;
        assert_eq!(
            l.task.await.unwrap(),
            "The Mac app refused: session id taken"
        );
    }

    #[tokio::test]
    async fn no_socket_means_the_mac_app_is_not_running() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(open(dir.path()).await.err().as_deref(), Some(NO_APP));
        let mut p = Phone::start(dir.path().to_path_buf(), start(false), LOOK);
        match p.reports.recv().await {
            Some(Report::Told(Told::Lost(why))) => assert_eq!(why, NO_APP),
            other => panic!("{other:?}"),
        }
    }

    /// A host on `dir/name` that answers hello with `protocol` and keeps the connection.
    fn host_on(dir: &Path, name: &str, protocol: u32) {
        let listener = tokio::net::UnixListener::bind(dir.join(name)).unwrap();
        tokio::spawn(async move {
            while let Ok((mut s, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut buf = [0u8; 64];
                    let _ = s.read(&mut buf).await;
                    let hello =
                        format!("{{\"t\":\"hello\",\"protocol\":{protocol},\"version\":\"x\"}}\n");
                    let _ = s.write_all(hello.as_bytes()).await;
                    let _ = s.read(&mut buf).await;
                });
            }
        });
    }

    #[tokio::test]
    async fn an_old_host_says_to_update_the_app() {
        let dir = tempfile::tempdir().unwrap();
        host_on(dir.path(), "termd.sock", 1);
        assert_eq!(open(dir.path()).await.err().as_deref(), Some(OLD_APP));
    }

    #[tokio::test]
    async fn the_newer_host_beside_an_old_one_is_used() {
        let dir = tempfile::tempdir().unwrap();
        host_on(dir.path(), "termd.sock", 1);
        host_on(dir.path(), "termd-v2.sock", 2);
        assert!(open(dir.path()).await.is_ok());
    }
}
