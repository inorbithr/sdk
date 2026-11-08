//! `iohr session`, `iohr stop` and the host half of `iohr runner run --session` (RFC 0213).
//!
//! A shell never opens the queue and never touches a task: it joins the host's socket
//! ([`crate::session::link`]) and the host does the rest.

#[cfg(unix)]
pub(crate) use unix::{data_dir, enter, open, start_host, stop};

#[cfg(not(unix))]
pub(crate) use other::{enter, open, start_host, stop};

#[cfg(unix)]
mod unix {
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc;
    use std::thread::JoinHandle;
    use std::time::{Duration, Instant};

    use crate::cli::{Global, SessionArgs, StopArgs};
    use crate::error::Error;
    use crate::ext::install::Store;
    use crate::output::Out;
    use crate::roles;
    use crate::session::host::{self, HostError, Options};
    use crate::session::lifecycle::Paths;
    use crate::session::link::{self, Stopped};
    use crate::session::queue::Task;
    use crate::session::turn::{self, Globals};

    /// How long `iohr session` waits for a host it started to answer: the host signs in and
    /// checks its runner before it listens.
    const START_WAIT: Duration = Duration::from_secs(30);

    fn project_dir(dir: Option<&Path>) -> Result<PathBuf, Error> {
        let dir = match dir {
            Some(d) => d.to_path_buf(),
            None => std::env::current_dir()
                .map_err(|e| Error::Failed(format!("cannot read the current folder: {e}")))?,
        };
        if !dir.is_dir() {
            return Err(Error::Usage(format!("{} is not a folder", dir.display())));
        }
        Ok(crate::paths::real(&dir))
    }

    /// The host's files for `project`: below `--data-dir` (`IOHR_DATA_DIR`), else the
    /// platform's data directory, the one extensions use.
    fn paths_for(g: &Global, project: &Path) -> Result<Paths, Error> {
        Ok(Paths::for_project(&data_dir(g)?, project))
    }

    /// The iohr data folder the hosts keep their files below.
    pub(crate) fn data_dir(g: &Global) -> Result<PathBuf, Error> {
        match &g.data_dir {
            Some(d) => Ok(d.clone()),
            None => Store::data_root().map_err(|e| Error::Failed(e.to_string())),
        }
    }

    /// The Roles a project hosts: the packs it holds, else what its language selects.
    fn host_roles(project: &Path) -> Vec<String> {
        let mut names: Vec<String> = roles::locals(project)
            .into_iter()
            .map(|l| l.role.name)
            .collect();
        if names.is_empty() {
            let lang = roles::detect_language(project);
            names = roles::selection(lang).into_iter().map(|r| r.name).collect();
        }
        names.sort();
        names.dedup();
        names
    }

    /// The host running inside `iohr runner run --session`.
    pub(crate) struct Hosted {
        stop: Arc<AtomicBool>,
        thread: JoinHandle<Result<(), HostError>>,
    }

    impl Hosted {
        /// Asks the host to end and waits for its Roles to finish what they hold.
        pub(crate) fn finish(self) -> Result<(), Error> {
            self.stop.store(true, Ordering::SeqCst);
            match self.thread.join() {
                Ok(Ok(())) => Ok(()),
                Ok(Err(e)) => Err(Error::Failed(e.to_string())),
                Err(_) => Err(Error::Failed("the session host panicked".into())),
            }
        }
    }

    /// The flags a Role's turn carries from this host: its profile, environment and data
    /// folder, so a turn runs where the host does.
    fn globals_of(g: &Global) -> Globals {
        let mut v: Vec<OsString> = Vec::new();
        if let Some(p) = &g.profile {
            v.push("--profile".into());
            v.push(p.as_str().into());
        }
        if let Some(e) = &g.env {
            v.push("--env".into());
            v.push(e.into());
        }
        if let Some(d) = &g.data_dir {
            v.push("--data-dir".into());
            v.push(d.into());
        }
        Globals(v)
    }

