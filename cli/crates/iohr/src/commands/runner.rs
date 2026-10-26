//! `iohr runner`: this machine's half of a coordinator turn (platform RFC 0120).
//!
//! The runner signs in to InOrbit with the profile's own session, like every command, and
//! to the model provider not at all: it starts the installed `claude`, which the person
//! signed in themselves.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::watch;

use crate::Env;
use crate::cli::{Global, RunnerCommand, RunnerConfigCommand};
use crate::context::{Ctx, account as resolve_account, blocking, run_profile, session};
use crate::error::Error;
use crate::output::Out;
use crate::runner::claude;
use crate::runner::exec::{ClaudeExec, Exec as _};
use crate::runner::gate;
use crate::runner::provider::{self, AuthState};
use crate::runner::run_loop::{GateCheck, LoopConfig, run_loop};
use crate::runner::service::{HttpService, Registration, RunnerInfo, Service as _};
use crate::runner::settings::{self, BuildFloor, Declared, Effective, Flags};
use crate::runner::signing::{self, Signer};
use crate::runner::workspace::Workspaces;
use iohr_auth::{Config, EntryKey, ProfileName, Redacted};

#[allow(clippy::too_many_lines, reason = "one arm per subcommand")]
pub(crate) async fn run(g: &Global, env: &Env, cmd: RunnerCommand, out: Out) -> Result<(), Error> {
    // Reads only config.toml: no sign-in needed.
    if let RunnerCommand::Config(RunnerConfigCommand::Show { capacity, kinds }) = cmd {
        let ctx = Ctx::load(g)?;
        let profile = run_profile(g, env, &ctx);
        let flags = Flags {
            capacity,
            kinds,
            ..Flags::default()
        };
        let e = effective(&ctx.config, profile.as_ref(), &flags)?;
        show_settings(&e, out).await;
        return Ok(());
    }
    let s = session(g, env).await?;
    let ctx = Ctx::load(g)?;
    if !matches!(cmd, RunnerCommand::Status { .. }) {
        require_scopes(&s)?;
    }
    match cmd {
        RunnerCommand::Register {
            name,
            capacity,
            always_on,
            account,
        } => {
            let account = resolve_account(&s, account.account)?;
            let flags = Flags {
                capacity,
                always_on,
                name,
                ..Flags::default()
            };
            let set = effective(&ctx.config, s.profile.as_ref(), &flags)?;
            let name = set
                .name
                .as_ref()
                .map_or_else(host_name, |n| n.value.clone());
            let exec = local(g)?;
            let state = exec.auth_status().await;
            if !state.signed_in() {
                return Err(provider_error(&state));
            }
            let svc = HttpService::new(&s.api, account.clone(), None);
            let mine = svc.list_runners(true).await?;
            // A name is only "already registered" when this machine holds that runner's
            // key. A lost key, or a runner registered from another machine, registers
            // afresh (a new runner; the old one can be revoked) instead of being stuck.
            let local_key = match runner_signer(&ctx, &s, &account, false).await {
                Ok(k) => Some(k.public_key()),
                Err(_) => None,
            };
            if let Some(r) = mine.iter().find(|r| {
                r.name == name
                    && !r.public_key.is_empty()
                    && local_key.as_deref() == Some(&r.public_key)
            }) {
                show_runner(r, out, "already registered");
                return Ok(());
            }
            if mine.iter().any(|r| r.name == name) {
                Out::note(&format!(
                    "a runner named {name} exists but this machine does not hold its key: registering this machine afresh; revoke the old one if it is gone"
                ));
            }
            let signer = runner_signer(&ctx, &s, &account, true).await?;
            // The challenge is one-use and lives five minutes: asked last, signed at once.
            let challenge = svc.challenge().await?;
            let mut identity = BTreeMap::new();
            if let Some(line) = exec.identity_line().await {
                identity.insert(
                    "claude".to_owned(),
                    provider::identity_sha256(&challenge.identity_salt, &line),
                );
            }
            let public_key = signer.public_key();
            let signature = signer
                .sign(signing::registration_message(&challenge.challenge, &public_key).as_bytes());
            let reg = Registration {
                name,
                os: std::env::consts::OS.into(),
                arch: std::env::consts::ARCH.into(),
                version: env!("CARGO_PKG_VERSION").into(),
                kinds: set.kinds.value.clone(),
                capacity: set.capacity.as_ref().map_or(1, |c| c.value),
                always_on: set.always_on.value,
                providers: exec.providers().await,
                identity_sha256: identity,
                public_key,
                challenge: challenge.challenge,
                signature,
            };
            let r = svc.register(&reg).await?;
            show_runner(&r, out, "registered");
            Ok(())
        }
        RunnerCommand::Status { account } => {
            let account = resolve_account(&s, account.account)?;
            let svc = HttpService::new(&s.api, account.clone(), None);
            let mine = svc.list_runners(true).await?;
            let exec = local(g).ok();
            let local_state = match &exec {
                Some(e) => Some((
                    e.program.display().to_string(),
                    e.version().await,
                    e.auth_status().await,
                )),
                None => None,
            };
            if out.json {
                Out::print_json(&serde_json::json!({
                    "runners": mine.iter().map(|r| serde_json::json!({
                        "runner_id": r.runner_id, "name": r.name, "status": r.status,
                        "last_seen_at": r.last_seen_at, "providers": r.providers,
                        "capacity": r.capacity,
                    })).collect::<Vec<_>>(),
                    "this_machine": local_state.as_ref().map(|(p, v, a)| serde_json::json!({
                        "name": host_name(), "claude": p, "version": v,
                        "subscription": a.signed_in(),
                    })),
                }));
                return Ok(());
            }
            let rows: Vec<Vec<String>> = mine
                .iter()
                .map(|r| {
                    vec![
                        r.runner_id.clone(),
                        r.name.clone(),
                        r.status.clone(),
                        crate::commands::day(&r.last_seen_at),
                        r.providers.join(", "),
                    ]
                })
                .collect();
            Out::table(&["RUNNER", "NAME", "STATUS", "SEEN", "PROVIDERS"], &rows);
            match local_state {
                Some((path, version, auth)) => Out::note(&format!(
                    "this machine ({}): claude {path} {version}; {}",
                    host_name(),
                    match auth {
                        AuthState::Subscription => "signed in on the subscription".to_owned(),
                        AuthState::SignedOut => "signed out: run claude, then /login".to_owned(),
                        AuthState::Refused(why) => format!("refused: {why}"),
                    }
                )),
                None => Out::note("this machine: claude was not found on PATH"),
            }
            Ok(())
        }
        RunnerCommand::Run {
            name,
            runner,
            interval,
            max_beats,
            capacity,
            kinds,
            account,
        } => {
            let account = resolve_account(&s, account.account)?;
            // Checked before anything is sent: a bad setting stops the start.
            let flags = Flags {
                capacity,
                kinds,
                ..Flags::default()
            };
            let set = effective(&ctx.config, s.profile.as_ref(), &flags)?;
            let signer = runner_signer(&ctx, &s, &account, false).await?;
            let local_key = signer.public_key();
            let svc = HttpService::new(&s.api, account.clone(), Some(signer));
            let all = svc.list_runners(true).await?;
            let host = set
                .name
                .as_ref()
                .map_or_else(host_name, |n| n.value.clone());
            let me: RunnerInfo =
                pick_runner(all, runner.as_deref(), name.as_deref(), &local_key, &host)?
                    .ok_or_else(|| {
                        Error::Failed(
                            "no such runner of yours: run `iohr runner register` first".into(),
                        )
                    })?;
            let mut exec = local(g)?;
            exec.runner_id.clone_from(&me.runner_id);
            exec.user.clone_from(&me.user);
            let auth = exec.auth_status().await;
            if !auth.signed_in() {
                return Err(provider_error(&auth));
            }
            exec.workdir = ctx.dir.join("runner").join("coordinate");
            let (tx, rx) = watch::channel(false);
            tokio::spawn(async move {
                let _ = tokio::signal::ctrl_c().await;
                let _ = tx.send(true);
            });
            let declared = Declared::of(&set);
            let at_once = declared
                .capacity
                .unwrap_or(me.capacity)
                .clamp(1, settings::MAX_CAPACITY);
            Out::note(&format!(
                "runner {} ({}) is running, up to {at_once} turns at once, taking {}; Ctrl-C stops it, SIGHUP reads config.toml again",
                me.name,
                me.runner_id,
                declared.kinds.join(", ")
            ));
            let (declare, declared) = watch::channel(declared);
            reload_on_hangup(ctx.dir.clone(), s.profile.clone(), flags, declare);
            let cfg = LoopConfig {
                interval: interval.map(Duration::from_secs),
                max_beats,
                // What it registered with, until a heartbeat declares another.
                capacity: usize::try_from(me.capacity).unwrap_or(1),
                declared: Some(declared),
                build_floor: Some(BuildFloor::default()),
                // The PATH the workers and gates get (env.rs passes it through); an
                // unset PATH finds nothing, so every tool is reported missing.
                tools_path: Some(std::env::var_os("PATH").unwrap_or_default()),
                gate: Some(GateCheck {
                    // The pack names the checkout relative to the turns' working directory.
                    workdir: exec.workdir.clone(),
                    ..GateCheck::default()
                }),
                // Job leases get their own checkout, `checkouts/<job_id>` on `job/<id>`,
                // and a replied turn's pull request, through the host's git and `gh`.
                workspaces: Some(Arc::new(Workspaces::for_runner(exec.workdir.clone()))),
                ..LoopConfig::default()
            };
            let (end, stats) = run_loop(&svc, &Arc::new(exec), &me, cfg, rx, &Out::note).await;
            Out::note(&format!(
                "stopped ({end:?}): {} heartbeats, {} failed, {} leases accepted, {} turns, {} replies delivered",
                stats.beats, stats.failures, stats.accepted, stats.turns, stats.reported
            ));
            Ok(())
        }
        RunnerCommand::GateReport {
            job,
            lease,
            token,
            repo,
            command,
            timeout,
            name,
            runner,
            account,
        } => {
            let account = resolve_account(&s, account.account)?;
            let signer = runner_signer(&ctx, &s, &account, false).await?;
            let local_key = signer.public_key();
            let svc = HttpService::new(&s.api, account.clone(), Some(signer));
            let all = svc.list_runners(true).await?;
            let me: RunnerInfo = pick_runner(
                all,
                runner.as_deref(),
                name.as_deref(),
                &local_key,
                &host_name(),
            )?
            .ok_or_else(|| {
                Error::Failed("no such runner of yours: run `iohr runner register` first".into())
            })?;
            let parent: Vec<(OsString, OsString)> = std::env::vars_os().collect();
            Out::note(&format!("checking {} in {}", command, repo.display()));
            let checked = gate::check(&repo, &command, Duration::from_secs(timeout), parent)
                .await
                .map_err(|e| Error::Failed(format!("no report was sent: {e}")))?;
            let report = gate::report_of(
                gate::Held {
                    job_id: &job,
                    lease_id: &lease,
                    fencing_token: token,
                    runner_id: &me.runner_id,
                },
                &checked,
            );
            match gate::send(&svc, &report).await {
                Ok(Some(a)) => {
                    if out.json {
                        Out::print_json(&serde_json::json!({
                            "job_id": report.job_id, "command": checked.command,
                            "exit_code": checked.exit_code, "head": checked.head,
                            "main_commit": checked.main_commit,
                            "output_sha256": checked.output_sha256,
                            "evidence_id": a.evidence_id, "duplicate": a.duplicate,
                            "stage": a.stage,
                        }));
                    } else {
                        Out::pairs(&[
                            ("command", checked.command.clone()),
                            ("exit", checked.exit_code.to_string()),
                            ("head", short(&checked.head).to_owned()),
                            ("main", short(&checked.main_commit).to_owned()),
                            ("evidence", a.evidence_id.clone()),
                            ("duplicate", a.duplicate.to_string()),
                            ("job is now", a.stage.clone()),
                        ]);
                    }
                    Ok(())
                }
                // An older coordinator has no such call: say so, and do not fail the caller.
                Ok(None) => {
                    Out::note(
                        "this coordinator does not take gate reports yet; nothing was recorded",
                    );
                    Ok(())
                }
                Err(e) => Err(e.into()),
            }
        }
        RunnerCommand::Config(RunnerConfigCommand::Show { .. }) => Ok(()),
    }
}

