//! A Role's turn on an ask task (RFC 0213, S2): the handler the host's pool runs.
//!
//! A turn is `iohr --json ask --headless --role <Role> ...` run as a child of the host, in the
//! folder the question was asked in: the one way Roles answer today (RFC 0211), so a Role in
//! the host reads, is limited and answers exactly as `iohr ask --role` does. The child never
//! routes again (`IOHR_ROUTED_TASK`). Its stdout is the answer's JSON; its stderr is read only
//! for the last line, to say why it failed. Neither is logged.
//!
//! The child leads a process group of its own, so that ending it ends what it started too
//! (a grandchild holding the output pipe would otherwise keep the turn waiting).
//!
//! When the host stops, the child is killed and the turn returns [`pool::INTERRUPTED`]: the
//! pool puts the task back in the queue without spending a try, so the next host answers it.

use std::ffi::OsString;
use std::io::Read;
use std::os::unix::process::CommandExt as _;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use super::pool;
use super::queue::Task;
use super::route::{Payload, Reply};

/// The environment variable that tells an `iohr ask` it is a Role's turn: it answers, it does
/// not fan out again.
pub(crate) const ROUTED_ENV: &str = "IOHR_ROUTED_TASK";

/// The most bytes read from a turn's stdout (the answer's JSON); more is dropped.
const OUT_MAX: u64 = 256 * 1024;
/// The most bytes read from a turn's stderr.
const ERR_MAX: u64 = 16 * 1024;
/// The most characters of a failure reason.
const REASON_MAX: usize = 300;
/// How often a turn looks at its child and at the stop flag.
const TICK: Duration = Duration::from_millis(50);

/// The flags every turn carries, taken from the host's own: which profile, environment and
/// data folder it runs in.
#[derive(Debug, Clone, Default)]
pub(crate) struct Globals(pub(crate) Vec<OsString>);

/// The arguments of one turn, after the global flags.
pub(crate) fn args(role: &str, p: &Payload) -> Vec<OsString> {
    let mut out: Vec<OsString> = ["--json", "ask", "--headless"]
        .iter()
        .map(OsString::from)
        .collect();
    out.push(format!("--role={role}").into());
    out.push("--cwd".into());
    out.push((&p.cwd).into());
    for (flag, value) in [
        ("--mode", &p.mode),
        ("--model", &p.model),
        ("--agent", &p.agent),
        ("--job", &p.job),
    ] {
        if let Some(v) = value {
            out.push(format!("{flag}={v}").into());
        }
    }
    out.push("--".into());
    out.push((&p.question).into());
    out
}

/// What the child printed: its answer, or why there is none.
pub(crate) fn reply_of(stdout: &str) -> Result<Reply, String> {
    let v: serde_json::Value =
        serde_json::from_str(stdout.trim()).map_err(|_| "the Role printed no answer".to_owned())?;
    let text = v
        .get("text")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "the Role printed no answer".to_owned())?;
    Ok(Reply {
        text: text.to_owned(),
        model: v
            .get("model")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        duration_ms: v
            .get("duration_ms")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_default(),
    })
}

/// The last non-empty line of the child's stderr, short, as the reason a turn failed.
pub(crate) fn reason_of(stderr: &str, code: Option<i32>) -> String {
    let last = stderr
        .lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty())
        .unwrap_or("");
    let last: String = last.chars().take(REASON_MAX).collect();
    match (last.is_empty(), code) {
        (false, _) => last,
        (true, Some(c)) => format!("the Role's turn ended with exit {c}"),
        (true, None) => "the Role's turn was ended by a signal".to_owned(),
    }
}

fn drain<R: Read + Send + 'static>(from: Option<R>, max: u64) -> JoinHandle<String> {
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut r) = from {
            let _ = r.by_ref().take(max).read_to_end(&mut buf);
            // Past the cap the rest is read and dropped, so the child never blocks on a pipe.
            let _ = std::io::copy(&mut r, &mut std::io::sink());
        }
        String::from_utf8_lossy(&buf).into_owned()
    })
}