    /// Starts the host for `project` on a thread of this process. `on_end` runs when it ends
    /// for any reason (the last session left, `iohr stop`, a queue failure), and is how the
    /// runner's own loop is told to stop. Returns once the socket accepts connections.
    pub(crate) fn start_host(
        g: &Global,
        project: &Path,
        on_end: impl FnOnce() + Send + 'static,
    ) -> Result<Hosted, Error> {
        let project = project_dir(Some(project))?;
        let paths = paths_for(g, &project)?;
        let roles = host_roles(&project);
        let globals = globals_of(g);
        let exe = std::env::current_exe()
            .map_err(|e| Error::Failed(format!("cannot find this program: {e}")))?;
        let stop = Arc::new(AtomicBool::new(false));
        let (told, heard) = mpsc::channel::<Result<(), String>>();
        let thread = {
            let (stop, told) = (stop.clone(), told.clone());
            std::thread::spawn(move || {
                // A Role's turn is `iohr ask --role` as a child of this host (RFC 0213, S2).
                let turn = |task: &Task, role: &str| {
                    let result = turn::run(&exe, &globals, task, role, &stop);
                    if let Ok(reply) = &result {
                        turn::keep_notes(&project, task, role, reply);
                    }
                    result
                };
                let result = host::serve(
                    &paths,
                    &roles,
                    &turn,
                    &stop,
                    Options::default(),
                    |recovered| {
                        let _ = told.send(Ok(()));
                        if recovered.requeued + recovered.failed > 0 {
                            Out::note(&format!(
                                "session queue recovered: {} task(s) back in the queue, {} failed after too many tries",
                                recovered.requeued, recovered.failed
                            ));
                        }
                    },
                );
                if let Err(e) = &result {
                    let _ = told.send(Err(e.to_string()));
                }
                on_end();
                result
            })
        };
        match heard.recv_timeout(START_WAIT) {
            Ok(Ok(())) => Ok(Hosted { stop, thread }),
            Ok(Err(why)) => {
                let _ = thread.join();
                Err(Error::Failed(why))
            }
            Err(_) => {
                stop.store(true, Ordering::SeqCst);
                Err(Error::Failed(
                    "the session host did not start in time".into(),
                ))
            }
        }
    }

    /// The command line of the host `iohr session` starts: this command line with the
    /// `session` command replaced by `runner run`, so every global flag (`--profile`,
    /// `--env`, ...) carries over, and `--session <project>` added. The command is the first
    /// word that is not a flag and not a flag's value, so a profile, a data folder or a project
    /// folder called `session` is left alone.
    fn host_args(argv: &[OsString], dir: Option<&Path>, project: &Path) -> Vec<OsString> {
        let takes_value = crate::cli::global_value_flags();
        let mut at = None;
        let mut i = 0;
        while i < argv.len() {
            let word = &argv[i];
            if takes_value.contains(word) {
                // `--profile session`: the next word is this flag's value.
                i += 2;
                continue;
            }
            if !word.to_string_lossy().starts_with('-') {
                at = Some(i);
                break;
            }
            i += 1;
        }
        let Some(at) = at else {
            // Bare `iohr`: only global flags, the command is added after them.
            let mut out = argv.to_vec();
            out.extend([
                "runner".into(),
                "run".into(),
                "--session".into(),
                project.into(),
            ]);
            return out;
        };
        let mut out: Vec<OsString> = Vec::new();
        for (i, a) in argv.iter().enumerate() {
            if i == at {
                out.push("runner".into());
                out.push("run".into());
            } else if i > at && dir.is_some_and(|d| a == d.as_os_str()) {
                // The folder is passed as the flag's value instead.
            } else {
                out.push(a.clone());
            }
        }
        out.push("--session".into());
        out.push(project.into());
        out
    }

