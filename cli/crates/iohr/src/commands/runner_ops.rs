//! `iohr runner prune` and `iohr runner service`: the runner's housekeeping on this
//! machine. Neither needs a sign-in, except `service uninstall --purge`, which names the
//! account whose keys it removes.

use std::path::PathBuf;
use std::time::SystemTime;

use crate::Env;
use crate::cli::{Global, RunnerServiceCommand};
use crate::context::{Ctx, account as resolve_account, session};
use crate::error::Error;
use crate::output::Out;
use crate::runner::approvals;
use crate::runner::hygiene::{self, Done, Plan, Retention};
use crate::runner::settings::Effective;
use crate::runner::unit::{self, Manager, Spec, Step, Unit};

/// `iohr runner prune [--dry-run]`.
pub(super) async fn prune(ctx: &Ctx, e: &Effective, dry_run: bool, out: Out) -> Result<(), Error> {
    let state = ctx.dir.join("runner");
    let max_gb = e.workspace_max_gb.as_ref().map(|c| c.value);
    let keep_days = e.workspace_keep_days.value;
    let r = Retention::of(keep_days, max_gb);
    let now = SystemTime::now();
    let (plan, done) = tokio::task::spawn_blocking(move || {
        let plan = hygiene::plan(&state, r, now);
        if dry_run {
            return (plan, None);
        }
        let in_use = plan.in_use.clone();
        let done = hygiene::apply(&state, r, plan, now);
        (
            Plan {
                remove: Vec::new(),
                in_use,
            },
            Some(done),
        )
    })
    .await
    .map_err(|e| Error::Failed(format!("the prune stopped: {e}")))?;
    let kept = match max_gb {
        Some(cap) => format!("unused for {keep_days} days, or over {cap} GB"),
        None => format!("unused for {keep_days} days"),
    };
    match done {
        None => {
            if out.json {
                Out::print_json(&serde_json::json!({
                    "dry_run": true, "remove": plan.remove, "in_use": plan.in_use,
                }));
            } else if plan.remove.is_empty() {
                Out::note(&format!("nothing to remove: nothing is {kept}"));
            } else {
                let rows: Vec<_> = plan.remove.iter().map(row).collect();
                Out::table(&["KIND", "PATH", "IDLE", "SIZE", "WHY"], &rows);
                Out::note(&format!(
                    "would remove {} ({kept}); run without --dry-run to remove them",
                    plan.remove.len()
                ));
            }
        }
        Some(done) => {
            if out.json {
                Out::print_json(&serde_json::json!({
                    "dry_run": false,
                    "removed": done.iter().filter(|(_, r)| r.is_ok()).map(|(i, _)| i).collect::<Vec<_>>(),
                    "not_removed": done.iter().filter_map(|(i, r)| r.as_ref().err().map(|e| serde_json::json!({
                        "kind": i.kind, "path": i.path, "error": e,
                    }))).collect::<Vec<_>>(),
                    "in_use": plan.in_use,
                }));
            } else {
                let rows: Vec<_> = done
                    .iter()
                    .filter(|(_, r)| r.is_ok())
                    .map(|(i, _)| row(i))
                    .collect();
                if !rows.is_empty() {
                    Out::table(&["KIND", "PATH", "IDLE", "SIZE", "WHY"], &rows);
                }
                Out::note(&summary_line(&done, &kept));
            }
        }
    }
    if !out.json && !plan.in_use.is_empty() {
        Out::note(&format!(
            "{} checkouts are held by a running lease: kept",
            plan.in_use.len()
        ));
    }
    Ok(())
}

fn row(i: &hygiene::Item) -> Vec<String> {
    vec![
        i.kind.as_str().to_owned(),
        i.path.display().to_string(),
        format!("{} days", i.idle_days),
        i.bytes.map_or_else(|| "-".to_owned(), hygiene::gb),
        i.why.to_owned(),
    ]
}

fn summary_line(done: &[Done], kept: &str) -> String {
    if done.is_empty() {
        format!("nothing to remove: nothing is {kept}")
    } else {
        hygiene::summary(done)
    }
}

