//! The runner upgrades itself (RFC 0120, owner rule of 2026-10-19: every local runner runs
//! the newest released `iohr`, updates itself and is restarted).
//!
//! # State machine
//!
//! ```text
//!            start
//!              | first look after FIRST_CHECK (+ jitter)
//!              v
//!   +-----> Waiting --- shutdown / loop ended ------------------> Stopped
//!   |          | delay elapsed
//!   |          v
//!   |      Checking -- upgrades off ------------------------------+
//!   |          | newest complete release (the `iohr upgrade` lookup)
//!   |          +-- not newer (semver, prereleases ordered) -------+
//!   |          +-- newer, APT or a copy iohr did not install -----+ reported once per version
//!   |          +-- newer, installed in the last 24 h, still old --+ (restart-loop guard)
//!   |          +-- lookup failed -> failures + 1 ----------------+
//!   |          v newer
//!   |      Preparing -- no work is held back: download, checksum and attestation (install
//!   |          |        script) or `brew` and the formula's version (Homebrew)
//!   |          +-- fails -> failures + 1 (the runner never stopped working)
//!   |          +-- the tap lags the release -> ledger records the version, failures + 1
//!   |          +-- shutdown -> Stopped
//!   |          v ready
//!   |      Draining -- want(true): beats declare nothing, offers are left, no approved run
//!   |          |        starts; waits until nothing is leased or running
//!   |          +-- DRAIN_LIMIT passes, or shutdown -> want(false), failures + 1 / Stopped
//!   |          v drained
//!   |      Installing -- the swap (rename, or `brew upgrade`); then `--version` of the file
//!   |          +-- fails -> want(false), failures + 1
//!   |          +-- says another version -> ledger records the target version (a tap that
//!   |          |   lags), want(false), failures + 1
//!   |          v ledger records the version (a failure to record stops here, too)
//!   |      Restarting -- exec: the process becomes the new binary and never comes back here
//!   |          |         exit: Outcome::Exit, the caller stops the loop and exits 75; the
//!   |          |         drain stays on, so no lease is taken in the last moments
//!   |          +-- exec fails -> want(false), failures + 1
//!   +----------+  every arrow back to Waiting waits `delay`: the interval after a success,
//!                 5 min doubling to 6 h (never past the interval) after failures in a row
//! ```
//!
//! Invariants: no lease is cut (drain first, and a run in flight holds the drain open);
//! the runner idles only for the swap itself, never for a download that can fail; any
//! failure leaves the old version running, logs one line, backs off and never retries in
//! a loop; the same version is not tried twice in 24 hours while this process still runs
//! an older one (also when the install gave another version); shutdown ends the machine
//! at every waiting point.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use semver::Version;
use tokio::sync::watch;

use super::drain::Drain;
use super::settings::RestartMode;
use super::supervisor::{self, Restart, Supervisor};
use crate::commands::upgrade::{self, Method, Upstream};
use crate::error::Error;

/// The first look comes this long after the runner starts (plus up to 10 percent).
pub(crate) const FIRST_CHECK: Duration = Duration::from_secs(60);
/// Back-off after a failure: from 5 minutes, doubling, to 6 hours.
pub(crate) const FAIL_MIN: Duration = Duration::from_mins(5);
pub(crate) const FAIL_MAX: Duration = Duration::from_hours(6);
/// How long to wait for leased work to finish before giving up on this round.
pub(crate) const DRAIN_LIMIT: Duration = Duration::from_hours(4);
/// A version installed this recently is not installed again by a runner that still says
/// it is older.
pub(crate) const REINSTALL_GUARD: Duration = Duration::from_hours(24);

/// What the person set (`auto_upgrade`, `upgrade_interval_minutes`, `upgrade_restart`),
/// read again on SIGHUP.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Policy {
    pub(crate) enabled: bool,
    pub(crate) interval: Duration,
    pub(crate) restart: RestartMode,
}

impl Policy {
    /// What the settings in use say.
    pub(crate) fn of(e: &super::settings::Effective) -> Self {
        Self {
            enabled: e.auto_upgrade.value,
            interval: e.upgrade_interval.value,
            restart: e.upgrade_restart.value,
        }
    }
}

/// Where this copy is and how it got there.
#[derive(Debug, Clone)]
pub(crate) struct Here {
    /// The executable as run (the path to run again).
    pub(crate) exe: PathBuf,
    /// Its real path.
    pub(crate) real: PathBuf,
    pub(crate) method: Method,
}