    fn start_process(
        g: &Global,
        dir: Option<&Path>,
        project: &Path,
        paths: &Paths,
    ) -> Result<(), Error> {
        use std::os::unix::process::CommandExt as _;
        let _ = g;
        paths
            .create()
            .map_err(|e| Error::Failed(format!("cannot make {}: {e}", paths.dir.display())))?;
        let exe = std::env::current_exe()
            .map_err(|e| Error::Failed(format!("cannot find this program: {e}")))?;
        let argv: Vec<OsString> = std::env::args_os().skip(1).collect();
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&paths.log)
            .map_err(|e| Error::Failed(format!("cannot open {}: {e}", paths.log.display())))?;
        // The log is appended to across starts: what this start wrote begins here.
        let log_from = log.metadata().map_or(0, |m| m.len());
        let mut child = Command::new(exe)
            .args(host_args(&argv, dir, project))
            .stdin(Stdio::null())
            .stdout(log.try_clone().map_err(|e| Error::Failed(e.to_string()))?)
            .stderr(log)
            // Its own process group: Ctrl-C in this terminal must not reach it, it stops
            // when the last session leaves.
            .process_group(0)
            .spawn()
            .map_err(|e| Error::Failed(format!("cannot start the session host: {e}")))?;
        let end = Instant::now() + START_WAIT;
        while Instant::now() < end {
            if link::running(paths).is_some() {
                return Ok(());
            }
            if let Ok(Some(status)) = child.try_wait() {
                return Err(Error::Failed(stopped_at_once(
                    &status.to_string(),
                    &paths.log,
                    log_from,
                )));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        Err(Error::Failed(format!(
            "the session host did not answer in {} seconds; see {}",
            START_WAIT.as_secs(),
            paths.log.display()
        )))
    }

    /// Why the host stopped at once: its own last line in the log (the cause, and what to do
    /// about it), redacted like every report of a tool's output, then the log's path.
    fn stopped_at_once(status: &str, log: &Path, from: u64) -> String {
        let said = std::fs::read(log).ok().and_then(|bytes| {
            let start = usize::try_from(from).unwrap_or(0).min(bytes.len());
            last_line(&String::from_utf8_lossy(&bytes[start..]))
        });
        match said {
            Some(line) => format!(
                "the session host stopped at once ({status}): {line}; see {}",
                log.display()
            ),
            None => format!(
                "the session host stopped at once ({status}); see {}",
                log.display()
            ),
        }
    }

    /// The last line of the host's log with something on it, redacted and cut to 300
    /// characters. `None` when the log said nothing.
    fn last_line(text: &str) -> Option<String> {
        const CAP: usize = 300;
        let line = text.lines().rev().map(str::trim).find(|l| !l.is_empty())?;
        let line = crate::runner::host_action::redact_output(line);
        let line = line.trim().trim_end_matches('.');
        Some(line.chars().take(CAP).collect())
    }

    /// `iohr session`: joins the host, starting it when none runs, and stays open.
    /// Joins the host of `project`, starting it first when none runs: what `iohr session` and
    /// bare `iohr` both do.
    async fn join_host(
        g: &Global,
        dir: Option<PathBuf>,
        project: &Path,
    ) -> Result<link::Session, Error> {
        let paths = paths_for(g, project)?;
        let (g, project) = (g.clone(), project.to_path_buf());
        tokio::task::spawn_blocking(move || {
            if link::running(&paths).is_none() {
                start_process(&g, dir.as_deref(), &project, &paths)?;
            }
            link::join(&paths).map_err(|e| Error::Failed(format!("cannot join the host: {e}")))
        })
        .await
        .map_err(|e| Error::Failed(e.to_string()))?
    }

    /// Bare `iohr` on a terminal: joins (or starts) the host of the current folder and floats
    /// the panel over the person's shell until it exits. When the host cannot be had, says so
    /// and still gives the shell with the panel: the asks and Workloads need no host.
    pub(crate) async fn enter(g: &Global, env: crate::Env) -> Result<(), Error> {
        let project = project_dir(None)?;
        let joined = match join_host(g, None, &project).await {
            Ok(j) => Some(j),
            Err(e) => {
                Out::note(&format!("no session host: {e}; the panel runs without one"));
                None
            }
        };
        let gone = joined
            .as_ref()
            .and_then(|j| j.watcher().ok())
            .map(|watcher| {
                let (tx, rx) = tokio::sync::oneshot::channel::<()>();
                // A plain thread: it blocks until the host closes, and must not hold the runtime
                // open once the shell has exited.
                std::thread::spawn(move || {
                    if watcher.wait_for_host() {
                        let _ = tx.send(());
                    }
                });
                rx
            });
        let stopped = crate::panel::session_shell(g, env, gone).await;
        drop(joined);
        if stopped? {
            Out::note("the session host stopped");
        }
        Ok(())
    }

    /// `iohr session`: joins the host, starting it when none runs, and stays open.
    pub(crate) async fn open(g: &Global, args: &SessionArgs, out: Out) -> Result<(), Error> {
        let project = project_dir(args.dir.as_deref())?;
        let joined = join_host(g, args.dir.clone(), &project).await?;
        let roles = host_roles(&project);
        if out.json {
            Out::print_json(&serde_json::json!({
                "project": project,
                "host_pid": joined.host_pid,
                "roles": roles,
            }));
        }
        Out::note(&format!(
            "session open: host {} runs {} Role(s) ({}); a `?` here is answered by every Role too, one card each; Ctrl-C leaves, the host stops when the last session has left",
            joined.host_pid,
            roles.len(),
            roles.join(", ")
        ));
        let watcher = joined.watcher().map_err(|e| Error::Failed(e.to_string()))?;
        let host_gone = tokio::task::spawn_blocking(move || watcher.wait_for_host());
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .map_err(|e| Error::Failed(e.to_string()))?;
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
            _ = host_gone => Out::note("the session host stopped"),
        }
        drop(joined);
        Ok(())
    }