/// The runner settings in effect: flags over `[profiles.<name>.runner]` over `[runner]`.
fn effective(
    config: &Config,
    profile: Option<&ProfileName>,
    flags: &Flags,
) -> Result<Effective, Error> {
    let own = profile.and_then(|p| {
        config
            .profile(p)
            .and_then(|pr| pr.runner.as_ref())
            .map(|r| (p.as_str(), r))
    });
    settings::resolve(&config.runner, own, flags).map_err(Error::Failed)
}

/// On SIGHUP, reads config.toml again and sends what the next heartbeat declares. A file
/// that cannot be read, or a bad value, keeps the settings in use and says why.
fn reload_on_hangup(
    dir: std::path::PathBuf,
    profile: Option<ProfileName>,
    flags: Flags,
    declare: watch::Sender<Declared>,
) {
    #[cfg(unix)]
    tokio::spawn(async move {
        use tokio::signal::unix::{SignalKind, signal};
        let Ok(mut hup) = signal(SignalKind::hangup()) else {
            Out::note(
                "SIGHUP cannot be watched here: restart the runner to read config.toml again",
            );
            return;
        };
        while hup.recv().await.is_some() {
            let read = Config::load(&dir)
                .map_err(|e| e.to_string())
                .and_then(|c| effective(&c, profile.as_ref(), &flags).map_err(|e| e.to_string()));
            match read {
                Ok(e) => {
                    let d = Declared::of(&e);
                    Out::note(&format!(
                        "config.toml read again: capacity {}, kinds {}",
                        d.capacity
                            .map_or_else(|| "as registered".to_owned(), |c| c.to_string()),
                        d.kinds.join(", ")
                    ));
                    let _ = declare.send(d);
                }
                Err(why) => Out::note(&format!(
                    "config.toml was not taken, the settings in use stay: {why}"
                )),
            }
        }
    });
    #[cfg(not(unix))]
    {
        let _ = (dir, profile, flags, declare);
    }
}