/// The releases and the installer; injected so tests need no network and no disk.
/// Errors are the one line to log, without secrets.
pub(crate) trait Source {
    /// What [`Source::prepare`] hands to [`Source::apply`]: the verified download.
    type Staged: Send + 'static;
    fn here(&self) -> Result<Here, String>;
    /// The newest complete release for a copy installed by `method`.
    async fn newest(&self, method: &Method) -> Result<Version, String>;
    /// `iohr upgrade`'s download and verification of `version` (or `brew` and the
    /// formula's version), before the runner stops taking work; changes nothing.
    async fn prepare(&self, here: &Here, version: &Version) -> Result<Self::Staged, Refused>;
    /// The swap of what [`Source::prepare`] made ready; the old binary stays on failure.
    async fn apply(
        &self,
        here: &Here,
        version: &Version,
        staged: Self::Staged,
    ) -> Result<(), String>;
    /// What the file at `here.exe` says its version is.
    fn installed(&self, here: &Here) -> Result<Version, String>;
}

/// Why a release cannot be made ready.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Refused {
    /// The one line to log, without secrets.
    pub(crate) why: String,
    /// Retrying this version before the guard window passes would fail the same way (the
    /// Homebrew tap lags the release): the ledger notes the version.
    pub(crate) hold: bool,
}

impl Refused {
    pub(crate) fn failed(why: impl Into<String>) -> Self {
        Self {
            why: why.into(),
            hold: false,
        }
    }
}

/// What was tried last, so a restart that does not take cannot loop.
pub(crate) trait Ledger {
    /// Whether `version` was installed (or found to be unavailable) within
    /// [`REINSTALL_GUARD`].
    fn installed_recently(&self, version: &Version) -> bool;
    fn record(&self, version: &Version) -> Result<(), String>;
}

/// Starts the new binary in place.
pub(crate) trait Restarter {
    /// Replaces this process with `here.exe`; returns only on failure, with the reason.
    fn exec(&self, here: &Here) -> String;
}

/// What looking for a release found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Check {
    Disabled,
    /// This process runs the newest release (or a newer one).
    Current(Version),
    /// A newer release this copy cannot install by itself, with what to do.
    Skipped(Version, String),
    Failed(String),
    Newer(Version, Here),
}

impl PartialEq for Here {
    fn eq(&self, o: &Self) -> bool {
        self.exe == o.exe && self.real == o.real && self.method == o.method
    }
}
impl Eq for Here {}

/// How the upgrader ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// The runner ended, or was asked to stop.
    Stopped,
    /// Installed; the caller stops the loop and exits [`supervisor::EXIT_RESTART`].
    Exit,
}

/// Seconds to wait before the next look: the interval after a success, the back-off after
/// `failures` in a row (never past the interval), plus up to 10 percent of `jitter`
/// (0.0 to 1.0) so runners do not look in step.
pub(crate) fn delay(interval: Duration, failures: u32, jitter: f64) -> Duration {
    let base = if failures == 0 {
        interval
    } else {
        FAIL_MIN
            .saturating_mul(2_u32.saturating_pow(failures.saturating_sub(1).min(16)))
            .min(FAIL_MAX)
            .min(interval.max(FAIL_MIN))
    };
    base + base.mul_f64(0.1 * jitter.clamp(0.0, 1.0))
}

pub(crate) struct Upgrader<S, L, R> {
    pub(crate) source: S,
    pub(crate) ledger: L,
    pub(crate) restarter: R,
    pub(crate) supervisor: Supervisor,
    pub(crate) current: Version,
    pub(crate) policy: watch::Receiver<Policy>,
    pub(crate) drain: Arc<Drain>,
    pub(crate) drain_limit: Duration,
    /// 0.0 to 1.0, read before each wait.
    pub(crate) jitter: fn() -> f64,
}

impl<S: Source, L: Ledger, R: Restarter> Upgrader<S, L, R> {
    /// One look: whether a newer release exists and can be installed by this copy.
    pub(crate) async fn check(&self) -> Check {
        if !self.policy.borrow().enabled {
            return Check::Disabled;
        }
        let here = match self.source.here() {
            Ok(h) => h,
            Err(e) => return Check::Failed(e),
        };
        let latest = match self.source.newest(&here.method).await {
            Ok(v) => v,
            Err(e) => return Check::Failed(e),
        };
        if latest <= self.current {
            return Check::Current(latest);
        }
        match &here.method {
            Method::Apt => Check::Skipped(
                latest,
                "this copy came from the APT package: run `sudo apt-get install --only-upgrade iohr`"
                    .to_owned(),
            ),
            Method::Other => Check::Skipped(
                latest,
                format!(
                    "this copy ({}) is not one iohr installed: install it again with Homebrew, the install script, APT or winget",
                    here.real.display()
                ),
            ),
            Method::Homebrew | Method::InstallScript { .. } => {
                if self.ledger.installed_recently(&latest) {
                    Check::Skipped(
                        latest.clone(),
                        format!(
                            "iohr {latest} was tried in the last 24 hours and this runner still says {}: not trying it again, restart the runner or install it yourself",
                            self.current
                        ),
                    )
                } else {
                    Check::Newer(latest, here)
                }
            }
        }
    }

