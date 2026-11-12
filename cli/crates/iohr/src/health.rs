//! What is wrong with this `iohr` and the machine it runs on, found locally (PRD 0208).
//!
//! The checks are pure ([`evaluate`] turns [`Probes`] into [`Finding`]s); the probes are the
//! only part that touches the machine. A finding carries a stable status code and, where
//! there is one, the command that fixes it. A fix that only restarts something already down
//! is `safe` and runs by itself only when `[doctor] auto_fix = true`; every other fix is an
//! offer, never run. Nothing here reads a question, an answer, a token or a file's content:
//! a finding is an id, a status code and a fixed sentence.
//!
//! `iohr doctor` shows the findings; the `?` card closes with the same ones. Reporting them
//! as sensor readings to the platform is a later slice.

use std::path::Path;

use iohr_auth::{Config, DoctorConfig};
use serde::Serialize;

/// The MCP bridge's local address (owner-only, a Mac with the InOrbit agent installed).
const BRIDGE_ADDR: &str = "127.0.0.1:47821";
/// The launchd job that keeps the bridge up.
const BRIDGE_LABEL: &str = "hr.inorbit.mcp-bridge";
/// Under this much free space on the config directory's disk, the disk is `low`.
pub(crate) const DISK_LOW_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// What the probes saw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Probes {
    /// Whether the session is usable: `None` when it was not asked (the `?` card) or the
    /// credential store would have prompted.
    pub(crate) signed_in: Option<bool>,
    pub(crate) bridge: Bridge,
    /// Free bytes where the config directory is; `None` when unknown.
    pub(crate) disk_free: Option<u64>,
    /// Whether `config.toml` parses; the error text is for the person at this terminal.
    pub(crate) config: Result<(), String>,
}

/// The MCP bridge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Bridge {
    /// Not installed here: nothing to check.
    Absent,
    /// Answers on its port.
    Up,
    /// Does not answer; `job_running` says whether launchd shows a process for it
    /// (`None`: could not ask).
    Down { job_running: Option<bool> },
}

/// A fix that is safe to run unasked: it only restarts something that is already down.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SafeFix {
    /// `launchctl kickstart` the bridge job (no `-k`: a running one is never killed).
    StartBridge,
}

/// One check's result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct Finding {
    /// The check: `auth`, `bridge`, `disk`, `config`.
    pub(crate) id: &'static str,
    /// A stable code: `ok`, `signed_out`, `down`, `low`, `invalid`.
    pub(crate) status: &'static str,
    /// One fixed sentence for the person.
    pub(crate) what: String,
    /// The command or step that fixes it; never run unasked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) offer: Option<String>,
    /// The fix that may run by itself under `auto_fix`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) safe: Option<SafeFix>,
}

impl Finding {
    pub(crate) fn failing(&self) -> bool {
        self.status != "ok"
    }

    fn ok(id: &'static str, what: &str) -> Self {
        Self {
            id,
            status: "ok",
            what: what.to_owned(),
            offer: None,
            safe: None,
        }
    }
}

/// The checks, in order. A check whose probe is unknown is left out.
pub(crate) fn evaluate(p: &Probes) -> Vec<Finding> {
    let mut out = Vec::new();
    match p.signed_in {
        Some(true) => out.push(Finding::ok("auth", "signed in")),
        Some(false) => out.push(Finding {
            id: "auth",
            status: "signed_out",
            what: "not signed in, or the sign-in expired".to_owned(),
            offer: Some("iohr login".to_owned()),
            safe: None,
        }),
        None => {}
    }
    match p.bridge {
        Bridge::Absent => {}
        Bridge::Up => out.push(Finding::ok("bridge", "the MCP bridge answers")),
        Bridge::Down { job_running } => out.push(Finding {
            id: "bridge",
            status: "down",
            what: "the MCP bridge does not answer".to_owned(),
            offer: Some(format!("launchctl kickstart gui/$(id -u)/{BRIDGE_LABEL}")),
            // Only a job launchd shows no process for is started; one that runs but does
            // not answer is for a person to look at.
            safe: (job_running == Some(false)).then_some(SafeFix::StartBridge),
        }),
    }
    match p.disk_free {
        Some(free) if free < DISK_LOW_BYTES => out.push(Finding {
            id: "disk",
            status: "low",
            what: format!("{} free on this disk", gib(free)),
            offer: Some("free some space; `mise run target:sweep` clears build folders".to_owned()),
            safe: None,
        }),
        Some(free) => out.push(Finding::ok("disk", &format!("{} free", gib(free)))),
        None => {}
    }
    match &p.config {
        Ok(()) => out.push(Finding::ok("config", "config.toml is valid")),
        Err(e) => out.push(Finding {
            id: "config",
            status: "invalid",
            what: format!("config.toml does not parse: {e}"),
            offer: Some("edit config.toml and fix the line it names".to_owned()),
            safe: None,
        }),
    }
    out
}

