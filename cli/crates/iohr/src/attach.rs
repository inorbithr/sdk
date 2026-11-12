//! Output the person chose to attach to a `?` question (RFC 0203, PRD 0187.1): a plain shell
//! cannot see what the last command printed, so the card says it was not sent and, for a
//! "why did that fail" on a failed command, offers two ways to send it: re-run the command
//! with its output captured (asked first, never on its own, bounded in time and size), or paste
//! it. Whatever is attached is scrubbed of secrets here, before it can be sent.

use std::io::Read as _;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use crate::runner::host_action::redact_output;

/// How long a re-run may take before it is stopped.
pub(crate) const RERUN_LIMIT: Duration = Duration::from_secs(10);

/// What a re-run keeps in memory: the end of what the command printed, where the failure is.
/// More than this is read and dropped, so a command that never stops printing cannot grow
/// this process.
const KEEP: usize = 16 * 1024;

/// The most a paste is read from the terminal.
const PASTE_READ: u64 = 1024 * 1024;

/// Fragments of a question that ask why something went wrong, in lowercase.
const FAILURE_WORDS: [&str; 17] = [
    "fail",
    "error",
    "broke",
    "crash",
    "wrong",
    "panic",
    "exception",
    "denied",
    "refused",
    "not work",
    "n't work",
    "not found",
    "what happened",
    "why did that",
    "why did it",
    "why is it",
    "why does it",
];

/// The last command, when it failed and the question asks why: what the offer needs to
/// re-run it. The command is the person's own and is only ever shown and, after a yes, run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Failed {
    pub(crate) command: String,
    pub(crate) exit_code: i32,
}

/// Whether the question looks like "why did that fail".
pub(crate) fn asks_why_it_failed(question: &str) -> bool {
    let q = question.to_lowercase().replace('\u{2019}', "'");
    FAILURE_WORDS.iter().any(|w| q.contains(w))
}

/// The offer: only for a failure question about a command that exited non-zero, and only while
/// no output is attached yet (once it is, there is nothing more to offer).
pub(crate) fn offer(
    question: &str,
    command: &str,
    exit_code: Option<i32>,
    attached: bool,
) -> Option<Failed> {
    let code = exit_code.filter(|c| *c != 0)?;
    (!attached && !command.trim().is_empty() && asks_why_it_failed(question)).then(|| Failed {
        command: command.to_owned(),
        exit_code: code,
    })
}

/// What a re-run printed.
#[derive(Debug)]
pub(crate) struct Captured {
    pub(crate) text: String,
    /// The command's exit status; none when it was stopped or killed by a signal.
    pub(crate) exit_code: Option<i32>,
    pub(crate) timed_out: bool,
}

#[cfg(unix)]
fn shell(command: &str) -> Command {
    let mut c = Command::new("/bin/sh");
    c.args(["-c", command]);
    c
}

#[cfg(not(unix))]
fn shell(command: &str) -> Command {
    let mut c = Command::new("cmd");
    c.args(["/C", command]);
    c
}