    /// `iohr stop`.
    pub(crate) async fn stop(g: &Global, args: &StopArgs, out: Out) -> Result<(), Error> {
        let project = project_dir(args.dir.as_deref())?;
        let paths = paths_for(g, &project)?;
        let wait = Duration::from_secs(args.wait);
        let stopped = tokio::task::spawn_blocking(move || link::stop(&paths, wait))
            .await
            .map_err(|e| Error::Failed(e.to_string()))?;
        let (state, pid) = match stopped {
            Stopped::NotRunning => ("not running", None),
            Stopped::Gone(p) => ("stopped", Some(p)),
            Stopped::Stopping(p) => ("stopping", Some(p)),
        };
        // Inside the panel the panel goes with the host: it lifts and leaves the terminal's modes
        // as the shell had them. With no host running, that is still what was stopped.
        let panel_closed = crate::panel::end_from_inside();
        if out.json {
            Out::print_json(&serde_json::json!({
                "project": project,
                "state": state,
                "host_pid": pid,
                "panel_closed": panel_closed,
            }));
        }
        let line = match stopped {
            Stopped::NotRunning if panel_closed => {
                "the panel is closed; no session host ran".into()
            }
            Stopped::NotRunning => format!("no session host runs for {}", project.display()),
            Stopped::Gone(p) => format!("session host {p} stopped"),
            Stopped::Stopping(p) => {
                format!("session host {p} is stopping: each Role finishes the task it holds first")
            }
        };
        Out::note(&if panel_closed && pid.is_some() {
            format!("{line}; the panel is closed")
        } else {
            line
        });
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn os(v: &[&str]) -> Vec<OsString> {
            v.iter().map(OsString::from).collect()
        }

        #[test]
        fn the_hosts_last_line_is_the_cause_and_is_redacted() {
            let log = "starting\n\nthis sign-in lacks coordinator:runner: sign in with `iohr login --scope coordinator:runner`\n  \n";
            assert_eq!(
                last_line(log).as_deref(),
                Some(
                    "this sign-in lacks coordinator:runner: sign in with `iohr login --scope coordinator:runner`"
                )
            );
            let secret = last_line("failed: Bearer abc123 at https://u:p@host/x").unwrap();
            assert!(
                !secret.contains("abc123") && !secret.contains("u:p"),
                "{secret}"
            );
            assert_eq!(last_line("  \n\n"), None);
            assert!(last_line(&"x ".repeat(400)).unwrap().chars().count() <= 300);
        }

        #[test]
        fn only_what_this_start_wrote_is_read_from_the_log() {
            let dir = tempfile::tempdir().unwrap();
            let log = dir.path().join("host.log");
            std::fs::write(&log, "old failure\n").unwrap();
            let from = std::fs::metadata(&log).unwrap().len();
            // This start wrote nothing: the old line is not its cause.
            assert_eq!(
                stopped_at_once("exit status: 1", &log, from),
                format!(
                    "the session host stopped at once (exit status: 1); see {}",
                    log.display()
                )
            );
            std::fs::write(&log, "old failure\nno runner key\n").unwrap();
            assert!(stopped_at_once("exit status: 1", &log, from).contains(": no runner key; see"));
        }

        #[test]
        fn the_host_gets_the_same_flags_and_the_folder_as_its_own_flag() {
            let p = Path::new("/p");
            assert_eq!(
                host_args(&os(&["--profile", "x", "session", "/p"]), Some(p), p),
                os(&["--profile", "x", "runner", "run", "--session", "/p"])
            );
            assert_eq!(
                host_args(&os(&["session", "--env", "local"]), None, p),
                os(&["runner", "run", "--env", "local", "--session", "/p"])
            );
        }

        /// A flag's value can be the word `session` (a profile of that name), and so can the
        /// folder: only the command itself is replaced.
        #[test]
        fn a_value_called_session_is_not_the_command() {
            let p = Path::new("session");
            assert_eq!(
                host_args(
                    &os(&["--profile", "session", "session", "session"]),
                    Some(p),
                    p
                ),
                os(&[
                    "--profile",
                    "session",
                    "runner",
                    "run",
                    "--session",
                    "session"
                ])
            );
            assert_eq!(
                host_args(&os(&["--data-dir", "session", "-v", "session"]), None, p),
                os(&[
                    "--data-dir",
                    "session",
                    "-v",
                    "runner",
                    "run",
                    "--session",
                    "session"
                ])
            );
            // Bare `iohr`: only global flags, the command follows them.
            assert_eq!(
                host_args(&os(&["--env", "local"]), None, p),
                os(&["--env", "local", "runner", "run", "--session", "session"])
            );
            // The `=` form is one word, and is never mistaken for the command either.
            assert_eq!(
                host_args(&os(&["--profile=session", "session"]), None, p),
                os(&["--profile=session", "runner", "run", "--session", "session"])
            );
        }
    }
}

#[cfg(not(unix))]
mod other {
    use std::path::Path;

    use crate::cli::{Global, SessionArgs, StopArgs};
    use crate::error::Error;
    use crate::output::Out;

    const NEEDS_UNIX: &str = "the session host needs a unix system";

    pub(crate) struct Hosted;

    impl Hosted {
        pub(crate) fn finish(self) -> Result<(), Error> {
            Ok(())
        }
    }

    pub(crate) fn start_host(
        _: &Global,
        _: &Path,
        _: impl FnOnce() + Send + 'static,
    ) -> Result<Hosted, Error> {
        Err(Error::Failed(NEEDS_UNIX.into()))
    }

    pub(crate) async fn enter(_: &Global, _: crate::Env) -> Result<(), Error> {
        Err(Error::Failed(NEEDS_UNIX.into()))
    }

    pub(crate) async fn open(_: &Global, _: &SessionArgs, _: Out) -> Result<(), Error> {
        Err(Error::Failed(NEEDS_UNIX.into()))
    }

    pub(crate) async fn stop(_: &Global, _: &StopArgs, _: Out) -> Result<(), Error> {
        Err(Error::Failed(NEEDS_UNIX.into()))
    }
}
