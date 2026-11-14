//! What the panel does to a workload: message it, bring its terminal forward, read its
//! transcript, open its pull request, stop it. Blocking and local; the proxy runs each call
//! off the loop and puts the answer in the panel.
//!
//! A session is the iohr-agents registry's entry (`~/.inorbit/agents/sessions`), so every
//! action goes through the files and processes the Mac app and `iohr-agents` already use:
//! a message is a line in the session's inbox and in the log (the protocol of
//! `iohr-agents send`), stop is the Mac app's "bring it here" (an interrupt, a pause, a
//! second interrupt, never a kill), the terminal is found by its tty.
//!
//! The text of a session and of a message is other people's: control characters are dropped
//! before it is stored or shown.

use std::io::Write as _;
use std::path::Path;
use std::time::{Duration, Instant};

use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use super::workloads::{self, SessionRef};

/// The longest message sent, in characters.
const MESSAGE_MAX: usize = 2000;
/// Pause between the two interrupts of a stop (the Mac app's).
const PAUSE: Duration = Duration::from_millis(600);
/// How long a stop waits for the process to end before it leaves it running.
const PATIENCE: Duration = Duration::from_secs(10);
/// How long a `gh` or `osascript` call may take.
const TOOL_LIMIT: Duration = Duration::from_secs(8);
/// Lines of a transcript shown.
pub(super) const TRANSCRIPT_LINES: usize = 40;

/// The one line of text a message carries: no control characters, trimmed, cut.
pub(super) fn clean_message(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(MESSAGE_MAX)
        .collect()
}

fn session(root: &Path, id: &str) -> Result<SessionRef, String> {
    if !id.starts_with("s:") {
        return Err("only a session on this Mac can do that".into());
    }
    workloads::session_ref(root, id).ok_or_else(|| "that session has ended".to_owned())
}

fn append(path: &Path, line: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(path)?;
    set_private(path);
    writeln!(f, "{line}")
}

#[cfg(unix)]
fn set_private(path: &Path) {
    use std::os::unix::fs::PermissionsExt as _;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

/// A message to the session, as `iohr-agents send` writes it: a line in its inbox (which the
/// inorbit plugin delivers into the session) and in the log the Mac app shows.
pub(super) fn message(
    root: &Path,
    id: &str,
    text: &str,
    now: OffsetDateTime,
) -> Result<String, String> {
    let text = clean_message(text);
    if text.is_empty() {
        return Err("a message cannot be empty".into());
    }
    let s = session(root, id)?;
    if s.pid.is_none_or(|p| !workloads::pid_alive(p)) {
        return Err("that session has ended".into());
    }
    let at = now.format(&Rfc3339).map_err(|e| e.to_string())?;
    let line = serde_json::json!({
        "at": at,
        "from": "iohr panel",
        "to": s.inbox_id,
        "text": text,
    })
    .to_string();
    let agents = root.join("agents");
    append(
        &agents.join("inbox").join(format!("{}.jsonl", s.inbox_id)),
        &line,
    )
    .and_then(|()| append(&agents.join("log.jsonl"), &line))
    .map_err(|e| format!("could not write the message: {e}"))?;
    Ok("message sent".into())
}

/// The last lines said in the session's transcript.
pub(super) fn transcript(root: &Path, id: &str) -> Result<Vec<String>, String> {
    let s = session(root, id)?;
    if s.transcript.as_os_str().is_empty() {
        return Err("this session has no transcript".into());
    }
    let lines = workloads::transcript_lines(&s.transcript, TRANSCRIPT_LINES);
    if lines.is_empty() {
        return Err("the transcript is empty or cannot be read".into());
    }
    Ok(lines)
}

/// The `AppleScript` that brings the terminal with this tty forward, given the tty as its one
/// argument (never spliced into the script). Answers which program it found.
const FOCUS: &str = r#"on run argv
  set want to "/dev/" & item 1 of argv
  tell application "System Events" to set running to name of every application process
  if running contains "iTerm2" then
    tell application "iTerm2"
      repeat with w in windows
        repeat with tb in tabs of w
          repeat with s in sessions of tb
            if tty of s is want then
              tell w to select tb
              tell tb to select s
              activate
              return "iTerm2"
            end if
          end repeat
        end repeat
      end repeat
    end tell
  end if
  if running contains "Terminal" then
    tell application "Terminal"
      repeat with w in windows
        repeat with tb in tabs of w
          if tty of tb is want then
            set selected of tb to true
            set index of w to 1
            activate
            return "Terminal"
          end if
        end repeat
      end repeat
    end tell
  end if
  return "none"
end run"#;

/// A tty name as `ps` writes it: letters and digits only, so it is safe as an argument.
fn tty_ok(tty: &str) -> bool {
    !tty.is_empty() && tty.len() <= 16 && tty.chars().all(|c| c.is_ascii_alphanumeric())
}

/// Runs a command with a deadline; its stdout when it exits 0.
fn run_limited(mut cmd: std::process::Command, dir: Option<&Path>) -> Result<String, String> {
    use std::io::Read as _;
    use std::process::Stdio;
    if let Some(d) = dir {
        cmd.current_dir(d);
    }
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("cannot run {}: {e}", cmd.get_program().to_string_lossy()))?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut out = String::new();
                if let Some(mut so) = child.stdout.take() {
                    let _ = so.read_to_string(&mut out);
                }
                return if status.success() {
                    Ok(out)
                } else {
                    Err(format!("{} failed", cmd.get_program().to_string_lossy()))
                };
            }
            Ok(None) if start.elapsed() < TOOL_LIMIT => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "{} took too long",
                    cmd.get_program().to_string_lossy()
                ));
            }
            Err(e) => return Err(e.to_string()),
        }
    }
}