/// What the service runs: this `iohr`, for `--profile` when given, with this shell's
/// `PATH`.
fn spec(g: &Global) -> Result<Spec, Error> {
    let program = std::env::current_exe()
        .and_then(std::fs::canonicalize)
        .map_err(|e| Error::Failed(format!("cannot find this iohr's own path: {e}")))?;
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|h| h.is_absolute())
        .ok_or_else(|| Error::Failed("HOME is not set to an absolute path".into()))?;
    let path =
        std::env::var("PATH").map_err(|_| Error::Failed("PATH is not set, or not UTF-8".into()))?;
    let config_dir = g
        .config_dir
        .as_deref()
        .map(std::path::absolute)
        .transpose()
        .map_err(|e| Error::Failed(format!("IOHR_CONFIG_DIR: {e}")))?;
    #[cfg(unix)]
    let uid = rustix::process::getuid().as_raw();
    #[cfg(not(unix))]
    let uid = 0;
    Ok(Spec {
        program,
        profile: g.profile.as_ref().map(|p| p.as_str().to_owned()),
        config_dir,
        home,
        path,
        uid,
        xdg_config: std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute()),
    })
}

fn here() -> Result<Manager, Error> {
    Manager::here().ok_or_else(|| {
        Error::Failed(
            "a runner service is installed on macOS (launchd) and Linux (systemd) only; here, start `iohr runner run` under the system's own service manager".into(),
        )
    })
}

fn steps_json(steps: &[Step]) -> Vec<String> {
    steps.iter().map(ToString::to_string).collect()
}

async fn carry_out(steps: Vec<Step>) -> Result<(), Error> {
    tokio::task::spawn_blocking(move || unit::run(&steps, Out::note))
        .await
        .map_err(|e| Error::Failed(format!("the service steps stopped: {e}")))?
        .map_err(Error::Failed)
}

/// `iohr runner service install|uninstall`.
pub(super) async fn service(
    g: &Global,
    env: &Env,
    ctx: &Ctx,
    cmd: RunnerServiceCommand,
    out: Out,
) -> Result<(), Error> {
    let s = spec(g)?;
    let u = unit::render(here()?, &s).map_err(Error::Failed)?;
    let probe = unit::probe(&u, &s);
    let loaded = tokio::task::spawn_blocking(move || unit::succeeds(&probe))
        .await
        .unwrap_or(false);
    match cmd {
        RunnerServiceCommand::Install { dry_run, force } => {
            if let Ok(old) = std::fs::read_to_string(&u.file)
                && old != u.text
                && !force
            {
                return Err(Error::Failed(format!(
                    "{} exists and differs from this one: `--force` replaces it (`--dry-run` shows the new one)",
                    u.file.display()
                )));
            }
            let steps = unit::install(&u, &s, loaded);
            if dry_run {
                show(&u, &steps, None, out);
                return Ok(());
            }
            carry_out(steps).await?;
            Out::note(&running(&u));
            Ok(())
        }
        RunnerServiceCommand::Uninstall {
            dry_run,
            purge,
            account,
        } => {
            let steps = unit::uninstall(&u, &s, loaded, u.file.exists());
            let purge = if purge {
                Some(Purge::of(g, env, ctx, &u, account.account).await?)
            } else {
                None
            };
            if dry_run {
                show(&u, &steps, purge.as_ref(), out);
                return Ok(());
            }
            if steps.is_empty() {
                Out::note(&format!("no runner service {} is installed", u.name));
            }
            carry_out(steps).await?;
            match purge {
                Some(p) => p.apply(ctx).await,
                None => Ok(()),
            }
        }
    }
}

fn running(u: &Unit) -> String {
    match (&u.manager, &u.log) {
        (Manager::Launchd, Some(log)) => format!(
            "the runner runs as {}, at login from now on; its log is {}",
            u.name,
            log.display()
        ),
        _ => format!(
            "the runner runs as {}, while you are signed in (`loginctl enable-linger` keeps it running after you sign out); `journalctl --user -u {}` shows its log",
            u.name, u.name
        ),
    }
}

