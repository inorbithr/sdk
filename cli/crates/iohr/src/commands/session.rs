//! `iohr session`, `iohr stop` and the host half of `iohr runner run --session` (RFC 0213).
//!
//! A shell never opens the queue and never touches a task: it joins the host's socket
//! ([`crate::session::link`]) and the host does the rest.

#[cfg(unix)]
pub(crate) use unix::{open, start_host, stop};

#[cfg(not(unix))]
pub(crate) use other::{open, start_host, stop};

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
        let data = match &g.data_dir {
            Some(d) => d.clone(),
            None => Store::data_root().map_err(|e| Error::Failed(e.to_string()))?,
        };
        Ok(Paths::for_project(&data, project))
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

    /// What a Role does with a task in this slice: nothing yet. The turn itself (`claude -p`
    /// through the runner's turn function) is the next slice, so a task fails with that said
    /// plainly instead of looking done.
    fn no_turn_yet(task: &Task, role: &str) -> Result<String, String> {
        Err(format!(
            "Role {role} cannot run task {} yet: its turn handler is not built (RFC 0213, next slice)",
            task.id
        ))
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
        let stop = Arc::new(AtomicBool::new(false));
        let (told, heard) = mpsc::channel::<Result<(), String>>();
        let thread = {
            let (stop, told) = (stop.clone(), told.clone());
            std::thread::spawn(move || {
                let result = host::serve(
                    &paths,
                    &roles,
                    &no_turn_yet,
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

    /// The command line of the host `iohr session` starts: this command line with `session`
    /// replaced by `runner run`, so every global flag (`--profile`, `--env`, ...) carries over,
    /// and `--session <project>` added.
    fn host_args(argv: &[OsString], dir: Option<&Path>, project: &Path) -> Vec<OsString> {
        let at = argv.iter().position(|a| a == "session");
        let mut out: Vec<OsString> = Vec::new();
        for (i, a) in argv.iter().enumerate() {
            if Some(i) == at {
                out.push("runner".into());
                out.push("run".into());
            } else if Some(i) > at && dir.is_some_and(|d| a == d.as_os_str()) {
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
                return Err(Error::Failed(format!(
                    "the session host stopped at once ({status}); see {}",
                    paths.log.display()
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

    /// `iohr session`: joins the host, starting it when none runs, and stays open.
    pub(crate) async fn open(g: &Global, args: &SessionArgs, out: Out) -> Result<(), Error> {
        let project = project_dir(args.dir.as_deref())?;
        let paths = paths_for(g, &project)?;
        let (g, dir, spawn_paths, spawn_project) =
            (g.clone(), args.dir.clone(), paths.clone(), project.clone());
        let joined = tokio::task::spawn_blocking(move || {
            if link::running(&spawn_paths).is_none() {
                start_process(&g, dir.as_deref(), &spawn_project, &spawn_paths)?;
            }
            link::join(&spawn_paths)
                .map_err(|e| Error::Failed(format!("cannot join the host: {e}")))
        })
        .await
        .map_err(|e| Error::Failed(e.to_string()))??;
        let roles = host_roles(&project);
        if out.json {
            Out::print_json(&serde_json::json!({
                "project": project,
                "host_pid": joined.host_pid,
                "roles": roles,
            }));
        }
        Out::note(&format!(
            "session open: host {} runs {} Role(s) ({}); Ctrl-C leaves, the host stops when the last session has left",
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
        if out.json {
            Out::print_json(&serde_json::json!({
                "project": project,
                "state": state,
                "host_pid": pid,
            }));
        }
        Out::note(&match stopped {
            Stopped::NotRunning => format!("no session host runs for {}", project.display()),
            Stopped::Gone(p) => format!("session host {p} stopped"),
            Stopped::Stopping(p) => {
                format!("session host {p} is stopping: each Role finishes the task it holds first")
            }
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

    pub(crate) async fn open(_: &Global, _: &SessionArgs, _: Out) -> Result<(), Error> {
        Err(Error::Failed(NEEDS_UNIX.into()))
    }

    pub(crate) async fn stop(_: &Global, _: &StopArgs, _: Out) -> Result<(), Error> {
        Err(Error::Failed(NEEDS_UNIX.into()))
    }
}