/// `iohr runner config show`: each value and where it came from.
async fn show_settings(e: &Effective, out: Out) {
    let floor = BuildFloor::default();
    let free = settings::free_bytes(&floor.volume).await;
    let gb = |b: u64| b / 1_000_000_000;
    let (capacity, capacity_from) = e.capacity.as_ref().map_or_else(
        || ("as registered".to_owned(), "registration".to_owned()),
        |c| (c.value.to_string(), c.source.to_string()),
    );
    let (name, name_from) = e.name.as_ref().map_or_else(
        || (host_name(), "host name".to_owned()),
        |n| (n.value.clone(), n.source.to_string()),
    );
    let volume = floor.volume.display().to_string();
    let free_text = free.map_or_else(|| "unknown".to_owned(), |f| format!("{} GB", gb(f)));
    if out.json {
        Out::print_json(&serde_json::json!({
            "capacity": { "value": e.capacity.as_ref().map(|c| c.value), "source": capacity_from },
            "kinds": { "value": e.kinds.value, "source": e.kinds.source.to_string() },
            "always_on": { "value": e.always_on.value, "source": e.always_on.source.to_string() },
            "name": { "value": name, "source": name_from },
            "build_floor": { "volume": volume, "floor_bytes": floor.bytes, "free_bytes": free },
        }));
        return;
    }
    let rows = vec![
        vec!["capacity".to_owned(), capacity, capacity_from],
        vec![
            "kinds".to_owned(),
            e.kinds.value.join(", "),
            e.kinds.source.to_string(),
        ],
        vec![
            "always_on".to_owned(),
            e.always_on.value.to_string(),
            e.always_on.source.to_string(),
        ],
        vec!["name".to_owned(), name, name_from],
        vec![
            "build_floor".to_owned(),
            format!(
                "{volume}: {free_text} free, build turns need {} GB",
                gb(floor.bytes)
            ),
            "fixed".to_owned(),
        ],
    ];
    Out::table(&["KEY", "VALUE", "SOURCE"], &rows);
}