/// Kills the child's whole process group (the child is its leader, see [`run`]).
fn end_group(child: &std::process::Child) {
    use rustix::process::{Pid, Signal, kill_process_group};
    if let Some(pid) = i32::try_from(child.id()).ok().and_then(Pid::from_raw) {
        let _ = kill_process_group(pid, Signal::KILL);
    }
}

/// One Role's turn on `task`. `Ok` is the reply as the task's `result_ref`.
pub(crate) fn run(
    exe: &Path,
    globals: &Globals,
    task: &Task,
    role: &str,
    stop: &AtomicBool,
) -> Result<String, String> {
    let payload = Payload::parse(&task.payload)?;
    let mut cmd = Command::new(exe);
    cmd.args(&globals.0)
        .args(args(role, &payload))
        .env(ROUTED_ENV, "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    // A folder that is gone is not the Role's to make: it answers from where the host is.
    if Path::new(&payload.cwd).is_dir() {
        cmd.current_dir(&payload.cwd);
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("cannot start the Role's turn: {e}"))?;
    let out = drain(child.stdout.take(), OUT_MAX);
    let err = drain(child.stderr.take(), ERR_MAX);
    let status = loop {
        if stop.load(Ordering::SeqCst) {
            end_group(&child);
            let _ = child.kill();
            let _ = child.wait();
            let _ = (out.join(), err.join());
            return Err(pool::INTERRUPTED.to_owned());
        }
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) => std::thread::sleep(TICK),
            Err(e) => return Err(format!("cannot wait for the Role's turn: {e}")),
        }
    };
    let (stdout, stderr) = (
        out.join().unwrap_or_default(),
        err.join().unwrap_or_default(),
    );
    if status.success() {
        reply_of(&stdout).map(|r| r.stored())
    } else {
        Err(reason_of(&stderr, status.code()))
    }
}

