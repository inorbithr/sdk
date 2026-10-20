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
use crate::cli::{Global, RunnerCommand};
use crate::context::{Ctx, account as resolve_account, blocking, session};
use crate::error::Error;
use crate::output::Out;
use crate::runner::claude;
use crate::runner::exec::{ClaudeExec, Exec as _};
use crate::runner::provider::{self, AuthState};
use crate::runner::run_loop::{LoopConfig, run_loop};
use crate::runner::service::{HttpService, Registration, RunnerInfo, Service as _};
use crate::runner::signing::{self, Signer};
use iohr_auth::{EntryKey, Redacted};

const KINDS: &[&str] = &["coordinate"];

#[allow(clippy::too_many_lines, reason = "one arm per subcommand")]
pub(crate) async fn run(g: &Global, env: &Env, cmd: RunnerCommand, out: Out) -> Result<(), Error> {
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
            let name = name.unwrap_or_else(host_name);
            let exec = local(g)?;
            let state = exec.auth_status().await;
            if !state.signed_in() {
                return Err(provider_error(&state));
            }
            let svc = HttpService {
                api: &s.api,
                account: account.clone(),
                signer: None,
            };
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
                kinds: KINDS.iter().map(|k| (*k).to_owned()).collect(),
                capacity,
                always_on,
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
            let svc = HttpService {
                api: &s.api,
                account: account.clone(),
                signer: None,
            };
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
            account,
        } => {
            let account = resolve_account(&s, account.account)?;
            let signer = runner_signer(&ctx, &s, &account, false).await?;
            let local_key = signer.public_key();
            let svc = HttpService {
                api: &s.api,
                account: account.clone(),
                signer: Some(signer),
            };
            let all = svc.list_runners(true).await?;
            let me: RunnerInfo = pick_runner(all, runner.as_deref(), name.as_deref(), &local_key)?
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
            Out::note(&format!(
                "runner {} ({}) is running, up to {} turns at once; Ctrl-C stops it",
                me.name,
                me.runner_id,
                me.capacity.clamp(1, 16)
            ));
            let cfg = LoopConfig {
                interval: interval.map(Duration::from_secs),
                max_beats,
                // What it registered with: the service offers no more than this.
                capacity: usize::try_from(me.capacity).unwrap_or(1),
                ..LoopConfig::default()
            };
            let (end, stats) = run_loop(&svc, &Arc::new(exec), &me, cfg, rx, &Out::note).await;
            Out::note(&format!(
                "stopped ({end:?}): {} heartbeats, {} failed, {} leases accepted, {} turns, {} replies delivered",
                stats.beats, stats.failures, stats.accepted, stats.turns, stats.reported
            ));
            Ok(())
        }
    }
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
/// one named after this host.
fn pick_runner(
    all: Vec<RunnerInfo>,
    id: Option<&str>,
    name: Option<&str>,
    local_key: &str,
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
    let host = host_name();
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
            pick_runner(all, None, None, "KEY_A")
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
            pick_runner(all(), Some("a"), None, "KEY_A")
                .unwrap()
                .unwrap()
                .runner_id,
            "a"
        );
        assert_eq!(
            pick_runner(all(), None, Some("one"), "KEY_A")
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
            pick_runner(all, None, None, "KEY_A")
                .unwrap()
                .unwrap()
                .runner_id,
            "h"
        );
        assert!(
            pick_runner(vec![r("a", "x", "")], None, None, "")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn a_key_held_by_more_than_one_runner_is_an_error_not_a_pick() {
        let all = vec![r("a", "one", "KEY_A"), r("b", "two", "KEY_A")];
        let e = pick_runner(all.clone(), None, None, "KEY_A").unwrap_err();
        let msg = format!("{e}");
        assert!(msg.contains("more than one runner") && msg.contains('a') && msg.contains('b'));
        // An explicit id still resolves it.
        assert_eq!(
            pick_runner(all, Some("b"), None, "KEY_A")
                .unwrap()
                .unwrap()
                .runner_id,
            "b"
        );
    }
}