fn gib(bytes: u64) -> String {
    #[allow(clippy::cast_precision_loss, reason = "a display value")]
    let g = bytes as f64 / (1024.0 * 1024.0 * 1024.0);
    format!("{g:.1} GiB")
}

/// The probes that need no sign-in and no network: the bridge, the disk and the file.
pub(crate) fn probe_local(dir: &Path) -> Probes {
    Probes {
        signed_in: None,
        bridge: probe_bridge(),
        disk_free: free_bytes(dir),
        config: Config::load(dir).map(|_| ()).map_err(|e| e.to_string()),
    }
}

fn probe_bridge() -> Bridge {
    let Ok(home) = etcetera::home_dir() else {
        return Bridge::Absent;
    };
    if !cfg!(target_os = "macos") || !home.join(".inorbit/bin/iohr-mcp-bridge").exists() {
        return Bridge::Absent;
    }
    let Ok(addr) = BRIDGE_ADDR.parse() else {
        return Bridge::Absent;
    };
    let up =
        std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(300)).is_ok();
    if up {
        Bridge::Up
    } else {
        Bridge::Down {
            job_running: job_running(),
        }
    }
}

/// Whether launchd shows a process for the bridge job.
fn job_running() -> Option<bool> {
    let target = format!("gui/{}/{BRIDGE_LABEL}", uid()?);
    let o = std::process::Command::new("launchctl")
        .args(["print", &target])
        .output()
        .ok()?;
    if !o.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&o.stdout);
    Some(text.lines().any(|l| l.trim_start().starts_with("pid = ")))
}

#[cfg(unix)]
#[allow(
    clippy::unnecessary_wraps,
    reason = "`None` off unix, where there is no uid"
)]
fn uid() -> Option<u32> {
    Some(rustix::process::getuid().as_raw())
}

#[cfg(not(unix))]
fn uid() -> Option<u32> {
    None
}

#[cfg(unix)]
fn free_bytes(dir: &Path) -> Option<u64> {
    // The config directory may not exist yet: ask about the nearest folder that does.
    let at = dir.ancestors().find(|d| d.exists())?;
    let s = rustix::fs::statvfs(at).ok()?;
    Some(s.f_bavail.saturating_mul(s.f_frsize))
}

#[cfg(not(unix))]
fn free_bytes(_: &Path) -> Option<u64> {
    None
}

/// Runs a safe fix; says whether the problem is gone afterwards.
pub(crate) fn run_safe(fix: SafeFix) -> bool {
    match fix {
        SafeFix::StartBridge => {
            let Some(uid) = uid() else { return false };
            let target = format!("gui/{uid}/{BRIDGE_LABEL}");
            let started = std::process::Command::new("launchctl")
                .args(["kickstart", &target])
                .output()
                .is_ok_and(|o| o.status.success());
            // The job needs a moment to open its port.
            for _ in 0..10 {
                if started && matches!(probe_bridge(), Bridge::Up) {
                    return true;
                }
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
            false
        }
    }
}

/// What a fix attempt did, for the line that tells the person.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Fixed {
    /// Ran and the check now passes.
    Yes,
    /// Ran and the problem is still there.
    Still,
}

/// Runs the safe fix of every failing finding, when allowed. Returns the finding ids and
/// what became of each.
pub(crate) fn apply_safe(
    findings: &[Finding],
    allowed: bool,
    run: impl Fn(SafeFix) -> bool,
) -> Vec<(&'static str, Fixed)> {
    if !allowed {
        return Vec::new();
    }
    findings
        .iter()
        .filter(|f| f.failing())
        .filter_map(|f| {
            f.safe
                .map(|s| (f.id, if run(s) { Fixed::Yes } else { Fixed::Still }))
        })
        .collect()
}

/// The lines that close a `?` card: nothing when all is well. A safe fix runs first when
/// the person turned `auto_fix` on, and the line says what happened.
pub(crate) fn notes(dir: &Path, cfg: &DoctorConfig) -> Vec<String> {
    lines(&evaluate(&probe_local(dir)), cfg.auto_fix_on(), run_safe)
}

/// Prints [`notes`] on stderr under a `?` card. The config is read again here so the card's
/// own code stays as it was; a config that does not load prints nothing (`iohr doctor` says why).
pub(crate) fn print_notes(g: &crate::cli::Global, dir: &Path) {
    if let Ok(here) = crate::context::Ctx::load(g) {
        for line in notes(dir, &here.config.doctor) {
            crate::output::Out::note(&line);
        }
    }
}