/// Brings the session's terminal forward (Terminal.app or iTerm2, by its tty).
pub(super) fn terminal(root: &Path, id: &str) -> Result<String, String> {
    let s = session(root, id)?;
    if !cfg!(target_os = "macos") {
        return Err("bringing a terminal forward works on macOS only".into());
    }
    if !tty_ok(&s.tty) {
        return Err("this session has no terminal (it runs in the background)".into());
    }
    let mut cmd = std::process::Command::new("osascript");
    cmd.args(["-e", FOCUS, &s.tty]);
    let found = run_limited(cmd, None)?;
    match found.trim() {
        "none" | "" => Err("no Terminal or iTerm2 window holds it".into()),
        app => Ok(format!("{app} is forward")),
    }
}

/// `https://github.com/<owner>/<repo>/pull/<n>` for `owner/repo#n`; `None` for anything else.
pub(super) fn pr_url(pr: &str) -> Option<String> {
    let (repo, n) = pr.rsplit_once('#')?;
    let (owner, name) = repo.split_once('/')?;
    let part = |s: &str| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    (part(owner) && part(name) && !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
        .then(|| format!("https://github.com/{owner}/{name}/pull/{n}"))
}

/// Only a GitHub pull request URL is ever opened.
fn pr_page(url: &str) -> Option<&str> {
    let rest = url.trim().strip_prefix("https://github.com/")?;
    (rest.contains("/pull/") && rest.chars().all(|c| c.is_ascii_graphic())).then_some(url.trim())
}

/// Opens the pull request in the browser: `known` is the workload's own (`owner/repo#n`, a
/// job's), else the session's branch is asked of `gh` in its folder.
pub(super) fn pr(root: &Path, id: &str, known: Option<&str>) -> Result<String, String> {
    let url = if let Some(u) = known.and_then(pr_url) {
        u
    } else {
        if !id.starts_with("s:") {
            return Err("no pull request is known for this".into());
        }
        let s = session(root, id)?;
        if s.cwd.is_empty() || !Path::new(&s.cwd).is_dir() {
            return Err("this session's folder is gone".into());
        }
        let mut cmd = std::process::Command::new("gh");
        cmd.args(["pr", "view", "--json", "url", "-q", ".url"])
            .env("GH_PROMPT_DISABLED", "1");
        let out = run_limited(cmd, Some(Path::new(&s.cwd)))
            .map_err(|_| format!("no pull request found for {}", branch_word(&s)))?;
        pr_page(&out)
            .map(str::to_owned)
            .ok_or_else(|| format!("no pull request found for {}", branch_word(&s)))?
    };
    if inorbithr::login::browser::open(&url) {
        Ok(format!("opened {url}"))
    } else {
        Ok(format!("no browser here: {url}"))
    }
}

fn branch_word(s: &SessionRef) -> String {
    if s.branch.is_empty() {
        "this folder".into()
    } else {
        format!("branch {}", s.branch)
    }
}

/// A process as `ps` shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Proc {
    pub(super) tty: String,
    pub(super) command: String,
}

