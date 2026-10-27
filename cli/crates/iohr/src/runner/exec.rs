//! Running one turn: the user's installed `claude`, started as published, with the
//! scrubbed environment and the arguments of [`claude::build_args`].

use std::ffi::OsString;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

use super::claude::{self, Outcome, Parsed, Session, TurnSpec};
use super::env;
use super::hygiene;
use super::provider::{self, AuthState};
use super::service::{Pack, ProviderState};

/// The most a provider may print; more is cut, and the parse then fails.
const MAX_OUTPUT: usize = 8 * 1024 * 1024;
/// Wall clock when a pack names none.
const DEFAULT_WALL_CLOCK: Duration = Duration::from_mins(30);

/// How a turn ended, before it is a `TurnReport`.
#[derive(Debug, Clone)]
pub(crate) struct Finished {
    /// The outcome's name: a [`Outcome`], or `refused_provider` or `refused_user`.
    pub(crate) outcome: String,
    pub(crate) parsed: Option<Parsed>,
    pub(crate) resumed: bool,
    pub(crate) session_id: String,
    pub(crate) detail: String,
    pub(crate) provider_version: String,
}

impl Finished {
    pub(crate) fn refused(outcome: &str, detail: impl Into<String>) -> Self {
        Self {
            outcome: outcome.into(),
            parsed: None,
            resumed: false,
            session_id: String::new(),
            detail: detail.into(),
            provider_version: String::new(),
        }
    }
}

type Boxed<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// What the loop needs from the thing that runs turns.
pub(crate) trait Exec: Send + Sync + 'static {
    /// Runs one turn.
    fn execute(&self, pack: Pack) -> Boxed<Finished>;
    /// Where the provider stands now, asked of the binary.
    fn providers(&self) -> Boxed<Vec<ProviderState>>;
    /// Whether the environment the child gets holds nothing denied.
    fn env_is_clean(&self) -> bool;
}

/// `claude`, found on the user's `PATH` and run as it is.
#[derive(Debug, Clone)]
pub(crate) struct ClaudeExec {
    pub(crate) program: PathBuf,
    /// A directory of the runner's own, the same every turn: Claude Code keys a session
    /// by its working directory, so a resume must start where the session did.
    pub(crate) workdir: PathBuf,
    /// Already scrubbed.
    env: Vec<(OsString, OsString)>,
    pub(crate) claude_dir: Option<PathBuf>,
    pub(crate) runner_id: String,
    pub(crate) user: String,
}

impl ClaudeExec {
    /// `parent_env` is the runner's own environment; it is scrubbed here and nowhere
    /// else, so a child can never get it unscrubbed.
    pub(crate) fn new(
        program: PathBuf,
        workdir: PathBuf,
        parent_env: impl IntoIterator<Item = (OsString, OsString)>,
        claude_dir: Option<PathBuf>,
        runner_id: String,
        user: String,
    ) -> Self {
        Self {
            program,
            workdir,
            env: env::scrub(parent_env),
            claude_dir,
            runner_id,
            user,
        }
    }

    /// Where each session's last measured context is kept: beside the working directory,
    /// not in it, so the provider's tools never see it.
    fn context_dir(&self) -> PathBuf {
        self.workdir
            .parent()
            .unwrap_or(&self.workdir)
            .join("context")
    }

    fn command(&self, args: &[String]) -> Command {
        let mut c = Command::new(&self.program);
        c.args(args)
            .env_clear()
            .envs(self.env.iter().map(|(k, v)| (k, v)))
            // The host's GitHub sign-in stays out of the model's tools (RFC 0147).
            .envs(env::contained())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        // Its own process group, so a stop can take the whole tree with it.
        #[cfg(unix)]
        c.process_group(0);
        c
    }