/// After a Role's turn: its handoff note in the project the turn ran in (RFC 0213, S2), so
/// its next turn starts from it. A note that cannot be written is said once on stderr and
/// never fails the turn: the answer is already in the queue's reply.
pub(crate) fn keep_notes(host_project: &Path, task: &Task, role: &str, stored_reply: &str) {
    let Ok(payload) = Payload::parse(&task.payload) else {
        return;
    };
    let Ok(reply) = serde_json::from_str::<Reply>(stored_reply) else {
        return;
    };
    let root = crate::memory::find_root(Path::new(&payload.cwd))
        .unwrap_or_else(|| host_project.to_path_buf());
    if let Err(e) = crate::roles::notes::record(&root, role, &payload.question, &reply.text) {
        crate::output::Out::note(&format!("iohr: no handoff note for Role {role}: {e}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(v: &[OsString]) -> Vec<String> {
        v.iter().map(|a| a.to_string_lossy().into_owned()).collect()
    }

    #[test]
    fn a_turn_asks_as_the_role_in_the_folder_and_the_question_comes_last() {
        let mut p = Payload::new("a1", "-why --role x?", "/work");
        p.mode = Some("plan".into());
        p.job = Some("job_1".into());
        assert_eq!(
            text(&args("rust-engineer", &p)),
            [
                "--json",
                "ask",
                "--headless",
                "--role=rust-engineer",
                "--cwd",
                "/work",
                "--mode=plan",
                "--job=job_1",
                "--",
                "-why --role x?",
            ]
        );
    }

    #[test]
    fn the_reply_is_the_text_the_model_and_the_time() {
        let r =
            reply_of(r#"{"text":"hi","model":"haiku","duration_ms":12,"agent":"claude"}"#).unwrap();
        assert_eq!(
            (r.text.as_str(), r.model.as_str(), r.duration_ms),
            ("hi", "haiku", 12)
        );
        assert!(reply_of("").is_err() && reply_of("{}").is_err());
    }

    #[test]
    fn a_failure_says_the_last_line_or_the_exit() {
        assert_eq!(reason_of("a\n\nboom \n", Some(1)), "boom");
        assert_eq!(reason_of("", Some(3)), "the Role's turn ended with exit 3");
        assert!(reason_of("", None).contains("signal"));
        assert_eq!(
            reason_of(&"x".repeat(900), Some(1)).chars().count(),
            REASON_MAX
        );
    }

    fn task(payload: &str) -> Task {
        Task {
            id: 1,
            external_id: Some("ask:a1:r".into()),
            role_hint: Some("r".into()),
            payload: payload.into(),
            attempts: 1,
        }
    }

    #[test]
    fn a_finished_turn_leaves_the_roles_handoff_in_the_folder_it_ran_in() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join(".git")).unwrap();
        let cwd = d.path().to_string_lossy().into_owned();
        let payload =
            serde_json::to_string(&Payload::new("a1", "why does it fail?", &cwd)).unwrap();
        let reply = Reply {
            text: "The lock is held.".into(),
            model: "m".into(),
            duration_ms: 5,
        };
        keep_notes(Path::new("/nowhere"), &task(&payload), "r", &reply.stored());
        let note = crate::roles::notes::read(d.path(), "r");
        assert!(
            note.contains("why does it fail?") && note.contains("The lock is held."),
            "{note}"
        );
        // A reply that is not a reply, or a Role name that is not safe, leaves nothing.
        keep_notes(Path::new("/nowhere"), &task(&payload), "r", "not json");
        keep_notes(
            Path::new("/nowhere"),
            &task(&payload),
            "../x",
            &reply.stored(),
        );
        assert_eq!(
            std::fs::read_dir(d.path().join(".inorbit/handoff"))
                .unwrap()
                .count(),
            1
        );
    }

    fn script(dir: &Path, body: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt as _;
        let path = dir.join("fake-iohr");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn payload_text(cwd: &Path) -> String {
        serde_json::to_string(&Payload::new("a1", "q", &cwd.display().to_string())).unwrap()
    }

    #[test]
    fn a_turn_returns_the_child_answer_and_runs_unrouted_in_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        let exe = script(
            dir.path(),
            r#"printf '{"text":"%s|%s","model":"m","duration_ms":5}' "$IOHR_ROUTED_TASK" "$(pwd -P)""#,
        );
        let stop = AtomicBool::new(false);
        let got = run(
            &exe,
            &Globals::default(),
            &task(&payload_text(dir.path())),
            "r",
            &stop,
        )
        .unwrap();
        let reply: Reply = serde_json::from_str(&got).unwrap();
        let here = std::fs::canonicalize(dir.path()).unwrap();
        assert_eq!(reply.text, format!("1|{}", here.display()));
        assert_eq!(reply.model, "m");
    }

    #[test]
    fn a_failing_turn_says_why_and_a_stop_ends_the_child_and_asks_for_a_requeue() {
        let dir = tempfile::tempdir().unwrap();
        let bad = script(dir.path(), "echo 'no claude' >&2; exit 4");
        let stop = AtomicBool::new(false);
        let t = task(&payload_text(dir.path()));
        assert_eq!(
            run(&bad, &Globals::default(), &t, "r", &stop),
            Err("no claude".to_owned())
        );
        assert!(run(&bad, &Globals::default(), &task("nope"), "r", &stop).is_err());

        let slow = script(dir.path(), "sleep 30");
        let stop = std::sync::Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let started = std::time::Instant::now();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            flag.store(true, Ordering::SeqCst);
        });
        assert_eq!(
            run(&slow, &Globals::default(), &t, "r", &stop),
            Err(pool::INTERRUPTED.to_owned())
        );
        assert!(started.elapsed() < Duration::from_secs(5));
    }
}