/// The first 12 characters of a commit.
fn short(sha: &str) -> &str {
    sha.get(..12).unwrap_or(sha)
}

/// The scopes a runner signs in with.
const SCOPES: &[&str] = &["coordinator:runner", "coordinator:read"];

/// The runner's calls need `coordinator:runner coordinator:read`.
fn require_scopes(s: &crate::context::Session) -> Result<(), Error> {
    let missing: Vec<&str> = SCOPES
        .iter()
        .copied()
        .filter(|w| !s.claims.scp.iter().any(|h| h == w))
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    Err(Error::Failed(format!(
        "this sign-in lacks {}: sign in with `iohr login --scope {}`",
        missing.join(", "),
        SCOPES.join(",")
    )))
}

/// The runner's Ed25519 key, from the credential store the profile uses (the macOS
/// Keychain on a Mac; a file only when the profile was made with `--insecure-storage`).
/// `create` makes one when there is none; otherwise a missing key says to register.
async fn runner_signer(
    ctx: &Ctx,
    s: &crate::context::Session,
    account: &str,
    create: bool,
) -> Result<Signer, Error> {
    use base64::Engine as _;
    let name = s.profile.clone().ok_or_else(|| {
        Error::Failed(
            "the runner's key is kept in your profile's credential store: sign in with `iohr login`, not with IOHR_TOKEN alone".into(),
        )
    })?;
    let storage = ctx
        .config
        .profile(&name)
        .map(|p| p.storage)
        .ok_or_else(|| Error::Failed(format!("there is no profile {name}")))?;
    let key = EntryKey::new(name, &format!("{account}_runner_key"))?;
    let store = ctx.store(storage)?;
    let k = key.clone();
    let seed: Option<Redacted<String>> = blocking(Arc::clone(&store), move |st| st.get(&k)).await?;
    let b64 = base64::engine::general_purpose::STANDARD;
    if let Some(seed) = seed {
        let bytes = b64
            .decode(seed.expose())
            .map_err(|_| Error::Failed("the stored runner key is damaged".into()))?;
        return Signer::from_seed(&bytes).map_err(Error::Failed);
    }
    if !create {
        return Err(Error::Failed(
            "this machine has no runner key: run `iohr runner register` first".into(),
        ));
    }
    let (signer, seed) = Signer::generate().map_err(Error::Failed)?;
    let secret = Redacted::new(b64.encode(seed));
    blocking(store, move |st| st.set(&key, &secret)).await?;
    Ok(signer)
}

