//! Who restarts the runner (`runner::self_upgrade`): launchd, systemd or nobody, read
//! from the environment, and how an upgraded runner comes back under each.
//!
//! - **exec** (the default): the same process becomes the new binary (`execv` of the same
//!   path and arguments, so the same PID and the same environment). The supervisor sees no
//!   stop, so no `ThrottleInterval`, `RestartSec`, start-limit or `KeepAlive` setting can
//!   leave the runner down, and no label or domain has to be known.
//! - **exit** (`upgrade_restart = "exit"`): under launchd or systemd the runner exits with
//!   [`EXIT_RESTART`] and the supervisor starts it again. A non-zero code restarts under
//!   `KeepAlive` (true or `SuccessfulExit=false`) and `Restart=always` or `on-failure`.
//!   With no supervisor there is nobody to start it, so it stays exec.
//!
//! `launchctl kickstart -k` is not used: it needs the label and the `gui/<uid>` domain,
//! kills the runner with SIGKILL from inside itself and leaves it down if `launchctl`
//! fails after the kill; exec has none of those failure modes.

use std::ffi::OsString;
use std::path::Path;

use super::settings::RestartMode;

/// The exit code of a runner that stops to be restarted by its supervisor: `EX_TEMPFAIL`.
pub(crate) const EXIT_RESTART: u8 = 75;

/// What supervises this process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Supervisor {
    /// A launchd job (`XPC_SERVICE_NAME` is its label, and launchd is the parent).
    Launchd { label: String },
    /// A systemd service, system or user (`INVOCATION_ID`).
    Systemd,
    /// A terminal, a script or anything else.
    None,
}

impl std::fmt::Display for Supervisor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Launchd { label } => write!(f, "launchd ({label})"),
            Self::Systemd => f.write_str("systemd"),
            Self::None => f.write_str("no supervisor"),
        }
    }
}

/// What the process knows about itself; a value so tests need no environment.
#[derive(Debug, Clone, Default)]
pub(crate) struct Surroundings {
    pub(crate) invocation_id: Option<String>,
    /// `SYSTEMD_EXEC_PID`: systemd sets it to the process it started.
    pub(crate) systemd_exec_pid: Option<String>,
    pub(crate) xpc_service_name: Option<String>,
    pub(crate) pid: u32,
    pub(crate) parent_pid: u32,
}

impl Surroundings {
    pub(crate) fn from_process() -> Self {
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
        Self {
            invocation_id: var("INVOCATION_ID"),
            systemd_exec_pid: var("SYSTEMD_EXEC_PID"),
            xpc_service_name: var("XPC_SERVICE_NAME"),
            pid: std::process::id(),
            #[cfg(unix)]
            parent_pid: std::os::unix::process::parent_id(),
            #[cfg(not(unix))]
            parent_pid: 0,
        }
    }
}

/// The supervisor of this process.
///
/// systemd sets `INVOCATION_ID` for every service it starts; when it also names the
/// process it started (`SYSTEMD_EXEC_PID`) and that is not this one, the variable was
/// inherited by a shell or a child and this is not the supervised process. launchd sets
/// `XPC_SERVICE_NAME` to the job's label, `0` in a terminal session, and the label of
/// the app in a shell opened from one; only a process whose parent is launchd itself
/// (PID 1) is the job.
pub(crate) fn detect(s: &Surroundings) -> Supervisor {
    if s.invocation_id.is_some()
        && s.systemd_exec_pid
            .as_deref()
            .is_none_or(|p| p.trim().parse::<u32>().is_ok_and(|p| p == s.pid))
    {
        return Supervisor::Systemd;
    }
    if let Some(label) = s.xpc_service_name.as_deref().map(str::trim)
        && label != "0"
        && !label.is_empty()
        && s.parent_pid == 1
    {
        return Supervisor::Launchd {
            label: label.to_owned(),
        };
    }
    Supervisor::None
}

/// How the runner comes back after an upgrade.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Restart {
    /// `execv` the new binary in this process.
    Exec,
    /// Exit with [`EXIT_RESTART`]; the supervisor starts it.
    Exit,
}