    /// `claude <args>` with no stdin, its stdout, within 30 s.
    async fn ask(&self, args: &[&str]) -> Option<String> {
        let args: Vec<String> = args.iter().map(|s| (*s).to_owned()).collect();
        let mut c = self.command(&args);
        c.stdin(Stdio::null());
        let out = tokio::time::timeout(Duration::from_secs(30), c.output())
            .await
            .ok()?
            .ok()?;
        Some(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    pub(crate) async fn auth_status(&self) -> AuthState {
        match self.ask(&["auth", "status"]).await {
            Some(text) => provider::parse_auth_status(&text),
            None => AuthState::Refused("claude auth status did not run".into()),
        }
    }

    /// The raw `claude auth status` line, for the identity hash at registration.
    pub(crate) async fn identity_line(&self) -> Option<String> {
        self.ask(&["auth", "status"]).await
    }

    pub(crate) async fn version(&self) -> String {
        self.ask(&["--version"])
            .await
            .map(|s| s.trim().to_owned())
            .unwrap_or_default()
    }

    /// The model's `cargo` builds into its repository's shared directory, not into a
    /// `target/` of the job's checkout (RFC 0189). `checkout` is relative to the working
    /// directory.
    fn turn_command(&self, args: &[String], checkout: &str) -> Command {
        let mut cmd = self.command(args);
        cmd.current_dir(&self.workdir);
        let rel = Path::new(checkout);
        if !checkout.is_empty()
            && rel
                .components()
                .all(|c| matches!(c, std::path::Component::Normal(_)))
            && let Some(t) = hygiene::target_for(&self.workdir, &self.workdir.join(rel))
        {
            cmd.env(hygiene::TARGET_VAR, t);
        }
        cmd
    }

    async fn run_turn(&self, pack: &Pack) -> Finished {
        if pack.user != self.user {
            return Finished::refused(
                "refused_user",
                "the turn is on another person's seat (invariant 1)",
            );
        }
        match self.auth_status().await {
            AuthState::Subscription => {}
            AuthState::SignedOut => {
                return Finished::refused("login_expired", "claude is signed out on this machine");
            }
            AuthState::Refused(why) => return Finished::refused("refused_provider", why),
        }
        let provider_version = self.version().await;
        let session = claude::choose_session(
            &pack.resume_session,
            &pack.resume_runner_id,
            &self.runner_id,
            |id| {
                self.claude_dir
                    .as_deref()
                    .is_some_and(|d| claude::session_exists_in(d, id))
            },
            |id| claude::recorded_context(&self.context_dir(), id),
            pack.context_ceiling,
            claude::new_session_id,
        );
        let args = match claude::build_args(&TurnSpec {
            system_prompt: &pack.system_prompt,
            model: &pack.model,
            max_turns: pack.max_turns,
            allowed_tools: &pack.allowed_tools,
            session: &session,
        }) {
            Ok(a) => a,
            Err(e) => return Finished::refused("failed", e.to_string()),
        };
        let fail = |detail: String| Finished {
            resumed: session.resumed(),
            session_id: session.id().to_owned(),
            provider_version: provider_version.clone(),
            ..Finished::refused("failed", detail)
        };
        if let Err(e) = tokio::fs::create_dir_all(&self.workdir).await {
            return fail(format!("the working directory: {e}"));
        }
        let mut cmd = self.turn_command(&args, &pack.checkout);
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => return fail(format!("claude did not start: {e}")),
        };
        // From here the guard kills the whole process group when this future is dropped
        // (the loop aborts a turn on shutdown or revocation) or leaves early.
        let mut group = GroupGuard::new(child.id());
        let wall = if pack.wall_clock_secs == 0 {
            DEFAULT_WALL_CLOCK
        } else {
            Duration::from_secs(pack.wall_clock_secs)
        };
        let (stdout, status) = match drive(&mut child, pack.prompt.as_bytes(), wall).await {
            Ok(done) => {
                group.disarm();
                done
            }
            Err(e) => {
                group.kill();
                let _ = child.kill().await;
                let _ = child.wait().await;
                return match e {
                    DriveError::Deadline => Finished {
                        outcome: Outcome::CapHit.as_str().into(),
                        detail: "wall-clock cap".into(),
                        ..fail(String::new())
                    },
                    DriveError::Failed(d) => fail(d),
                };
            }
        };
        let parsed = claude::parse_result(&String::from_utf8_lossy(&stdout), status.success());
        // What the provider measured now decides, next turn, whether this session is
        // resumed or started afresh from the pack.
        if !parsed.session_id.is_empty() {
            claude::record_context(
                &self.context_dir(),
                &parsed.session_id,
                claude::context_tokens(&parsed.usage),
            );
        }
        Finished {
            outcome: parsed.outcome.as_str().into(),
            session_id: if parsed.session_id.is_empty() {
                session.id().to_owned()
            } else {
                parsed.session_id.clone()
            },
            parsed: Some(parsed),
            resumed: matches!(session, Session::Resume(_)),
            detail: String::new(),
            provider_version,
        }
    }
}

/// Why [`drive`] stopped early.
enum DriveError {
    Deadline,
    Failed(String),
}

/// Writes the prompt, reads at most [`MAX_OUTPUT`] bytes of stdout and waits for the
/// exit, all under one deadline. A write failure or more output than the cap ends it at
/// once; the caller kills and reaps the child.
async fn drive(
    child: &mut Child,
    prompt: &[u8],
    wall: Duration,
) -> Result<(Vec<u8>, std::process::ExitStatus), DriveError> {
    let stdin = child.stdin.take();
    let stdout = child.stdout.take();
    let work = async {
        let ((), out) = tokio::try_join!(write_prompt(stdin, prompt), read_capped(stdout))?;
        let status = child
            .wait()
            .await
            .map_err(|e| DriveError::Failed(format!("claude stopped: {e}")))?;
        Ok((out, status))
    };
    match tokio::time::timeout(wall, work).await {
        Ok(r) => r,
        Err(_) => Err(DriveError::Deadline),
    }
}

async fn write_prompt(stdin: Option<ChildStdin>, prompt: &[u8]) -> Result<(), DriveError> {
    if let Some(mut stdin) = stdin {
        stdin
            .write_all(prompt)
            .await
            .map_err(|e| DriveError::Failed(format!("the prompt was not taken by claude: {e}")))?;
        // Dropped here: the prompt ends.
    }
    Ok(())
}

/// Reads to the end, holding at most `MAX_OUTPUT` bytes; one more is an error.
async fn read_capped(stdout: Option<ChildStdout>) -> Result<Vec<u8>, DriveError> {
    let mut out = Vec::new();
    let Some(mut stdout) = stdout else {
        return Ok(out);
    };
    let mut buf = vec![0_u8; 16 * 1024];
    loop {
        let n = stdout
            .read(&mut buf)
            .await
            .map_err(|e| DriveError::Failed(format!("claude's output: {e}")))?;
        if n == 0 {
            return Ok(out);
        }
        if out.len() + n > MAX_OUTPUT {
            return Err(DriveError::Failed(format!(
                "claude printed more than {MAX_OUTPUT} bytes; stopped"
            )));
        }
        out.extend_from_slice(&buf[..n]);
    }
}

/// Kills a turn's whole process group (`claude` and whatever its tools started) when it
/// is dropped while armed: an aborted turn task never leaves a process running.
pub(super) struct GroupGuard {
    pgid: Option<u32>,
}

impl GroupGuard {
    pub(super) fn new(pid: Option<u32>) -> Self {
        // `process_group(0)` made the child its group's leader: pgid is its pid.
        Self {
            pgid: if cfg!(unix) { pid } else { None },
        }
    }