    /// Prepares the install (download and checks), drains, swaps and restarts.
    /// `Ok(Some(Outcome::Exit))`: the caller stops and exits; `Ok(None)`: shutdown came
    /// first; `Err`: the line to log, and the old version still runs with leases taken
    /// again (it never stopped taking them when the failure came before the drain).
    pub(crate) async fn upgrade(
        &self,
        version: &Version,
        here: &Here,
        mut stop: watch::Receiver<bool>,
        log: &dyn Fn(&str),
    ) -> Result<Option<Outcome>, String> {
        log(&format!(
            "iohr {version} is out (this runner is {}): downloading and checking it",
            self.current
        ));
        // Everything that can fail on the network or the machine happens while the runner
        // still works: a failed download must not idle it.
        let staged = tokio::select! {
            r = self.source.prepare(here, version) => match r {
                Ok(staged) => staged,
                Err(refused) => {
                    let note = if refused.hold {
                        self.ledger.record(version).err().map_or_else(
                            String::new,
                            |e| format!(" (and cannot note it, so the next look repeats: {e})"),
                        )
                    } else {
                        String::new()
                    };
                    return Err(format!(
                        "iohr {version} cannot be installed, {} keeps running: {}{note}",
                        self.current, refused.why
                    ));
                }
            },
            _ = stop.changed() => return Ok(None),
        };
        log(&format!(
            "iohr {version} is ready: taking no new work until the running work is done"
        ));
        self.drain.want(true);
        let outcome = self
            .upgrade_drained(version, here, staged, &mut stop, log)
            .await;
        // Any way out but an exit takes work again; an exec that worked never gets here,
        // and on exit the caller ends the loop, so no lease is taken in between.
        if !matches!(outcome, Ok(Some(Outcome::Exit))) {
            self.drain.want(false);
        }
        outcome
    }

    async fn upgrade_drained(
        &self,
        version: &Version,
        here: &Here,
        staged: S::Staged,
        stop: &mut watch::Receiver<bool>,
        log: &dyn Fn(&str),
    ) -> Result<Option<Outcome>, String> {
        tokio::select! {
            () = self.drain.until_drained() => {}
            () = tokio::time::sleep(self.drain_limit) => {
                return Err(format!(
                    "work was still running after {} minutes: upgrade to {version} later",
                    self.drain_limit.as_secs() / 60
                ));
            }
            _ = stop.changed() => return Ok(None),
        }
        self.source
            .apply(here, version, staged)
            .await
            .map_err(|e| {
                format!(
                    "installing iohr {version} failed, {} keeps running: {e}",
                    self.current
                )
            })?;
        let said = self
            .source
            .installed(here)
            .map_err(|e| format!("iohr {version} was installed but does not run: {e}"))?;
        if said != *version {
            // Homebrew installs the formula's version, and the tap can lag the release:
            // note the version so the next look does not repeat this every round.
            let note = self
                .ledger
                .record(version)
                .err()
                .map_or_else(String::new, |e| {
                    format!(" (and cannot note it, so the next look repeats: {e})")
                });
            return Err(format!(
                "iohr {version} was installed but the file says {said}: not restarting{note}"
            ));
        }
        self.ledger
            .record(version)
            .map_err(|e| format!("cannot note the install of {version}, not restarting: {e}"))?;
        match supervisor::plan(&self.supervisor, self.policy.borrow().restart) {
            Restart::Exit => {
                log(&format!(
                    "iohr {version} is installed: exiting {} for {} to start it",
                    supervisor::EXIT_RESTART,
                    self.supervisor
                ));
                Ok(Some(Outcome::Exit))
            }
            Restart::Exec => {
                log(&format!(
                    "iohr {version} is installed: restarting in place ({})",
                    self.supervisor
                ));
                Err(format!(
                    "iohr {version} is installed but {}; {} keeps running",
                    self.restarter.exec(here),
                    self.current
                ))
            }
        }
    }