/// `claude` as installed: the first one on the user's `PATH`.
fn local(g: &Global) -> Result<ClaudeExec, Error> {
    let parent: Vec<(OsString, OsString)> = std::env::vars_os().collect();
    let path = std::env::var_os("PATH");
    let program = provider::find_on_path(claude_name(), path.as_ref()).ok_or_else(|| {
        Error::Failed("claude was not found on PATH: install Claude Code and sign in with `claude`, then `/login`".into())
    })?;
    let claude_dir = claude::claude_dir(
        std::env::var_os("CLAUDE_CONFIG_DIR").map(Into::into),
        etcetera::home_dir().ok(),
    );
    let _ = g;
    Ok(ClaudeExec::new(
        program,
        std::env::temp_dir(),
        parent,
        claude_dir,
        String::new(),
        String::new(),
    ))
}

const fn claude_name() -> &'static str {
    if cfg!(windows) {
        "claude.exe"
    } else {
        "claude"
    }
}

fn provider_error(state: &AuthState) -> Error {
    match state {
        AuthState::SignedOut => Error::Failed(
            "claude is signed out: run `claude`, then `/login` (the subscription; the runner never signs in for you)".into(),
        ),
        AuthState::Refused(why) => Error::Failed(format!(
            "the runner runs on the subscription only: {why}. Sign in again with `claude`, then `/login`."
        )),
        AuthState::Subscription => Error::Failed("unreachable".into()),
    }
}