/// [`notes`] with the findings and the fixer passed in.
pub(crate) fn lines(
    findings: &[Finding],
    auto: bool,
    run: impl Fn(SafeFix) -> bool,
) -> Vec<String> {
    let fixed = apply_safe(findings, auto, run);
    let mut out = Vec::new();
    for f in findings.iter().filter(|f| f.failing()) {
        let line = match fixed.iter().find(|(id, _)| *id == f.id) {
            Some((_, Fixed::Yes)) => format!("Fixed: {} (iohr doctor, auto_fix is on)", f.what),
            Some((_, Fixed::Still)) => {
                format!(
                    "Tried to fix, still failing: {}; `iohr doctor` has details",
                    f.what
                )
            }
            None => match (&f.safe, &f.offer) {
                (Some(_), _) => format!(
                    "{}; `iohr doctor --fix` fixes it, or set `[doctor] auto_fix = true`",
                    f.what
                ),
                (None, Some(_)) => format!("{}; `iohr doctor` says how to fix it", f.what),
                (None, None) => f.what.clone(),
            },
        };
        out.push(line);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probes() -> Probes {
        Probes {
            signed_in: Some(true),
            bridge: Bridge::Up,
            disk_free: Some(50 * 1024 * 1024 * 1024),
            config: Ok(()),
        }
    }

    fn status<'a>(f: &'a [Finding], id: &str) -> &'a Finding {
        f.iter()
            .find(|f| f.id == id)
            .expect("a finding for the check")
    }

    #[test]
    fn a_healthy_machine_has_no_failing_finding() {
        let f = evaluate(&probes());
        assert_eq!(f.len(), 4);
        assert!(f.iter().all(|f| !f.failing()), "{f:?}");
        assert_eq!(lines(&f, true, |_| true), Vec::<String>::new());
    }

    #[test]
    fn each_known_problem_has_its_status_and_fix() {
        let mut p = probes();
        p.signed_in = Some(false);
        p.disk_free = Some(10 * 1024 * 1024);
        p.config = Err("expected a table".to_owned());
        p.bridge = Bridge::Down {
            job_running: Some(true),
        };
        let f = evaluate(&p);
        assert_eq!(status(&f, "auth").status, "signed_out");
        assert_eq!(status(&f, "auth").offer.as_deref(), Some("iohr login"));
        assert_eq!(status(&f, "disk").status, "low");
        assert_eq!(status(&f, "config").status, "invalid");
        assert_eq!(status(&f, "bridge").status, "down");
        // A job that runs but does not answer is not restarted unasked; none here is safe.
        assert!(f.iter().all(|f| f.safe.is_none()));
    }

    #[test]
    fn only_a_stopped_bridge_has_a_safe_fix() {
        let mut p = probes();
        p.bridge = Bridge::Down {
            job_running: Some(false),
        };
        assert_eq!(
            status(&evaluate(&p), "bridge").safe,
            Some(SafeFix::StartBridge)
        );
        p.bridge = Bridge::Down { job_running: None };
        assert_eq!(status(&evaluate(&p), "bridge").safe, None);
    }

    #[test]
    fn unknown_probes_and_a_missing_bridge_are_left_out() {
        let p = Probes {
            signed_in: None,
            bridge: Bridge::Absent,
            disk_free: None,
            config: Ok(()),
        };
        let f = evaluate(&p);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].id, "config");
    }

    #[test]
    fn a_safe_fix_runs_only_when_the_person_said_yes() {
        let mut p = probes();
        p.bridge = Bridge::Down {
            job_running: Some(false),
        };
        let f = evaluate(&p);
        let ran = std::cell::Cell::new(0);
        let run = |_| {
            ran.set(ran.get() + 1);
            true
        };
        let off = lines(&f, false, run);
        assert_eq!(ran.get(), 0, "never without the opt-in");
        assert!(off[0].contains("auto_fix = true"), "{off:?}");
        let on = lines(&f, true, run);
        assert_eq!(ran.get(), 1);
        assert!(on[0].starts_with("Fixed:"), "{on:?}");
        let failed = lines(&f, true, |_| false);
        assert!(failed[0].starts_with("Tried to fix"), "{failed:?}");
    }

    #[test]
    fn an_offered_fix_is_never_run_even_with_the_opt_in() {
        let mut p = probes();
        p.signed_in = Some(false);
        let f = evaluate(&p);
        let ran = std::cell::Cell::new(false);
        let l = lines(&f, true, |_| {
            ran.set(true);
            true
        });
        assert!(!ran.get());
        assert!(l[0].contains("iohr doctor"), "{l:?}");
    }

    #[test]
    fn a_finding_is_codes_only() {
        let mut p = probes();
        p.signed_in = Some(false);
        let json = serde_json::to_string(&evaluate(&p)).expect("serializes");
        assert!(json.contains("\"status\":\"signed_out\""));
        assert!(!json.contains("token"));
    }

    #[test]
    fn config_check_reads_the_real_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(probe_local(dir.path()).config.is_ok());
        std::fs::write(dir.path().join("config.toml"), "[ask\nmode =").expect("write");
        assert!(probe_local(dir.path()).config.is_err());
    }
}