/// `--dry-run`: the steps, then the file (install) or what a purge removes, and nothing
/// done.
fn show(u: &Unit, steps: &[Step], purge: Option<&Purge>, out: Out) {
    let install = steps.iter().any(|s| matches!(s, Step::Write(..)));
    if out.json {
        Out::print_json(&serde_json::json!({
            "dry_run": true,
            "name": u.name,
            "file": u.file,
            "steps": steps_json(steps),
            "text": install.then_some(&u.text),
            "purge": purge.map(Purge::json),
        }));
        return;
    }
    if steps.is_empty() {
        Out::note(&format!("no runner service {} is installed", u.name));
    }
    for s in steps {
        Out::note(&format!("would {s}"));
    }
    if let Some(p) = purge {
        for line in p.lines() {
            Out::note(&format!("would {line}"));
        }
    }
    if install {
        Out::raw(u.text.as_bytes());
    }
}

/// What `uninstall --purge` removes besides the service.
struct Purge {
    session: crate::context::Session,
    account: String,
    approvals: PathBuf,
    /// The shared checkouts, mirrors and build directories, when no other runner
    /// service is installed; otherwise empty, and `others` names them.
    shared: Vec<PathBuf>,
    others: Vec<PathBuf>,
}

impl Purge {
    async fn of(
        g: &Global,
        env: &Env,
        ctx: &Ctx,
        u: &Unit,
        account: Option<String>,
    ) -> Result<Self, Error> {
        let s = session(g, env).await?;
        let account = resolve_account(&s, account)?;
        let approvals = approvals::state_dir(&ctx.dir, &account).map_err(Error::Failed)?;
        let others = unit::others(u);
        let shared = if others.is_empty() {
            hygiene::shared(&ctx.dir.join("runner"))
        } else {
            Vec::new()
        };
        Ok(Self {
            session: s,
            account,
            approvals,
            shared,
            others,
        })
    }

    fn lines(&self) -> Vec<String> {
        let a = &self.account;
        let mut l = vec![
            format!("remove the runner key {a}_runner_key from the credential store"),
            format!("remove the run key {a}_approvals_runner_key from the credential store"),
            format!("remove {}", self.approvals.display()),
        ];
        l.extend(
            self.shared
                .iter()
                .map(|p| format!("remove {}", p.display())),
        );
        if !self.others.is_empty() {
            l.push(format!(
                "keep the shared checkouts, mirrors and build directories: {} still uses them",
                self.others
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        l
    }

    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "account": self.account,
            "keys": [format!("{}_runner_key", self.account), format!("{}_approvals_runner_key", self.account)],
            "remove": std::iter::once(&self.approvals).chain(&self.shared).collect::<Vec<_>>(),
            "kept_for": self.others,
        })
    }

    async fn apply(self, ctx: &Ctx) -> Result<(), Error> {
        for name in super::runner::forget_keys(ctx, &self.session, &self.account).await? {
            Out::note(&format!("removed {name} from the credential store"));
        }
        let approvals = self.approvals.clone();
        let state = ctx.dir.join("runner");
        let shared = !self.shared.is_empty();
        let removed = tokio::task::spawn_blocking(move || {
            let mut gone = Vec::new();
            match std::fs::remove_dir_all(&approvals) {
                Ok(()) => gone.push(approvals),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(format!("remove {}: {e}", approvals.display())),
            }
            if shared {
                gone.extend(hygiene::remove_shared(&state)?);
            }
            Ok(gone)
        })
        .await
        .map_err(|e| Error::Failed(format!("the purge stopped: {e}")))?
        .map_err(Error::Failed)?;
        for p in removed {
            Out::note(&format!("removed {}", p.display()));
        }
        for line in self.lines().into_iter().filter(|l| l.starts_with("keep ")) {
            Out::note(&line);
        }
        Out::note(
            "the runner stays registered with InOrbit, under the same name; this machine no longer has its key",
        );
        Ok(())
    }
}