/// The process `pid` now, from `ps`; `None` when it does not run.
pub(super) fn probe(pid: u32) -> Option<Proc> {
    let out = std::process::Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "tty=,command="])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    let line = String::from_utf8_lossy(&out.stdout);
    let line = line.lines().next()?.trim();
    let (tty, command) = line.split_once(char::is_whitespace)?;
    Some(Proc {
        tty: tty.trim().to_owned(),
        command: command.trim().to_owned(),
    })
}

/// Whether a command line is an agent (claude or codex) the panel may interrupt.
pub(super) fn is_agent(command: &str) -> bool {
    command
        .split_whitespace()
        .take(3)
        .filter_map(|w| w.rsplit('/').next())
        .any(|exe| matches!(exe, "claude" | "codex"))
}

/// Interrupts one process by pid (SIGINT, what Ctrl+C sends).
fn interrupt(pid: u32) {
    if let Some(p) = i32::try_from(pid)
        .ok()
        .and_then(rustix::process::Pid::from_raw)
    {
        let _ = rustix::process::kill_process(p, rustix::process::Signal::INT);
    }
}

/// The Mac app's "bring it here", for the panel's stop: the process is the registry's, still
/// the agent it was when read (a reused pid is never signalled) and on the registry's
/// terminal; one interrupt, a pause, a second (the agents exit on two), never a kill; then
/// it waits up to `patience` and leaves it running if it has not ended. `probe`, `signal` and
/// the times are parameters so a test needs no process.
pub(super) fn stop_with(
    s: &SessionRef,
    probe: &dyn Fn(u32) -> Option<Proc>,
    signal: &dyn Fn(u32),
    pause: Duration,
    patience: Duration,
) -> Result<String, String> {
    let pid = s.pid.filter(|p| *p > 1 && *p != std::process::id());
    let Some(pid) = pid else {
        return Err("that session has ended".into());
    };
    let Some(before) = probe(pid) else {
        return Err("that session has ended".into());
    };
    if !is_agent(&before.command) {
        return Err("that pid is no longer an agent; nothing was stopped".into());
    }
    if !s.tty.is_empty() && before.tty != s.tty {
        return Err("that session moved to another terminal; nothing was stopped".into());
    }
    let same = || probe(pid).is_some_and(|p| p == before);
    if same() {
        signal(pid);
    }
    std::thread::sleep(pause);
    if same() {
        signal(pid);
    }
    let start = Instant::now();
    while start.elapsed() < patience {
        if !same() {
            return Ok("stopped".into());
        }
        std::thread::sleep(Duration::from_millis(100).min(patience));
    }
    Ok("asked it to stop; it is still running".into())
}