/// Runs `command` once, in `cwd`, with no input, its output and errors in one stream, and
/// stops it after `limit` (its whole process group). Only the end of the output is kept.
///
/// # Errors
///
/// The command could not be started.
pub(crate) fn rerun(command: &str, cwd: &Path, limit: Duration) -> Result<Captured, String> {
    let (mut reader, writer) =
        std::io::pipe().map_err(|e| format!("cannot capture its output: {}", e.kind()))?;
    let again = writer
        .try_clone()
        .map_err(|e| format!("cannot capture its output: {}", e.kind()))?;
    let mut c = shell(command);
    c.current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(again)
        .stderr(writer);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        c.process_group(0);
    }
    let mut child = c
        .spawn()
        .map_err(|e| format!("could not start it: {}", e.kind()))?;
    // The parent's copies of the write end must close, or the reader never sees the end.
    drop(c);
    let tail = Arc::new(Mutex::new(Vec::<u8>::new()));
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let sink = Arc::clone(&tail);
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 {
                break;
            }
            let mut t = sink.lock().unwrap_or_else(PoisonError::into_inner);
            t.extend_from_slice(&buf[..n]);
            if t.len() > KEEP * 2 {
                let cut = t.len() - KEEP;
                t.drain(..cut);
            }
        }
        let _ = done_tx.send(());
    });
    let started = Instant::now();
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) if started.elapsed() >= limit => {
                timed_out = true;
                stop(&mut child);
                break None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(_) => {
                stop(&mut child);
                break None;
            }
        }
    };
    // Something it started may still hold the pipe: wait a moment, then take what there is.
    let _ = done_rx.recv_timeout(Duration::from_millis(300));
    let bytes = tail.lock().unwrap_or_else(PoisonError::into_inner).clone();
    // The end, at most KEEP bytes, from a whole character.
    let bytes = &bytes[bytes.len().saturating_sub(KEEP)..];
    Ok(Captured {
        text: clean(String::from_utf8_lossy(bytes).trim_start_matches('\u{fffd}')),
        exit_code: status.and_then(|s| s.code()),
        timed_out,
    })
}

/// Stops the command and everything it started, then reaps it.
fn stop(child: &mut std::process::Child) {
    #[cfg(unix)]
    crate::runner::exec::kill_group(child.id());
    let _ = child.kill();
    let _ = child.wait();
}

/// Reads what the person pasted, to the end of input (Ctrl-D), at most 1 MiB, keeping the end.
///
/// # Errors
///
/// The input could not be read.
pub(crate) fn paste(input: impl std::io::Read) -> Result<String, String> {
    let mut bytes = Vec::new();
    input
        .take(PASTE_READ)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("cannot read what you pasted: {}", e.kind()))?;
    Ok(clean(&String::from_utf8_lossy(&bytes)))
}

/// Terminal colours, carriage returns and other control characters out; newlines and tabs stay.
fn clean(text: &str) -> String {
    console::strip_ansi_codes(text)
        .chars()
        .filter(|c| matches!(c, '\n' | '\t') || !c.is_control())
        .collect()
}

/// Names whose value is a secret.
const SECRET_NAMES: [&str; 9] = [
    "password",
    "passwd",
    "secret",
    "token",
    "apikey",
    "api_key",
    "api-key",
    "credential",
    "private_key",
];

fn secret_name(name: &str) -> bool {
    let n = name
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_lowercase();
    SECRET_NAMES.iter().any(|s| n.contains(s)) || n == "authorization"
}