/// The runner `run` drives: the one named by `--runner` (id) or `--name`; else the one
/// whose public key this machine holds (whatever name it was registered under); else the
/// one named `host` (the configured name, else this host's).
fn pick_runner(
    all: Vec<RunnerInfo>,
    id: Option<&str>,
    name: Option<&str>,
    local_key: &str,
    host: &str,
) -> Result<Option<RunnerInfo>, Error> {
    if let Some(id) = id {
        return Ok(all.into_iter().find(|r| r.runner_id == id));
    }
    if let Some(name) = name {
        return Ok(all.into_iter().find(|r| r.name == name));
    }
    let mut by_key = all
        .iter()
        .enumerate()
        .filter(|(_, r)| !r.public_key.is_empty() && r.public_key == local_key)
        .map(|(i, _)| i);
    if let Some(first) = by_key.next() {
        if by_key.next().is_some() {
            let ids: Vec<&str> = all
                .iter()
                .filter(|r| !r.public_key.is_empty() && r.public_key == local_key)
                .map(|r| r.runner_id.as_str())
                .collect();
            return Err(Error::Failed(format!(
                "this machine's key matches more than one runner ({}): pass --runner <id> or --name",
                ids.join(", ")
            )));
        }
        return Ok(all.into_iter().nth(first));
    }
    Ok(all.into_iter().find(|r| r.name == host))
}

fn host_name() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .ok()
        .filter(|h| !h.trim().is_empty())
        .or_else(|| {
            std::process::Command::new("hostname")
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
                .filter(|h| !h.is_empty())
        })
        .unwrap_or_else(|| "runner".into())
}

fn show_runner(r: &RunnerInfo, out: Out, what: &str) {
    if out.json {
        Out::print_json(&serde_json::json!({
            "result": what, "runner_id": r.runner_id, "name": r.name, "status": r.status,
        }));
    } else {
        Out::pairs(&[
            ("runner", r.runner_id.clone()),
            ("name", r.name.clone()),
            ("status", format!("{} ({what})", r.status)),
        ]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(id: &str, name: &str, key: &str) -> RunnerInfo {
        RunnerInfo {
            runner_id: id.into(),
            name: name.into(),
            public_key: key.into(),
            ..RunnerInfo::default()
        }
    }

    #[test]
    fn a_custom_name_is_found_by_the_key_this_machine_holds() {
        let all = vec![r("a", "other", "KEY_B"), r("b", "my-custom-name", "KEY_A")];
        assert_eq!(
            pick_runner(all, None, None, "KEY_A", &host_name())
                .unwrap()
                .unwrap()
                .runner_id,
            "b"
        );
    }

    #[test]
    fn explicit_id_and_name_win_over_the_key() {
        let all = || vec![r("a", "one", "KEY_B"), r("b", "two", "KEY_A")];
        assert_eq!(
            pick_runner(all(), Some("a"), None, "KEY_A", &host_name())
                .unwrap()
                .unwrap()
                .runner_id,
            "a"
        );
        assert_eq!(
            pick_runner(all(), None, Some("one"), "KEY_A", &host_name())
                .unwrap()
                .unwrap()
                .runner_id,
            "a"
        );
    }

    #[test]
    fn falls_back_to_the_hostname_without_a_key_match() {
        let all = vec![r("a", "x", "KEY_B"), r("h", &host_name(), "")];
        assert_eq!(
            pick_runner(all, None, None, "KEY_A", &host_name())
                .unwrap()
                .unwrap()
                .runner_id,
            "h"
        );
        assert!(
            pick_runner(vec![r("a", "x", "")], None, None, "", &host_name())
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn a_key_held_by_more_than_one_runner_is_an_error_not_a_pick() {
        let all = vec![r("a", "one", "KEY_A"), r("b", "two", "KEY_A")];
        let e = pick_runner(all.clone(), None, None, "KEY_A", &host_name()).unwrap_err();
        let msg = format!("{e}");
        assert!(msg.contains("more than one runner") && msg.contains('a') && msg.contains('b'));
        // An explicit id still resolves it.
        assert_eq!(
            pick_runner(all, Some("b"), None, "KEY_A", &host_name())
                .unwrap()
                .unwrap()
                .runner_id,
            "b"
        );
    }
}