/// Stops the session `id` (see [`stop_with`]).
pub(super) fn stop(root: &Path, id: &str) -> Result<String, String> {
    let s = session(root, id)?;
    stop_with(&s, &probe, &interrupt, PAUSE, PATIENCE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    fn registry(dir: &Path, id: &str, pid: u32, extra: &serde_json::Value) {
        let d = dir.join("agents/sessions");
        std::fs::create_dir_all(&d).unwrap();
        let mut o = serde_json::json!({
            "id": id, "provider": "claude", "pid": pid.to_string(), "tty": "ttys003",
            "cwd": "/nowhere", "branch": "feat/x", "name": "Fix sdk",
        });
        for (k, v) in extra.as_object().unwrap() {
            o[k] = v.clone();
        }
        std::fs::write(d.join(format!("{id}.json")), o.to_string()).unwrap();
    }

    fn now() -> OffsetDateTime {
        OffsetDateTime::parse("2026-11-08T10:00:00Z", &Rfc3339).unwrap()
    }

    #[test]
    fn a_message_is_one_clean_line_in_the_inbox_and_the_log() {
        let dir = tempfile::tempdir().unwrap();
        // This test process is the live pid.
        registry(
            dir.path(),
            "inorbit-1a2b",
            std::process::id(),
            &serde_json::json!({}),
        );
        let r = message(
            dir.path(),
            "s:inorbit-1a2b",
            "  rebase\x1b[31m on\nmain\t please ",
            now(),
        );
        assert_eq!(r, Ok("message sent".into()));
        let inbox =
            std::fs::read_to_string(dir.path().join("agents/inbox/inorbit-1a2b.jsonl")).unwrap();
        let log = std::fs::read_to_string(dir.path().join("agents/log.jsonl")).unwrap();
        assert_eq!(inbox, log);
        let v: serde_json::Value = serde_json::from_str(inbox.trim()).unwrap();
        assert_eq!(v["to"], "inorbit-1a2b");
        assert_eq!(v["from"], "iohr panel");
        assert_eq!(v["at"], "2026-11-08T10:00:00Z");
        assert_eq!(v["text"], "rebase [31m on main please");
        assert!(!inbox.contains('\x1b'));
    }

    #[test]
    fn a_message_to_nothing_a_dead_session_or_a_job_is_refused_and_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        registry(dir.path(), "gone", 4_000_000, &serde_json::json!({}));
        assert!(
            message(dir.path(), "s:gone", "hi", now())
                .unwrap_err()
                .contains("ended")
        );
        assert!(
            message(dir.path(), "s:nope", "hi", now())
                .unwrap_err()
                .contains("ended")
        );
        assert!(
            message(dir.path(), "j:job_1", "hi", now())
                .unwrap_err()
                .contains("session")
        );
        assert!(
            message(dir.path(), "s:gone", " \x07 ", now())
                .unwrap_err()
                .contains("empty")
        );
        assert!(!dir.path().join("agents/inbox").exists());
    }

    #[test]
    fn the_transcript_is_the_last_lines_said() {
        let dir = tempfile::tempdir().unwrap();
        let t = dir.path().join("t.jsonl");
        let line = |role: &str, text: &str| {
            serde_json::json!({"message": {"role": role, "content": [{"type": "text", "text": text}]}}).to_string()
        };
        std::fs::write(
            &t,
            [
                line("user", "fix the build"),
                "not json".to_owned(),
                line("assistant", "Looking.\n\nFound it in cli.rs"),
                serde_json::json!({"message": {"role": "user", "content": "plain string"}})
                    .to_string(),
            ]
            .join("\n"),
        )
        .unwrap();
        registry(
            dir.path(),
            "s1",
            1,
            &serde_json::json!({"transcript": t.to_str().unwrap()}),
        );
        assert_eq!(
            transcript(dir.path(), "s:s1").unwrap(),
            [
                "you: fix the build",
                "agent: Looking.",
                "agent: Found it in cli.rs",
                "you: plain string"
            ]
        );
        registry(dir.path(), "s2", 1, &serde_json::json!({"transcript": ""}));
        assert!(
            transcript(dir.path(), "s:s2")
                .unwrap_err()
                .contains("no transcript")
        );
        registry(
            dir.path(),
            "s3",
            1,
            &serde_json::json!({"transcript": "/nowhere/t.jsonl"}),
        );
        assert!(
            transcript(dir.path(), "s:s3")
                .unwrap_err()
                .contains("empty or cannot")
        );
    }

    #[test]
    fn a_pull_request_url_is_built_only_from_a_well_formed_reference() {
        assert_eq!(
            pr_url("inorbithr/sdk#448").as_deref(),
            Some("https://github.com/inorbithr/sdk/pull/448")
        );
        for bad in [
            "sdk#448", "a/b#", "a/b#4x", "a/b c#1", "a/b/c#1", "#1", "a/b",
        ] {
            assert_eq!(pr_url(bad), None, "{bad}");
        }
        assert!(pr_page("https://github.com/inorbithr/sdk/pull/448\n").is_some());
        assert!(pr_page("https://evil.example/inorbithr/sdk/pull/1").is_none());
        assert!(pr_page("https://github.com/inorbithr/sdk").is_none());
    }

    #[test]
    fn only_a_claude_or_codex_command_is_an_agent() {
        assert!(is_agent("/Users/n/.local/bin/claude --resume x"));
        assert!(is_agent("node /opt/codex exec"));
        assert!(!is_agent("/bin/zsh -l"));
        assert!(!is_agent("vim claude.md"));
    }

    fn sref(pid: Option<u32>) -> SessionRef {
        SessionRef {
            inbox_id: "a".into(),
            pid,
            tty: "ttys003".into(),
            cwd: String::new(),
            branch: String::new(),
            transcript: std::path::PathBuf::new(),
        }
    }

    fn agent() -> Proc {
        Proc {
            tty: "ttys003".into(),
            command: "claude --x".into(),
        }
    }

    #[test]
    fn stop_interrupts_twice_never_kills_and_reports_an_ended_process() {
        let alive = Cell::new(true);
        let signals = RefCell::new(Vec::new());
        let probe = |_| alive.get().then(agent);
        let signal = |pid: u32| {
            signals.borrow_mut().push(pid);
            if signals.borrow().len() == 2 {
                alive.set(false);
            }
        };
        let r = stop_with(
            &sref(Some(42)),
            &probe,
            &signal,
            Duration::ZERO,
            Duration::from_secs(2),
        );
        assert_eq!(r, Ok("stopped".into()));
        assert_eq!(*signals.borrow(), [42, 42]);
    }

    #[test]
    fn stop_leaves_a_process_that_does_not_end_and_says_so() {
        let n = Cell::new(0);
        let r = stop_with(
            &sref(Some(42)),
            &|_| Some(agent()),
            &|_| n.set(n.get() + 1),
            Duration::ZERO,
            Duration::from_millis(250),
        );
        assert_eq!(n.get(), 2, "two interrupts, no more");
        assert_eq!(r, Ok("asked it to stop; it is still running".into()));
    }

    #[test]
    fn stop_refuses_a_reused_pid_another_terminal_and_pids_that_are_not_a_session() {
        let n = Cell::new(0);
        let signal = |_| n.set(n.get() + 1);
        let go = |s: &SessionRef, p: Option<Proc>| {
            stop_with(s, &|_| p.clone(), &signal, Duration::ZERO, Duration::ZERO)
        };
        let shell = Proc {
            tty: "ttys003".into(),
            command: "/bin/zsh".into(),
        };
        assert!(
            go(&sref(Some(42)), Some(shell))
                .unwrap_err()
                .contains("no longer an agent")
        );
        let other = Proc {
            tty: "ttys009".into(),
            command: "claude".into(),
        };
        assert!(
            go(&sref(Some(42)), Some(other))
                .unwrap_err()
                .contains("another terminal")
        );
        assert!(go(&sref(Some(42)), None).unwrap_err().contains("ended"));
        assert!(
            go(&sref(None), Some(agent()))
                .unwrap_err()
                .contains("ended")
        );
        assert!(
            go(&sref(Some(1)), Some(agent()))
                .unwrap_err()
                .contains("ended")
        );
        assert!(
            go(&sref(Some(std::process::id())), Some(agent()))
                .unwrap_err()
                .contains("ended")
        );
        assert_eq!(n.get(), 0, "nothing was signalled");
    }

    #[test]
    fn a_tty_is_an_argument_only_when_plain() {
        assert!(tty_ok("ttys003"));
        for bad in [
            "",
            "ttys 3",
            "../x",
            "tty\"; do shell script",
            "ttys0030000000000000",
        ] {
            assert!(!tty_ok(bad), "{bad}");
        }
    }
}