/// A value after a secret's name (`password=...`, `"token": "..."`, `Authorization: ...`)
/// replaced, line by line.
fn mask_values(text: &str) -> String {
    text.lines()
        .map(|line| {
            let mut hide_next = false;
            line.split_inclusive(char::is_whitespace)
                .map(|piece| {
                    let word = piece.trim_end_matches(char::is_whitespace);
                    let rest = &piece[word.len()..];
                    if word.is_empty() {
                        return piece.to_owned();
                    }
                    if hide_next {
                        // `Authorization: Bearer TOKEN`: the scheme is not the secret.
                        hide_next = word.eq_ignore_ascii_case("bearer")
                            || word.eq_ignore_ascii_case("basic");
                        return format!("[redacted]{rest}");
                    }
                    if let Some((name, value)) = word.split_once('=')
                        && secret_name(name)
                    {
                        // `authorization=Bearer TOKEN`: the value is the scheme, the
                        // secret is the next word.
                        hide_next = value.eq_ignore_ascii_case("bearer")
                            || value.eq_ignore_ascii_case("basic");
                        return format!("{name}=[redacted]{rest}");
                    }
                    if let Some(name) = word.strip_suffix(':')
                        && secret_name(name)
                    {
                        hide_next = true;
                    }
                    if let Some((name, value)) = word.split_once(':')
                        && !value.is_empty()
                        && !value.starts_with("//")
                        && secret_name(name)
                    {
                        return format!("{name}:[redacted]{rest}");
                    }
                    piece.to_owned()
                })
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Output made safe to send: values after a secret's name, then the runner's redaction (URLs,
/// hosts and ports, IP addresses, GitHub tokens, JWTs, bearer values and long secret-looking
/// words). Applied to every output, whether captured, pasted or read from a file.
pub(crate) fn scrub(text: &str) -> String {
    redact_output(&mask_values(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failure_question_is_told_from_the_rest() {
        for q in [
            "why did that fail",
            "Why did that fail?",
            "what went wrong",
            "why is it failing",
            "it doesn\u{2019}t work, why",
            "what happened",
            "explain this error",
        ] {
            assert!(asks_why_it_failed(q), "{q}");
        }
        for q in ["list the files", "what is a monad", "how do I rebase"] {
            assert!(!asks_why_it_failed(q), "{q}");
        }
    }

    #[test]
    fn the_offer_needs_a_failed_command_a_failure_question_and_no_output_yet() {
        let want = Some(Failed {
            command: "cargo build".into(),
            exit_code: 101,
        });
        assert_eq!(
            offer("why did that fail", "cargo build", Some(101), false),
            want
        );
        assert_eq!(
            offer("why did that fail", "cargo build", Some(0), false),
            None
        );
        assert_eq!(offer("why did that fail", "cargo build", None, false), None);
        assert_eq!(offer("why did that fail", "", Some(1), false), None);
        assert_eq!(
            offer("why did that fail", "cargo build", Some(101), true),
            None
        );
        assert_eq!(
            offer("list the files", "cargo build", Some(101), false),
            None
        );
    }

    #[test]
    fn secrets_are_scrubbed_before_anything_is_sent() {
        let text = "connecting with password=hunter2 to db\n\
            \"token\": \"abc123def\"\n\
            Authorization: Bearer abc.def.ghi\n\
            API_KEY=sk-live-0123456789 failed\n\
            gh auth used ghp_0123456789abcdefghijklmnopqrstuvwxyz1234\n\
            error: could not read https://user:pw@example.com/x\n\
            cannot find module './thing'";
        let out = scrub(text);
        for leaked in [
            "hunter2",
            "abc123def",
            "abc.def.ghi",
            "sk-live-0123456789",
            "ghp_",
            "user:pw",
        ] {
            assert!(!out.contains(leaked), "{leaked} in {out}");
        }
        assert!(out.contains("cannot find module './thing'"), "{out}");
        assert!(out.contains("connecting with"), "{out}");
    }

    #[test]
    fn pasted_text_loses_colours_and_control_characters_but_keeps_lines() {
        let got = paste("\u{1b}[31merror\u{1b}[0m: no\r\nline\ttwo\u{7}\n".as_bytes()).unwrap();
        assert_eq!(got, "error: no\nline\ttwo\n");
    }

    #[cfg(unix)]
    #[test]
    fn a_rerun_captures_both_streams_and_the_exit_status() {
        let dir = std::env::temp_dir();
        let got = rerun(
            "echo out; echo err 1>&2; exit 3",
            &dir,
            Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(got.exit_code, Some(3));
        assert!(!got.timed_out);
        assert!(
            got.text.contains("out") && got.text.contains("err"),
            "{got:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_rerun_is_stopped_at_the_limit_and_keeps_what_it_printed() {
        let dir = std::env::temp_dir();
        let started = Instant::now();
        let got = rerun("echo before; sleep 30", &dir, Duration::from_millis(300)).unwrap();
        assert!(got.timed_out, "{got:?}");
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(got.text.contains("before"), "{got:?}");
    }

    #[cfg(unix)]
    #[test]
    fn a_rerun_that_never_stops_printing_keeps_only_its_end() {
        let dir = std::env::temp_dir();
        let got = rerun(
            "i=0; while :; do echo line-$i-xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx; i=$((i+1)); done",
            &dir,
            Duration::from_millis(400),
        )
        .unwrap();
        assert!(got.timed_out);
        assert!(got.text.len() <= KEEP, "{}", got.text.len());
        assert!(!got.text.contains("line-0-"), "the start is dropped");
    }
}
