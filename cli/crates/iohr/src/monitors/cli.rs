//! cli checks (RFC 0008): the project's own built binary, run with argv and never a shell, one
//! or more steps in a throwaway folder that is removed after the check, whatever happens.
//!
//! Each check gets its own folder under [`Cli::root`], with its own `home` and `tmp`, and its
//! own copy of each folder it names (iohr: `{board}` and `{monitors}`), so a write verb changes
//! the copy and never the project. The binary sees a scrubbed environment: `HOME` and `TMPDIR`
//! inside the folder, a short `PATH`, no token, and every proxy variable pointing at a closed
//! port unless the check says `network = true`. A drop guard removes the folder on a pass, a
//! failure, a timeout or a panic, and another kills a child still running.
//!
//! Each step: the argv, after the paths are put in, must match `expect.request` (checked
//! before the binary starts); the exit code must be `expect.exit` (0 when left out); stderr
//! must hold no token, key, email or home path; and stdout, read at most 1 MiB, must match
//! `expect.shape`. A failure names a place and two types, or the kind of thing found, never a
//! value.

use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use super::run::{TIMEOUT, Verdict};
use super::shape::{MAX_BODY, Shapes, check_body, first_mismatch};
use super::{Monitor, Shape, Step};

/// The most stderr read from one step.
pub const MAX_STDERR: usize = 64 * 1024;
/// Where every proxy variable points when a check has no network: a port nothing listens on.
const NO_NETWORK: &str = "http://127.0.0.1:9";
const PROXIES: [&str; 7] = [
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "http_proxy",
    "https_proxy",
    "all_proxy",
    "INORBIT_PROXY",
];

/// How to run the cli checks: the binary just built, where the throwaway folders go, and the
/// project folders a check may name in its argv, each copied in when it is named.
#[derive(Debug, Clone)]
pub struct Cli {
    /// The binary `target = { cli = ... }` resolves to.
    pub bin: PathBuf,
    /// The folder the throwaway folders are made in; it is left empty after every check.
    pub root: PathBuf,
    /// A name used as `{name}` in an argv, and the file or folder copied in for it.
    pub copies: Vec<(String, PathBuf)>,
}

/// A throwaway folder, removed when it is dropped, with its parent once that is empty too
/// (a parallel run still holding a folder there keeps the parent).
struct Scratch(PathBuf);