/// The restart for a supervisor and a setting: exit only where something will start the
/// runner again.
pub(crate) fn plan(supervisor: &Supervisor, mode: RestartMode) -> Restart {
    match (mode, supervisor) {
        (RestartMode::Exit, Supervisor::Launchd { .. } | Supervisor::Systemd) => Restart::Exit,
        _ => Restart::Exec,
    }
}

/// Replaces this process with the program at `path`, with this process's arguments and
/// environment. Returns only when that failed, with the reason.
#[cfg(unix)]
pub(crate) fn exec_in_place(path: &Path) -> String {
    use std::os::unix::process::CommandExt as _;
    let mut args = std::env::args_os();
    let argv0 = args.next().unwrap_or_else(|| OsString::from("iohr"));
    let rest: Vec<OsString> = args.collect();
    let err = std::process::Command::new(path)
        .arg0(argv0)
        .args(rest)
        .exec();
    format!("cannot start {}: {err}", path.display())
}

/// Replaces this process: not possible here.
#[cfg(not(unix))]
pub(crate) fn exec_in_place(path: &Path) -> String {
    let _ = (path, OsString::new());
    "restarting in place is not supported on this system: restart the runner yourself".to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn here() -> Surroundings {
        Surroundings {
            pid: 4242,
            parent_pid: 777,
            ..Surroundings::default()
        }
    }

    #[test]
    fn a_systemd_service_is_known_by_its_invocation_id() {
        let mut s = here();
        assert_eq!(detect(&s), Supervisor::None, "a bare environment");
        s.invocation_id = Some("0f5e".into());
        assert_eq!(detect(&s), Supervisor::Systemd);
        s.systemd_exec_pid = Some("4242".into());
        assert_eq!(detect(&s), Supervisor::Systemd, "the pid systemd started");
        s.systemd_exec_pid = Some("999".into());
        assert_eq!(
            detect(&s),
            Supervisor::None,
            "inherited from another process of the service"
        );
        s.systemd_exec_pid = Some("not a pid".into());
        assert_eq!(detect(&s), Supervisor::None);
    }

    #[test]
    fn a_launchd_job_is_known_by_its_label_and_its_parent() {
        let mut s = here();
        s.xpc_service_name = Some("hr.inorbit.runner".into());
        assert_eq!(
            detect(&s),
            Supervisor::None,
            "a shell opened from an app inherits the label, its parent is not launchd"
        );
        s.parent_pid = 1;
        assert_eq!(
            detect(&s),
            Supervisor::Launchd {
                label: "hr.inorbit.runner".into()
            }
        );
        s.xpc_service_name = Some("0".into());
        assert_eq!(detect(&s), Supervisor::None, "0 is a terminal session");
        s.xpc_service_name = Some("  ".into());
        assert_eq!(detect(&s), Supervisor::None);
    }

    #[test]
    fn systemd_wins_when_both_are_set() {
        let s = Surroundings {
            invocation_id: Some("x".into()),
            xpc_service_name: Some("hr.inorbit.runner".into()),
            parent_pid: 1,
            ..here()
        };
        assert_eq!(detect(&s), Supervisor::Systemd);
    }

    #[test]
    fn exit_is_planned_only_where_a_supervisor_starts_the_runner_again() {
        let launchd = Supervisor::Launchd { label: "l".into() };
        for sup in [&launchd, &Supervisor::Systemd, &Supervisor::None] {
            assert_eq!(plan(sup, RestartMode::Exec), Restart::Exec, "{sup}");
        }
        assert_eq!(plan(&launchd, RestartMode::Exit), Restart::Exit);
        assert_eq!(plan(&Supervisor::Systemd, RestartMode::Exit), Restart::Exit);
        assert_eq!(plan(&Supervisor::None, RestartMode::Exit), Restart::Exec);
    }

    #[cfg(unix)]
    #[test]
    fn a_program_that_is_not_there_comes_back_as_a_reason_not_a_crash() {
        let why = exec_in_place(Path::new("/nonexistent/iohr-for-test"));
        assert!(
            why.starts_with("cannot start /nonexistent/iohr-for-test"),
            "{why}"
        );
    }
}