    /// The child exited and was reaped: its pid may be reused, so nothing to kill.
    pub(super) fn disarm(&mut self) {
        self.pgid = None;
    }

    fn kill(&mut self) {
        if let Some(pgid) = self.pgid.take() {
            kill_group(pgid);
        }
    }
}

impl Drop for GroupGuard {
    fn drop(&mut self) {
        self.kill();
    }
}

/// Test helper: whether process `pid` is gone, waiting up to 5 s. A zombie counts as gone: it is
/// dead and only waits for its parent (or an init that does not reap) to collect it.
#[cfg(all(test, unix))]
pub(crate) async fn process_gone(pid: i32) -> bool {
    for _ in 0..100 {
        let alive = std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        // /proc where it exists (Linux, any ps); `ps` elsewhere (macOS has no /proc).
        let zombie = match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
            Ok(t) => t
                .rsplit(')')
                .next()
                .is_some_and(|r| r.trim_start().starts_with('Z')),
            Err(_) => std::process::Command::new("ps")
                .args(["-o", "stat=", "-p", &pid.to_string()])
                .stderr(Stdio::null())
                .output()
                .is_ok_and(|o| {
                    String::from_utf8_lossy(&o.stdout)
                        .trim_start()
                        .starts_with('Z')
                }),
        };
        if !alive || zombie {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

/// `kill -KILL -<pgid>`. The crate forbids `unsafe`, so the shell's `kill` does it. No `--`:
/// dash (Debian and Ubuntu's `/bin/sh`) reads it as a pid and refuses, and the error goes to
/// /dev/null, so the group would survive without a word.
fn kill_group(pgid: u32) {
    let _ = std::process::Command::new("/bin/sh")
        .args(["-c", "kill -KILL \"-$0\" 2>/dev/null", &pgid.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

impl Exec for ClaudeExec {
    fn execute(&self, pack: Pack) -> Boxed<Finished> {
        let me = self.clone();
        Box::pin(async move { me.run_turn(&pack).await })
    }

    fn providers(&self) -> Boxed<Vec<ProviderState>> {
        let me = self.clone();
        Box::pin(async move {
            let s = me.auth_status().await;
            vec![ProviderState {
                provider: "claude".into(),
                method: if s.signed_in() { "subscription" } else { "" }.into(),
                signed_in: s.signed_in(),
            }]
        })
    }

    fn env_is_clean(&self) -> bool {
        env::is_clean(&self.env)
    }
}

#[cfg(all(test, unix))]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]
    use super::process_gone as gone;
    use super::*;
    use std::os::unix::fs::PermissionsExt as _;

    const ME: &str = "run_me";
    const OLD: &str = "7d2f9a10-1111-4222-8333-444455556666";

    /// A `claude` that answers `auth status` and `--version` and, for a turn, writes the
    /// arguments, the environment and the prompt it was started with next to itself.
    fn fake_claude(dir: &std::path::Path, auth: &str) -> PathBuf {
        let p = dir.join("claude");
        std::fs::write(
            &p,
            format!(
                r#"#!/bin/sh
d=$(dirname "$0")
case "$1" in
  auth) echo '{auth}'; exit 0;;
  --version) echo '9.9.9 (fake)'; exit 0;;
esac
printf '%s\n' "$@" > "$d/args.txt"
env > "$d/env.txt"
cat > "$d/prompt.txt"
echo '{{"type":"result","subtype":"success","is_error":false,"result":"{{\"say\":\"ok\"}}","session_id":"s1","num_turns":2,"duration_ms":5,"total_cost_usd":0.01}}'
"#
            ),
        )
        .unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    const SUB: &str = r#"{"loggedIn":true,"authMethod":"claude.ai","apiProvider":"firstParty"}"#;

    fn dirty_parent() -> Vec<(OsString, OsString)> {
        [
            ("PATH", "/usr/bin:/bin"),
            ("HOME", "/home/x"),
            ("ANTHROPIC_API_KEY", "sk-secret"),
            ("ANTHROPIC_BASE_URL", "https://proxy.example"),
            ("CLAUDE_CODE_USE_BEDROCK", "1"),
            ("AWS_ACCESS_KEY_ID", "AKIA"),
            ("GOOGLE_APPLICATION_CREDENTIALS", "/k.json"),
            ("OPENAI_API_KEY", "sk-o"),
            // Not in the old denylist: only an allowlist drops them.
            ("GH_TOKEN", "ghp_secret"),
            ("GITHUB_TOKEN", "ghs_secret"),
            ("AZURE_API_KEY", "azure-secret"),
            ("FOO_SECRET", "foo-secret"),
        ]
        .map(|(k, v)| (k.into(), v.into()))
        .to_vec()
    }

    fn pack(user: &str) -> Pack {
        Pack {
            turn_id: "turn_1".into(),
            user: user.into(),
            kind: "coordinate".into(),
            system_prompt: "THE PACK".into(),
            prompt: "hello there".into(),
            model: "sonnet".into(),
            max_turns: 7,
            allowed_tools: vec!["Read".into()],
            ..Pack::default()
        }
    }

    fn exec(dir: &std::path::Path, auth: &str) -> ClaudeExec {
        ClaudeExec::new(
            fake_claude(dir, auth),
            dir.join("work"),
            dirty_parent(),
            Some(dir.join("claudedir")),
            ME.into(),
            "usr_me".into(),
        )
    }

    #[tokio::test]
    async fn a_fresh_turn_starts_claude_clean_and_without_bare() {
        let dir = tempfile::tempdir().unwrap();
        let e = exec(dir.path(), SUB);
        assert!(e.env_is_clean());
        let f = e.execute(pack("usr_me")).await;
        assert_eq!(f.outcome, "replied", "{}", f.detail);
        assert!(!f.resumed);
        assert_eq!(f.provider_version, "9.9.9 (fake)");
        let args = std::fs::read_to_string(dir.path().join("args.txt")).unwrap();
        let args: Vec<&str> = args.lines().collect();
        assert!(!args.contains(&"--bare"));
        assert!(args.contains(&"--session-id") && args.contains(&"--system-prompt"));
        assert!(!args.contains(&"--resume"));
        assert_eq!(args[..3], ["-p", "--output-format", "json"]);
        let env = std::fs::read_to_string(dir.path().join("env.txt")).unwrap();
        for bad in [
            "ANTHROPIC_API_KEY",
            "ANTHROPIC_BASE_URL",
            "CLAUDE_CODE_USE_BEDROCK",
            "AWS_ACCESS_KEY_ID",
            "GOOGLE_APPLICATION_CREDENTIALS",
            "OPENAI_API_KEY",
            "GH_TOKEN",
            "GITHUB_TOKEN",
            "AZURE_API_KEY",
            "FOO_SECRET",
            "ghp_secret",
            "azure-secret",
            "foo-secret",
        ] {
            assert!(!env.contains(bad), "{bad} reached the child");
        }
        assert!(env.contains("HOME=/home/x"));
        // The containment: gh signed out, no git credential helper, no prompt.
        assert!(env.contains(&format!("GH_CONFIG_DIR={}", env::NO_GH_CONFIG)));
        assert!(env.contains("GIT_CONFIG_KEY_0=credential.helper"));
        assert!(env.lines().any(|l| l == "GIT_CONFIG_VALUE_0="));
        assert!(env.contains("GIT_TERMINAL_PROMPT=0"));
        assert!(!env.contains("IOHR_GIT_TOKEN"));
        assert_eq!(
            std::fs::read_to_string(dir.path().join("prompt.txt")).unwrap(),
            "hello there"
        );
    }

    #[tokio::test]
    async fn this_runners_session_is_resumed_and_another_runners_is_not() {
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("claudedir/projects/-w");
        std::fs::create_dir_all(&proj).unwrap();
        std::fs::write(proj.join(format!("{OLD}.jsonl")), "{}\n").unwrap();
        let e = exec(dir.path(), SUB);

        let mut p = pack("usr_me");
        p.resume_session = OLD.into();
        p.resume_runner_id = ME.into();
        let f = e.execute(p.clone()).await;
        assert!(f.resumed, "{}", f.detail);
        let args = std::fs::read_to_string(dir.path().join("args.txt")).unwrap();
        let args: Vec<&str> = args.lines().collect();
        assert!(args.windows(2).any(|w| w == ["--resume", OLD]));
        assert!(!args.contains(&"--session-id") && !args.contains(&"--system-prompt"));

        p.resume_runner_id = "run_other".into();
        let f = e.execute(p).await;
        assert!(!f.resumed);
        let args = std::fs::read_to_string(dir.path().join("args.txt")).unwrap();
        assert!(args.contains("--session-id") && !args.contains("--resume"));
    }

    #[tokio::test]
    async fn a_session_whose_last_context_is_over_the_ceiling_starts_fresh_from_the_pack() {
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("claudedir/projects/-w");
        std::fs::create_dir_all(&proj).unwrap();
        std::fs::write(proj.join(format!("{OLD}.jsonl")), "{}\n").unwrap();
        let e = exec(dir.path(), SUB);
        claude::record_context(&e.context_dir(), OLD, 120_001);

        let mut p = pack("usr_me");
        p.resume_session = OLD.into();
        p.resume_runner_id = ME.into();
        p.context_ceiling = 120_000;
        let f = e.execute(p.clone()).await;
        assert!(!f.resumed, "{}", f.detail);
        let args = std::fs::read_to_string(dir.path().join("args.txt")).unwrap();
        assert!(args.contains("--session-id") && args.contains("--system-prompt"));
        assert!(!args.contains("--resume") && !args.contains("--fork-session"));

        // Under the ceiling the same session is resumed.
        claude::record_context(&e.context_dir(), OLD, 119_999);
        let f = e.execute(p).await;
        assert!(f.resumed, "{}", f.detail);
    }

    #[tokio::test]
    async fn another_persons_turn_and_a_non_subscription_sign_in_never_start_claude() {
        let dir = tempfile::tempdir().unwrap();
        let e = exec(dir.path(), SUB);
        assert_eq!(e.execute(pack("usr_other")).await.outcome, "refused_user");
        assert!(!dir.path().join("args.txt").exists());

        let dir = tempfile::tempdir().unwrap();
        let e = exec(
            dir.path(),
            r#"{"loggedIn":true,"authMethod":"api_key","apiProvider":"firstParty"}"#,
        );
        assert_eq!(e.execute(pack("usr_me")).await.outcome, "refused_provider");
        assert!(!dir.path().join("args.txt").exists());

        let dir = tempfile::tempdir().unwrap();
        let e = exec(dir.path(), r#"{"loggedIn":false}"#);
        assert_eq!(e.execute(pack("usr_me")).await.outcome, "login_expired");
        assert!(!e.providers().await[0].signed_in);
    }

    /// A `claude` whose turn is `body`; `auth status` and `--version` answer as usual.
    fn script_claude(dir: &std::path::Path, body: &str) -> ClaudeExec {
        let p = dir.join("claude");
        std::fs::write(
            &p,
            format!(
                "#!/bin/sh\ncase \"$1\" in\n  auth) echo '{SUB}'; exit 0;;\n  --version) echo 1; exit 0;;\nesac\n{body}\n"
            ),
        )
        .unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        ClaudeExec::new(
            p,
            dir.join("work"),
            dirty_parent(),
            None,
            ME.into(),
            "usr_me".into(),
        )
    }

    #[tokio::test]
    async fn a_child_that_never_reads_its_prompt_hits_the_wall_clock_and_is_killed() {
        let dir = tempfile::tempdir().unwrap();
        let pidfile = dir.path().join("pid");
        let e = script_claude(
            dir.path(),
            &format!("echo $$ > {}\nexec sleep 60", pidfile.display()),
        );
        let mut p = pack("usr_me");
        // Far over the pipe's buffer: the write blocks for good.
        p.prompt = "x".repeat(4 * 1024 * 1024);
        p.wall_clock_secs = 1;
        let started = std::time::Instant::now();
        let f = tokio::time::timeout(Duration::from_secs(15), e.execute(p))
            .await
            .expect("the stdin write is under the deadline");
        assert_eq!(f.outcome, "cap_hit", "{}", f.detail);
        assert!(started.elapsed() < Duration::from_secs(10));
        let pid: i32 = std::fs::read_to_string(&pidfile)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert!(gone(pid).await, "claude {pid} is still running");
    }

    #[tokio::test]
    async fn output_over_the_cap_stops_the_turn_instead_of_filling_memory() {
        let dir = tempfile::tempdir().unwrap();
        let e = script_claude(
            dir.path(),
            "cat > /dev/null\nexec yes xxxxxxxxxxxxxxxxxxxxxxxx",
        );
        let mut p = pack("usr_me");
        p.wall_clock_secs = 20;
        let f = tokio::time::timeout(Duration::from_secs(25), e.execute(p))
            .await
            .unwrap();
        assert_eq!(f.outcome, "failed");
        assert!(f.detail.contains("more than"), "{}", f.detail);
    }

    #[tokio::test]
    async fn a_dropped_turn_kills_claude_and_what_it_started() {
        let dir = tempfile::tempdir().unwrap();
        let (pid, kid) = (dir.path().join("pid"), dir.path().join("kid"));
        let e = script_claude(
            dir.path(),
            &format!(
                "sleep 60 &\necho $! > {}\necho $$ > {}\nwait",
                kid.display(),
                pid.display()
            ),
        );
        let task = tokio::spawn(async move { e.execute(pack("usr_me")).await });
        for _ in 0..100 {
            if pid.exists() && kid.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let read = |f: &std::path::Path| -> i32 {
            std::fs::read_to_string(f).unwrap().trim().parse().unwrap()
        };
        let (p, k) = (read(&pid), read(&kid));
        task.abort();
        let _ = task.await;
        assert!(gone(p).await, "claude {p} is still running");
        assert!(gone(k).await, "its child {k} is still running");
    }
}