impl Scratch {
    fn new(root: &Path, name: &str) -> io::Result<Scratch> {
        static N: AtomicU64 = AtomicU64::new(0);
        fs::create_dir_all(root)?;
        let path = root.join(format!(
            "{name}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path)?;
        let s = Scratch(path);
        fs::create_dir(s.0.join("home"))?;
        fs::create_dir(s.0.join("tmp"))?;
        Ok(s)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
        if let Some(parent) = self.0.parent() {
            let _ = fs::remove_dir(parent);
        }
    }
}

/// A child process, killed and waited for when it is dropped before it ends.
struct Kid(Child);

impl Drop for Kid {
    fn drop(&mut self) {
        if matches!(self.0.try_wait(), Ok(None)) {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}

impl Cli {
    /// Runs every step of a cli check in one throwaway folder and judges it.
    pub fn run(&self, m: &Monitor, shapes: Option<&dyn Shapes>) -> Verdict {
        let scratch = match Scratch::new(&self.root, &m.name) {
            Ok(s) => s,
            Err(e) => {
                return Verdict::Fail(format!("could not make the throwaway folder: {}", e.kind()));
            }
        };
        let deadline = Instant::now() + m.expect.max_ms.map_or(TIMEOUT, Duration::from_millis);
        for (i, step) in m.steps.iter().enumerate() {
            if let Err(why) = self.step(m, i + 1, step, &scratch.0, deadline, shapes) {
                return Verdict::Fail(why);
            }
        }
        Verdict::Pass
    }

    fn step(
        &self,
        m: &Monitor,
        n: usize,
        step: &Step,
        dir: &Path,
        deadline: Instant,
        shapes: Option<&dyn Shapes>,
    ) -> Result<(), String> {
        let argv = self
            .argv(step, dir)
            .map_err(|e| format!("step {n}: could not copy a project folder in: {}", e.kind()))?;
        if let (Some(path), Some(shapes)) = (&step.expect.request, shapes) {
            let schema = shapes.validator(path)?;
            let value = serde_json::Value::from(argv.clone());
            if let Some(miss) = first_mismatch(&schema, &value) {
                return Err(format!("request step {n} {miss}"));
            }
        }
        let mut cmd = Command::new(&self.bin);
        cmd.args(&argv)
            .current_dir(dir)
            .env_clear()
            .env("HOME", dir.join("home"))
            .env("TMPDIR", dir.join("tmp"))
            .env("PATH", "/usr/bin:/bin")
            .env("LANG", "C")
            .env("NO_COLOR", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if !m.network {
            for k in PROXIES {
                cmd.env(k, NO_NETWORK);
            }
        }
        let mut kid = Kid(cmd
            .spawn()
            .map_err(|e| format!("step {n}: the binary did not start: {}", e.kind()))?);
        let out = kid
            .0
            .stdout
            .take()
            .map(|r| thread::spawn(move || slurp(r, MAX_BODY)));
        let err = kid
            .0
            .stderr
            .take()
            .map(|r| thread::spawn(move || slurp(r, MAX_STDERR)));
        let status = wait(&mut kid.0, deadline)
            .ok_or_else(|| format!("step {n} timed out; the binary was stopped"))?;
        let (stdout, out_over) = out.and_then(|h| h.join().ok()).unwrap_or_default();
        let (stderr, _) = err.and_then(|h| h.join().ok()).unwrap_or_default();

        let want = step.expect.exit.unwrap_or(0);
        if status.code() != Some(want) {
            let found = status
                .code()
                .map_or_else(|| "a signal".to_owned(), |c| c.to_string());
            return Err(format!("step {n} /exit: expected {want}, found {found}"));
        }
        let text = String::from_utf8_lossy(&stderr)
            .replace(&dir.to_string_lossy().into_owned(), "{tmp}")
            .replace(&self.bin.to_string_lossy().into_owned(), "iohr");
        if let Some(kind) = leak(&text) {
            return Err(format!("step {n} stderr holds {kind}"));
        }
        match (&step.expect.shape, shapes) {
            (Some(_), _) if out_over => Err(format!(
                "shape step {n} /: expected at most {MAX_BODY} bytes, found more"
            )),
            (Some(Shape::One(path)), Some(shapes)) => {
                check_body(shapes, path, &stdout).map_err(|e| format!("shape step {n} {e}"))
            }
            _ => Ok(()),
        }
    }

    /// The step's argv with `{tmp}`, `{home}` and each named copy put in; a copy is made the
    /// first time a step names it.
    fn argv(&self, step: &Step, dir: &Path) -> io::Result<Vec<String>> {
        let mut out = Vec::with_capacity(step.argv.len());
        for a in &step.argv {
            let mut a = a
                .replace("{tmp}", &dir.to_string_lossy())
                .replace("{home}", &dir.join("home").to_string_lossy());
            for (name, from) in &self.copies {
                let key = format!("{{{name}}}");
                if a.contains(&key) {
                    let to = dir.join(name);
                    if !to.exists() {
                        copy(from, &to)?;
                    }
                    a = a.replace(&key, &to.to_string_lossy());
                }
            }
            out.push(a);
        }
        Ok(out)
    }
}

/// A file, or a folder with its regular files and subfolders (fixtures live in one), copied to
/// `to`. Symlinks are skipped, so a copy never reaches outside the folder it names.
fn copy(from: &Path, to: &Path) -> io::Result<()> {
    if from.is_file() {
        return fs::copy(from, to).map(drop);
    }
    fs::create_dir(to)?;
    for e in fs::read_dir(from)? {
        let e = e?;
        let kind = e.file_type()?;
        if kind.is_file() {
            fs::copy(e.path(), to.join(e.file_name()))?;
        } else if kind.is_dir() {
            copy(&e.path(), &to.join(e.file_name()))?;
        }
    }
    Ok(())
}

/// Reads everything, keeping at most `limit` bytes, so the child never blocks on a full pipe.
/// True when there was more.
fn slurp(mut r: impl Read, limit: usize) -> (Vec<u8>, bool) {
    let (mut kept, mut over, mut buf) = (Vec::new(), false, [0u8; 8192]);
    while let Ok(n) = r.read(&mut buf) {
        if n == 0 {
            break;
        }
        let room = limit.saturating_sub(kept.len());
        kept.extend_from_slice(&buf[..n.min(room)]);
        over |= n > room;
    }
    (kept, over)
}

/// The child's exit, or None when the deadline passed first (the guard then stops it).
fn wait(child: &mut Child, deadline: Instant) -> Option<ExitStatus> {
    loop {
        match child.try_wait() {
            Ok(Some(s)) => return Some(s),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(5)),
            _ => return None,
        }
    }
}

/// The kind of secret or personal value in `text`, if any: never the value itself.
pub fn leak(text: &str) -> Option<&'static str> {
    const TOKENS: [&str; 12] = [
        "ghp_",
        "gho_",
        "ghs_",
        "ghu_",
        "github_pat_",
        "sk-",
        "xoxb-",
        "xoxp-",
        "AKIA",
        "iohr_",
        "eyJ",
        "glpat-",
    ];
    if text.contains("-----BEGIN") {
        return Some("a private key");
    }
    let tokenish = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '-';
    let starts_word = |at: usize| text[..at].chars().next_back().is_none_or(|c| !tokenish(c));
    for t in TOKENS {
        for (at, _) in text.match_indices(t) {
            let tail = text[at + t.len()..]
                .chars()
                .take_while(|&c| tokenish(c))
                .count();
            if starts_word(at) && tail >= 12 {
                return Some("a token");
            }
        }
    }
    let lower = text.to_ascii_lowercase();
    for k in ["bearer ", "token=", "password=", "secret=", "api_key="] {
        for (at, _) in lower.match_indices(k) {
            if lower[at + k.len()..]
                .chars()
                .take_while(|&c| tokenish(c))
                .count()
                >= 8
            {
                return Some("a secret");
            }
        }
    }
    for (at, _) in text.match_indices('@') {
        let user = text[..at]
            .chars()
            .rev()
            .take_while(|&c| c.is_ascii_alphanumeric() || "._%+-".contains(c))
            .count();
        let host: String = text[at + 1..]
            .chars()
            .take_while(|&c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
            .collect();
        if user > 0 && host.trim_end_matches('.').contains('.') {
            return Some("an email");
        }
    }
    for home in ["/Users/", "/home/"] {
        if text
            .match_indices(home)
            .any(|(at, _)| text[at + home.len()..].starts_with(|c: char| c.is_ascii_alphanumeric()))
        {
            return Some("a home path");
        }
    }
    None
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn a_leak_names_its_kind_and_plain_words_pass() {
        assert_eq!(leak("no card task-1 on the board; disk-free"), None);
        assert_eq!(leak("error: unexpected argument '--nope' found"), None);
        // Built at run time so no token-shaped literal sits in the source (gitleaks).
        assert_eq!(
            leak(&format!("auth: {}_{}", "ghp", "0123456789abcdefABCDEF")),
            Some("a token")
        );
        assert_eq!(
            leak("Authorization: Bearer abcdefgh12345678"),
            Some("a secret")
        );
        assert_eq!(leak("mail me at someone@example.org"), Some("an email"));
        assert_eq!(leak("read /Users/someone/x"), Some("a home path"));
        assert_eq!(
            leak("-----BEGIN OPENSSH PRIVATE KEY-----"),
            Some("a private key")
        );
        assert_eq!(leak("in {tmp}/board/board.json"), None);
    }

    #[test]
    fn slurp_keeps_the_limit_and_says_when_there_was_more() {
        assert_eq!(slurp(&b"abcdef"[..], 4), (b"abcd".to_vec(), true));
        assert_eq!(slurp(&b"ab"[..], 4), (b"ab".to_vec(), false));
    }

    #[test]
    fn the_scratch_folder_is_gone_after_a_panic() {
        let tmp = tempfile::Builder::new()
            .prefix("eye-monitors-scratch-")
            .tempdir()
            .unwrap();
        let root = tmp.path().join("scratch");
        let r = std::panic::catch_unwind(|| {
            let s = Scratch::new(&root, "boom").unwrap();
            fs::write(s.0.join("tmp/x"), "x").unwrap();
            panic!("on purpose");
        });
        assert!(r.is_err());
        assert!(
            !root.exists(),
            "the scratch folder or its empty parent is left behind"
        );
    }
}