    /// Looks on the schedule until the runner ends. Every failure is one log line and a
    /// longer wait; nothing here retries at once.
    pub(crate) async fn run(&self, mut stop: watch::Receiver<bool>, log: &dyn Fn(&str)) -> Outcome {
        let mut failures: u32 = 0;
        let mut told: Option<Version> = None;
        let mut wait = FIRST_CHECK + FIRST_CHECK.mul_f64(0.1 * (self.jitter)().clamp(0.0, 1.0));
        loop {
            tokio::select! {
                biased;
                _ = stop.changed() => return Outcome::Stopped,
                () = tokio::time::sleep(wait) => {}
            }
            match self.check().await {
                Check::Disabled => {
                    failures = 0;
                }
                Check::Current(_) => failures = 0,
                Check::Skipped(v, what) => {
                    failures = 0;
                    if told.as_ref() != Some(&v) {
                        log(&format!(
                            "iohr {v} is out; the runner does not upgrade itself: {what}"
                        ));
                        told = Some(v);
                    }
                }
                Check::Failed(e) => {
                    failures = failures.saturating_add(1);
                    log(&format!(
                        "looking for a newer iohr failed ({failures} in a row): {e}"
                    ));
                }
                Check::Newer(v, here) => match self.upgrade(&v, &here, stop.clone(), log).await {
                    Ok(Some(Outcome::Exit)) => return Outcome::Exit,
                    Ok(Some(Outcome::Stopped) | None) => return Outcome::Stopped,
                    Err(e) => {
                        failures = failures.saturating_add(1);
                        log(&format!(
                            "upgrade failed ({failures} in a row), the old version runs on: {e}"
                        ));
                    }
                },
            }
            wait = delay(self.policy.borrow().interval, failures, (self.jitter)());
        }
    }
}

/// The real release lookup and installer: the code of `iohr upgrade`.
pub(crate) struct RealSource;

impl Source for RealSource {
    type Staged = upgrade::Staged;

    fn here(&self) -> Result<Here, String> {
        let (exe, real, method) = upgrade::detect_here().map_err(|e| e.to_string())?;
        Ok(Here { exe, real, method })
    }

    async fn newest(&self, method: &Method) -> Result<Version, String> {
        Upstream::from_process()
            .map_err(|e| e.to_string())?
            .newest(method)
            .await
            .map_err(|e| e.to_string())
    }

    async fn prepare(&self, here: &Here, version: &Version) -> Result<Self::Staged, Refused> {
        let (real, method, version) = (here.real.clone(), here.method.clone(), version.clone());
        // `brew` blocks, and the runner's loop shares a thread with this: it runs on the
        // blocking pool, on a runtime of its own with its own HTTP client.
        tokio::task::spawn_blocking(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| Refused::failed(format!("cannot start: {e}")))?;
            rt.block_on(async {
                Upstream::from_process()
                    .map_err(|e| Refused::failed(e.to_string()))?
                    .prepare(&method, &real, &version, false)
                    .await
                    .map_err(|e| match e {
                        upgrade::Prepare::TapBehind { .. } => Refused {
                            why: Error::from(e).to_string(),
                            hold: true,
                        },
                        upgrade::Prepare::Failed(e) => Refused::failed(e.to_string()),
                    })
            })
        })
        .await
        .map_err(|e| Refused::failed(format!("the download did not finish: {e}")))?
    }

    async fn apply(&self, _: &Here, version: &Version, staged: Self::Staged) -> Result<(), String> {
        // `brew upgrade` and the file replacement block.
        let version = version.clone();
        tokio::task::spawn_blocking(move || staged.apply(&version).map_err(|e| e.to_string()))
            .await
            .map_err(|e| format!("the install did not finish: {e}"))?
    }

    fn installed(&self, here: &Here) -> Result<Version, String> {
        upgrade::installed_version(&here.exe).map_err(|e| e.to_string())
    }
}

/// The marker file `runner/last-upgrade` in the config directory: `<version> <unix seconds>`.
pub(crate) struct FileLedger {
    pub(crate) path: PathBuf,
}

impl Ledger for FileLedger {
    fn installed_recently(&self, version: &Version) -> bool {
        let Ok(text) = std::fs::read_to_string(&self.path) else {
            return false;
        };
        let mut parts = text.split_whitespace();
        let (Some(v), Some(at)) = (
            parts.next(),
            parts.next().and_then(|a| a.parse::<u64>().ok()),
        ) else {
            return false;
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        v == version.to_string() && now.saturating_sub(at) < REINSTALL_GUARD.as_secs()
    }

    fn record(&self, version: &Version) -> Result<(), String> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {}", dir.display(), e.kind()))?;
        }
        std::fs::write(&self.path, format!("{version} {now}\n"))
            .map_err(|e| format!("{}: {}", self.path.display(), e.kind()))
    }
}

/// `execv` of the same path with the same arguments.
pub(crate) struct ExecRestarter;

impl Restarter for ExecRestarter {
    fn exec(&self, here: &Here) -> String {
        supervisor::exec_in_place(&here.exe)
    }
}

#[cfg(test)]
mod tests;
