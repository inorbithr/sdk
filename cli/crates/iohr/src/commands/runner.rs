//! `iohr runner`: this machine's half of a coordinator turn (platform RFC 0120).
//!
//! The runner signs in to InOrbit with the profile's own session, like every command, and
//! to the model provider not at all: it starts the installed `claude`, on the Anthropic
//! API key this machine keeps in the profile's credential store (`iohr runner api-key
//! set`, RFC 0214), given to `claude` alone; or, for personal use only, on the plan the
//! person signed `claude` in to themselves.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::sync::Arc;
use std::time::Duration;

use semver::Version;
use tokio::sync::watch;

use crate::Env;
use crate::cli::SignIn;
use crate::cli::{Global, RunnerApiKeyCommand, RunnerCommand, RunnerConfigCommand};
use crate::commands::session;
use crate::context::{
    Ctx, Prompts, account as resolve_account, blocking, io_session, run_profile, session,
    session_for,
};
use crate::error::Error;
use crate::output::Out;
use crate::runner::approvals::{self, enrol};
use crate::runner::claude;
use crate::runner::drain::Drain;
use crate::runner::exec::{ClaudeExec, Exec as _};
use crate::runner::gate;
use crate::runner::host_action::HostRunner;
use crate::runner::hygiene;
use crate::runner::personal_use;
use crate::runner::provider::{self, AuthState, LocalAuth};
use crate::runner::run_loop::{self, GateCheck, LoopConfig, run_loop};
use crate::runner::self_upgrade::{
    DRAIN_LIMIT, ExecRestarter, FileLedger, Outcome, Policy, RealSource, Upgrader,
};
use crate::runner::service::{HttpService, Registration, RunnerInfo, Service as _};
use crate::runner::settings::{self, BuildFloor, Declared, Effective, Flags};
use crate::runner::signing::{self, RunKey, Signer};
use crate::runner::supervisor::{self, Surroundings};
use crate::runner::workspace::Workspaces;
use iohr_auth::{Config, EntryKey, ProfileName, Redacted};

#[allow(clippy::too_many_lines, reason = "one arm per subcommand")]
pub(crate) async fn run(g: &Global, env: &Env, cmd: RunnerCommand, out: Out) -> Result<(), Error> {
    // Reads only config.toml: no sign-in needed.
    if let RunnerCommand::Config(RunnerConfigCommand::Show {
        capacity,
        kinds,
        no_auto_upgrade,
    }) = cmd
    {
        let ctx = Ctx::load(g)?;
        let profile = run_profile(g, env, &ctx);
        let flags = Flags {
            capacity,
            kinds,
            no_auto_upgrade,
            sign_identity: env.sign_identity.clone(),
            ..Flags::default()
        };
        let e = effective(&ctx.config, profile.as_ref(), &flags)?;
        show_settings(&e, out).await;
        return Ok(());
    }
    // Reads and writes only `runner/personal-use.json`: no sign-in needed.
    if let RunnerCommand::PersonalUse { accept, withdraw } = cmd {
        let ctx = Ctx::load(g)?;
        return personal_use_command(&ctx, accept, withdraw, out);
    }
    // Housekeeping on this machine: no sign-in needed (`uninstall --purge` signs in
    // itself, to name the account whose keys it removes).
    if let RunnerCommand::Prune { dry_run } = cmd {
        let ctx = Ctx::load(g)?;
        let profile = run_profile(g, env, &ctx);
        let flags = Flags {
            sign_identity: env.sign_identity.clone(),
            ..Flags::default()
        };
        let e = effective(&ctx.config, profile.as_ref(), &flags)?;
        return super::runner_ops::prune(&ctx, &e, dry_run, out).await;
    }
    if let RunnerCommand::Service(svc) = cmd {
        let ctx = Ctx::load(g)?;
        return super::runner_ops::service(g, env, &ctx, svc, out).await;
    }
    let ctx = Ctx::load(g)?;
    // A session host (RFC 0213) is a runner of its own: it signs in as the `session`
    // profile, never as the profile the live runner uses (see `host_session`).
    let hosting = matches!(
        cmd,
        RunnerCommand::Run {
            session: Some(_),
            ..
        }
    );
    let s = match g.sign_in {
        Some(SignIn::Io) => io_session(g, &ctx).await?,
        None if hosting => host_session(g, env, &ctx).await?,
        None => session(g, env).await?,
    };
    // `run-key` calls only the approvals API, which checks the person account itself.
    if !matches!(
        cmd,
        RunnerCommand::Status { .. } | RunnerCommand::RunKey { .. } | RunnerCommand::ApiKey(_)
    ) {
        require_scopes(&s)?;
    }
    match cmd {
        RunnerCommand::Register {
            name,
            capacity,
            always_on,
            holder,
            refresh,
            account,
        } => {
            let holder_subject = holder.map(|h| h.trim().to_owned());
            if holder_subject.as_deref() == Some("") {
                return Err(Error::Usage("--holder names a subject".into()));
            }
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
            let exec = with_stored_key(local(g)?, &ctx, &s, &account).await?;
            let state = exec.auth_status().await;
            check_auth(&state, &local_auth(&ctx.dir, &set))?;
            if state == AuthState::Subscription {
                Out::note(OWN_PLAN_NOTE);
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
            // The key alone says this machine already has a runner, whatever name it was
            // registered under: a second one for the same key is never made. A pinned id
            // (`--runner`, `id` in the settings) picks among several.
            let held = held_runner(
                &mine,
                local_key.as_deref(),
                set.id.as_ref().map(|i| i.value.as_str()),
            );
            if let Some(r) = held
                && !refresh
            {
                show_runner(r, out, "already registered");
                pin_note(&r.runner_id, s.profile.as_ref());
                return Ok(());
            }
            let held = held.map(|r| r.runner_id.clone());
            if held.is_none() && mine.iter().any(|r| r.name == name) {
                Out::note(&format!(
                    "a runner named {name} exists but this machine does not hold its key: registering this machine afresh; revoke the old one if it is gone"
                ));
            }
            let signer = runner_signer(&ctx, &s, &account, true).await?;
            // The challenge is one-use and lives five minutes: asked last, signed at once.
            let challenge = svc.challenge().await?;
            // On the person's own plan the sign-in must name its seat (organisation and
            // email): an answer that does not is refused here rather than registered
            // without an identity. An API key names no seat, and the key itself is
            // never hashed or sent.
            let identity = if state == AuthState::Subscription {
                let Some(line) = exec.identity_line().await else {
                    return Err(Error::Failed(
                        "claude auth status names no organisation and email for this sign-in: run `claude`, then `/login`, and register again".into(),
                    ));
                };
                BTreeMap::from([(
                    "claude".to_owned(),
                    provider::identity_sha256(&challenge.identity_salt, &line),
                )])
            } else {
                BTreeMap::new()
            };
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
                holder_subject,
            };
            let r = svc.register(&reg).await?;
            match held {
                Some(id) if id == r.runner_id => {
                    show_runner(&r, out, "refreshed");
                    pin_note(&r.runner_id, s.profile.as_ref());
                }
                Some(id) => {
                    show_runner(&r, out, "registered");
                    Out::note(&format!(
                        "the service made a new runner rather than refreshing {id}: revoke {id} once this one runs"
                    ));
                }
                None => {
                    show_runner(&r, out, "registered");
                    pin_note(&r.runner_id, s.profile.as_ref());
                }
            }
            Ok(())
        }
        RunnerCommand::Status { account } => {
            let account = resolve_account(&s, account.account)?;
            let svc = HttpService::new(&s.api, account.clone(), None);
            let mine = svc.list_runners(true).await?;
            let key_stored = stored_api_key(&ctx, &s, &account).await.unwrap_or_default();
            let api_key_stored = key_stored.is_some();
            let exec = local(g).ok().map(|e| match key_stored {
                Some(k) => e.with_auth(claude::Auth::ApiKey(k)),
                None => e,
            });
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
                        "subscription": *a == AuthState::Subscription,
                        "auth": a.method(), "api_key_stored": api_key_stored,
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
                        AuthState::ApiKey => "on an Anthropic API key".to_owned(),
                        AuthState::Subscription =>
                            "on your own Claude plan (personal use only)".to_owned(),
                        AuthState::SignedOut =>
                            "signed out: store an API key with `iohr runner api-key set`".to_owned(),
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
            no_auto_upgrade,
            session,
            account,
        } => {
            let account = resolve_account(&s, account.account)?;
            // Checked before anything is sent: a bad setting stops the start. A session
            // host (RFC 0213) is started by `iohr session` and ends with it: it never
            // upgrades itself or restarts into a new binary under a person's open terminal.
            let flags = Flags {
                capacity,
                kinds,
                no_auto_upgrade: no_auto_upgrade || session.is_some(),
                sign_identity: env.sign_identity.clone(),
                ..Flags::default()
            };
            let mut set = effective(&ctx.config, s.profile.as_ref(), &flags)?;
            let mut exec = with_stored_key(local(g)?, &ctx, &s, &account).await?;
            let auth = exec.auth_status().await;
            let allowed = local_auth(&ctx.dir, &set);
            check_auth(&auth, &allowed)?;
            let signer = runner_signer(&ctx, &s, &account, false).await?;
            // On the person's own plan, one turn at a time, whatever is configured.
            let flags = if auth == AuthState::Subscription {
                let one = Flags {
                    capacity: Some(1),
                    ..flags
                };
                set = effective(&ctx.config, s.profile.as_ref(), &one)?;
                one
            } else {
                flags
            };
            let local_key = signer.public_key();
            let svc = HttpService::new(&s.api, account.clone(), Some(signer)).holding();
            let all = svc.list_runners(true).await?;
            let host = set
                .name
                .as_ref()
                .map_or_else(host_name, |n| n.value.clone());
            let pinned = runner.or_else(|| set.id.as_ref().map(|i| i.value.clone()));
            let me: RunnerInfo =
                pick_runner(all, pinned.as_deref(), name.as_deref(), &local_key, &host)?
                    .ok_or_else(|| {
                        Error::Failed(
                            "no such runner of yours: run `iohr runner register` first".into(),
                        )
                    })?;
            exec.runner_id.clone_from(&me.runner_id);
            exec.user.clone_from(&me.user);
            let (local_tx, local_rx) = watch::channel(allowed);
            let mut exec = exec.with_local(local_rx);
            exec.workdir = ctx.dir.join("runner").join("coordinate");
            let (tx, rx) = watch::channel(false);
            let tx = Arc::new(tx);
            let on_ctrl_c = tx.clone();
            tokio::spawn(async move {
                let _ = tokio::signal::ctrl_c().await;
                let _ = on_ctrl_c.send(true);
            });
            // The session host (RFC 0213) beside the turns: when it ends (the last session
            // left, `iohr stop`) so does the runner, and SIGTERM stops both.
            let hosted = match &session {
                Some(project) => {
                    let ended = tx.clone();
                    let host = session::start_host(g, project, move || {
                        let _ = ended.send(true);
                    })?;
                    stop_on_sigterm(tx.clone());
                    Some(host)
                }
                None => None,
            };
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
            Out::note(match auth {
                AuthState::ApiKey => "Claude: on an Anthropic API key",
                _ => OWN_PLAN_NOTE,
            });
            let one_at_a_time = auth == AuthState::Subscription;
            let (declare, declared) = watch::channel(declared);
            let (policy, policy_rx) = watch::channel(Policy::of(&set));
            let signing = policy_rx.clone();
            reload_on_hangup(
                ctx.dir.clone(),
                s.profile.clone(),
                flags,
                declare,
                policy,
                local_tx,
            );
            // One drain: the turn loop and the approved-run poller take no new work while
            // an upgrade is waiting to install, and say when nothing is left running.
            let drain = Arc::new(Drain::default());
            let cfg = LoopConfig {
                drain: Some(drain.clone()),
                interval: interval.map(Duration::from_secs),
                max_beats,
                // What it registered with, until a heartbeat declares another; one on
                // the person's own plan.
                capacity: if one_at_a_time {
                    1
                } else {
                    usize::try_from(me.capacity).unwrap_or(1)
                },
                declared: Some(declared),
                build_floor: Some(BuildFloor::default()),
                report_host: true,
                gate: Some(GateCheck {
                    // The pack names the checkout relative to the turns' working directory.
                    workdir: exec.workdir.clone(),
                    ..GateCheck::default()
                }),
                // Job leases get their own checkout, `checkouts/<job_id>` on `job/<id>`,
                // and a replied turn's pull request, through the host's git and `gh`.
                workspaces: Some(Arc::new(Workspaces::for_runner(exec.workdir.clone()))),
                allow_host_git: set.allow_host_git_credentials.value,
                // While it holds no lease, what nothing used for the days kept goes.
                sweep: Some(hygiene::Sweep {
                    state: ctx.dir.join("runner"),
                    retention: hygiene::Retention::of(
                        set.workspace_keep_days.value,
                        set.workspace_max_gb.as_ref().map(|c| c.value),
                    ),
                }),
                ..LoopConfig::default()
            };
            // Approved pinned runs (RFC 0153), beside the turns, when this machine is set
            // up for them; stopped once the turn loop ends.
            let mut asks = approvals_setup(&ctx, &s, &svc, &account, &set).await;
            if let Some(a) = asks.as_mut() {
                a.drain = Some(drain.clone());
            }
            let (stop_asks, asks_stopped) = watch::channel(false);
            let upgrader = Version::parse(env!("CARGO_PKG_VERSION")).ok().map(|current| {
                let supervisor = supervisor::detect(&Surroundings::from_process());
                Out::note(&format!(
                    "auto-upgrade is {} ({}), looking every {} minutes, restart {} under {supervisor}, signing {} ({})",
                    if set.auto_upgrade.value { "on" } else { "off" },
                    set.auto_upgrade.source,
                    set.upgrade_interval.value.as_secs() / 60,
                    set.upgrade_restart.value.as_str(),
                    set.upgrade_sign.value,
                    set.upgrade_sign.source,
                ));
                Upgrader {
                    source: RealSource { policy: signing },
                    ledger: FileLedger {
                        path: ctx.dir.join("runner").join("last-upgrade"),
                    },
                    restarter: ExecRestarter,
                    supervisor,
                    current,
                    policy: policy_rx,
                    drain: drain.clone(),
                    drain_limit: DRAIN_LIMIT,
                    jitter: run_loop::jitter,
                    exec_failed: std::sync::Mutex::default(),
                }
            });
            let upgrade_stopped = asks_stopped.clone();
            let renewal = async {
                match upgrader {
                    Some(u) => {
                        let out = u.run(upgrade_stopped, &Out::note).await;
                        if out == Outcome::Exit {
                            // The turn loop ends; main exits with the restart code.
                            let _ = tx.send(true);
                        }
                        out
                    }
                    None => Outcome::Stopped,
                }
            };
            let exec = Arc::new(exec);
            // Realtime call answers on warm sessions, beside the turns (core RFC 0215).
            let voice = crate::runner::voice::serve(
                &svc,
                &exec,
                &me.runner_id,
                asks_stopped.clone(),
                crate::runner::voice::Pace::default(),
                &Out::note,
            );
            let turns = async {
                let ended = run_loop(&svc, &exec, &me, cfg, rx, &Out::note).await;
                let _ = stop_asks.send(true);
                ended
            };
            let approved = async {
                if let Some(a) = asks.as_mut() {
                    a.serve(&svc, asks_stopped, &Out::note).await;
                }
            };
            let ((end, stats), (), renewed, ()) = tokio::join!(turns, approved, renewal, voice);
            Out::note(&format!(
                "stopped ({end:?}): {} heartbeats, {} failed, {} leases accepted, {} turns, {} replies delivered",
                stats.beats, stats.failures, stats.accepted, stats.turns, stats.reported
            ));
            if let Some(host) = hosted {
                host.finish()?;
            }
            if end == run_loop::End::SignInRefused {
                let who = s
                    .profile
                    .as_ref()
                    .map_or_else(|| "the default".to_owned(), ToString::to_string);
                return Err(Error::NotSignedIn(format!(
                    "sign-in needed for profile {who}: the API refused this runner's sign-in {} times in a row after a fresh token; sign in again (iohr login, or Io's secret), then start the runner",
                    run_loop::REFUSED_BEFORE_EXIT
                )));
            }
            if renewed == Outcome::Exit {
                // A new iohr is installed: the supervisor starts it.
                return Err(Error::Restart);
            }
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
            let set = effective(&ctx.config, s.profile.as_ref(), &Flags::default())?;
            let pinned = runner.or_else(|| set.id.as_ref().map(|i| i.value.clone()));
            let me: RunnerInfo = pick_runner(
                all,
                pinned.as_deref(),
                name.as_deref(),
                &local_key,
                &host_name(),
            )?
            .ok_or_else(|| {
                Error::Failed("no such runner of yours: run `iohr runner register` first".into())
            })?;
            let parent: Vec<(OsString, OsString)> = std::env::vars_os().collect();
            Out::note(&format!("checking {} in {}", command, repo.display()));
            let checked = gate::check(
                &repo,
                &command,
                Duration::from_secs(timeout),
                parent,
                gate::MainFrom::Checkout,
            )
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
                            "output_tail": checked.output_tail,
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
                        if !checked.output_tail.is_empty() {
                            Out::note(&format!(
                                "the check failed; last output (redacted, local only):\n{}",
                                checked.output_tail
                            ));
                        }
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
        RunnerCommand::RunKey {
            proof,
            label,
            wait,
            show,
            account,
        } => {
            let account = resolve_account(&s, account.account)?;
            if show {
                // Read only: no key is made and no ask is filed.
                let key = run_key(&ctx, &s, &account, false).await?.ok_or_else(|| {
                    Error::Failed("this machine has no run key (`iohr runner run-key`)".into())
                })?;
                let public = key.public_key();
                let svc = HttpService::new(&s.api, account.clone(), None);
                let runners = svc.approval_runners().await?;
                let found = approvals::enrolled(&runners, &public);
                let field = |k: &str| {
                    found
                        .and_then(|r| r.get(k))
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned)
                };
                let (id, label) = (field("runner_id"), field("label"));
                if out.json {
                    Out::print_json(&serde_json::json!({
                        "public_key": public, "runner_id": id, "label": label,
                    }));
                    return Ok(());
                }
                Out::pairs(&[
                    ("public_key", public),
                    ("enrolled", id.unwrap_or_else(|| "no".to_owned())),
                    ("label", label.unwrap_or_default()),
                ]);
                return Ok(());
            }
            let key = run_key(&ctx, &s, &account, true)
                .await?
                .ok_or_else(|| Error::Failed("no run key could be made".into()))?;
            let public = key.public_key();
            let label = label.unwrap_or_else(|| format!("iohr runner on {}", host_name()));
            let label = label.trim().to_owned();
            if label.is_empty() || label.chars().count() > 100 {
                return Err(Error::Usage("--label: 1 to 100 characters".into()));
            }
            let svc = HttpService::new(&s.api, account.clone(), None);
            let by_proof = proof.is_some();
            if let Some(path) = proof {
                let text = std::fs::read_to_string(&path).map_err(|e| {
                    Error::Failed(format!("cannot read {}: {}", path.display(), e.kind()))
                })?;
                let v: serde_json::Value = serde_json::from_str(&text)
                    .map_err(|_| Error::Failed(format!("{} is not JSON", path.display())))?;
                let p = approvals::proof_from_json(&v).map_err(Error::Failed)?;
                let message =
                    signing::run_enroll_message(&public, &label, p.signed_at, &p.nonce, &p.key_id);
                let possession = key.sign(message.as_bytes()).map_err(Error::Failed)?;
                svc.enroll_run_key(&public, &label, &p, &possession).await?;
            }
            let runners = svc.approval_runners().await?;
            let mut enrolled = approvals::enrolled(&runners, &public)
                .and_then(|r| r.get("runner_id"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned);
            // RFC 0172, amendment 1: not enrolled yet, so ask on the phone and wait.
            let mut asked = None;
            if enrolled.is_none() && !by_proof {
                let a = enrol::ask(&svc, &key, &label)
                    .await
                    .map_err(Error::Failed)?;
                Out::note(&format!(
                    "approve this run key on your phone: {label} \u{b7} {} (ask {})",
                    a.short, a.approval_id
                ));
                Out::note(&format!(
                    "check the phone shows the same fingerprint before you approve; waiting up to {wait} minutes"
                ));
                let wait = std::time::Duration::from_mins(wait);
                match enrol::wait_and_enroll(&svc, &key, &label, &a, wait, enrol::POLL)
                    .await
                    .map_err(Error::Failed)?
                {
                    enrol::Outcome::Enrolled(id) => enrolled = Some(id),
                    enrol::Outcome::Closed(state) => {
                        return Err(Error::Failed(format!(
                            "the enrolment ask {} is {state}; nothing was enrolled",
                            a.approval_id
                        )));
                    }
                    enrol::Outcome::Waiting => {}
                }
                asked = Some(a);
            }
            let keys_file = approvals::state_dir(&ctx.dir, &account)
                .map_err(Error::Failed)?
                .join("device-keys");
            let keys = approvals::load_device_keys(&keys_file).map_err(Error::Failed)?;
            if out.json {
                Out::print_json(&serde_json::json!({
                    "public_key": public,
                    "label": label,
                    "runner_id": enrolled,
                    "fingerprint": asked.as_ref().map(|a| &a.fingerprint),
                    "approval_id": asked.as_ref().map(|a| &a.approval_id),
                    "device_keys": keys.len(),
                    "device_keys_file": keys_file.display().to_string(),
                }));
                return Ok(());
            }
            Out::pairs(&[
                ("public_key", public),
                ("label", label),
                (
                    "enrolled",
                    enrolled.clone().unwrap_or_else(|| "no".to_owned()),
                ),
                (
                    "device_keys",
                    format!("{} in {}", keys.len(), keys_file.display()),
                ),
            ]);
            if enrolled.is_none() {
                Out::note(
                    "not enrolled yet: the ask is still open on your phone; run this again to keep waiting",
                );
            }
            Ok(())
        }
        RunnerCommand::ApiKey(c) => api_key_command(&ctx, &s, c, out).await,
        RunnerCommand::Config(RunnerConfigCommand::Show { .. })
        | RunnerCommand::PersonalUse { .. }
        | RunnerCommand::Prune { .. }
        | RunnerCommand::Service(_) => Ok(()),
    }
}

/// `iohr runner api-key set|remove`: the Anthropic API key, in the profile's credential
/// store under `{account}_claude_api_key`. Its value is never printed.
async fn api_key_command(
    ctx: &Ctx,
    s: &crate::context::Session,
    cmd: RunnerApiKeyCommand,
    out: Out,
) -> Result<(), Error> {
    const LABEL: &str = "the Anthropic API key";
    match cmd {
        RunnerApiKeyCommand::Set {
            file,
            stdin,
            account,
        } => {
            let account = resolve_account(s, account.account)?;
            let key = match (file, stdin) {
                (Some(path), _) => crate::prompt::secret_from_file(LABEL, &path)?,
                (None, true) => crate::prompt::secret_from_stdin(LABEL)?,
                (None, false) if crate::prompt::interactive() => crate::prompt::secret(LABEL)?,
                (None, false) => {
                    return Err(Error::Usage(
                        "there is no terminal to ask for the key on: pipe it in with `--stdin`, or name a file with `--file PATH`".into(),
                    ));
                }
            };
            if key.expose().chars().any(char::is_whitespace) {
                return Err(Error::Usage(format!(
                    "{LABEL} holds white space: check what was pasted"
                )));
            }
            let stored = StoredKey::open(ctx, s, &account, API_KEY_ENTRY).await?;
            stored.write(key.expose().as_bytes()).await?;
            if out.json {
                Out::print_json(&serde_json::json!({ "api_key_stored": true }));
            } else {
                Out::note(
                    "stored the Anthropic API key in this profile's credential store; restart a running runner to use it",
                );
            }
            Ok(())
        }
        RunnerApiKeyCommand::Remove { account } => {
            let account = resolve_account(s, account.account)?;
            let stored = StoredKey::open(ctx, s, &account, API_KEY_ENTRY).await?;
            let removed = stored.delete().await?;
            if out.json {
                Out::print_json(&serde_json::json!({ "removed": removed }));
            } else if removed {
                Out::note("removed the stored Anthropic API key; restart a running runner");
            } else {
                Out::note("there was no stored Anthropic API key");
            }
            Ok(())
        }
    }
}

/// `iohr runner personal-use`: the notice, and its acceptance on this machine.
fn personal_use_command(ctx: &Ctx, accept: bool, withdraw: bool, out: Out) -> Result<(), Error> {
    if withdraw {
        personal_use::remove(&ctx.dir).map_err(Error::Failed)?;
    } else if accept {
        personal_use::save(&ctx.dir, &personal_use::Accepted::now()).map_err(Error::Failed)?;
    }
    let holds = personal_use::load(&ctx.dir).is_some_and(|a| a.holds(std::time::SystemTime::now()));
    if out.json {
        Out::print_json(&serde_json::json!({
            "notice": personal_use::NOTICE,
            "acknowledgement": personal_use::ACKNOWLEDGEMENT,
            "notice_version": personal_use::NOTICE_VERSION,
            "accepted": holds,
        }));
        return Ok(());
    }
    Out::raw(
        format!(
            "{}\n\n{}\n",
            personal_use::NOTICE,
            personal_use::ACKNOWLEDGEMENT
        )
        .as_bytes(),
    );
    Out::note(match (holds, accept, withdraw) {
        (_, _, true) => "withdrawn: the runner no longer runs on your own Claude plan",
        (true, true, _) => {
            "accepted on this machine for a year, or until the notice changes; it takes `claude_auth = \"any\"` in [runner] as well"
        }
        (true, _, _) => "accepted on this machine",
        (false, _, _) => {
            "not accepted on this machine: `iohr runner personal-use --accept` accepts it"
        }
    });
    Ok(())
}

/// The approvals half of `iohr runner run`, when this machine has a run key the
/// approvals service has enrolled and pins at least one device key; otherwise a note
/// says what is missing and the runner takes turns only.
async fn approvals_setup(
    ctx: &Ctx,
    s: &crate::context::Session,
    svc: &HttpService<'_>,
    account: &str,
    set: &Effective,
) -> Option<approvals::Asks<HostRunner>> {
    let off = |why: &str| {
        Out::note(&format!("approved runs: off, {why}"));
        None
    };
    let key = match run_key(ctx, s, account, false).await {
        Ok(Some(k)) => k,
        Ok(None) => return off("this machine has no run key (`iohr runner run-key`)"),
        Err(e) => return off(&e.to_string()),
    };
    let dir = match approvals::state_dir(&ctx.dir, account) {
        Ok(d) => d,
        Err(e) => return off(&e),
    };
    let keys = match approvals::load_device_keys(&dir.join("device-keys")) {
        Ok(k) if !k.is_empty() => k,
        Ok(_) => {
            return off(&format!(
                "no device key is pinned in {}",
                dir.join("device-keys").display()
            ));
        }
        Err(e) => return off(&e),
    };
    let public = key.public_key();
    let runner_id = match svc.approval_runners().await {
        Ok(all) => match approvals::enrolled(&all, &public) {
            Some(r) => r
                .get("runner_id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            None => return off("the run key is not enrolled (`iohr runner run-key --proof`)"),
        },
        Err(e) => return off(&format!("the approvals runners could not be read: {e}")),
    };
    let seen = match approvals::Seen::load(&dir.join("seen")) {
        Ok(seen) => seen,
        Err(e) => return off(&e),
    };
    let mut tools = HostRunner::find(
        set.kube_context.as_ref().map(|c| c.value.as_str()),
        set.kubeconfig.as_ref().map(|c| c.value.as_str()),
        set.user_units
            .as_ref()
            .map_or(&[][..], |u| u.value.as_slice()),
        set.core_checkout.as_ref().map(|c| c.value.as_str()),
    );
    tools.account = Some(account.to_owned());
    Out::note(&format!(
        "approved runs: on, as {runner_id}, {} device keys pinned{}; user units: {}",
        keys.len(),
        match tools.kube_problem() {
            None => String::new(),
            Some(why) => format!("; {why}, so every Kubernetes action will be refused"),
        },
        if tools.user_units.is_empty() {
            "none".to_owned()
        } else {
            tools.user_units.join(", ")
        }
    ));
    let pin = approvals::Pin {
        runner_id,
        runner_public_key: public,
    };
    Some(approvals::Asks::new(key, pin, keys, seen, tools))
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
    policy: watch::Sender<Policy>,
    local: watch::Sender<LocalAuth>,
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
                    let now = Policy::of(&e);
                    Out::note(&format!(
                        "auto-upgrade {}, every {} minutes, restart {}, signing {}",
                        if now.enabled { "on" } else { "off" },
                        now.interval.as_secs() / 60,
                        now.restart.as_str(),
                        now.sign
                    ));
                    let _ = policy.send(now);
                    let allowed = local_auth(&dir, &e);
                    Out::note(&format!(
                        "claude_auth {}, codex_auth {}, personal-use notice {}",
                        allowed.claude.as_str(),
                        allowed.codex.as_str(),
                        if allowed.personal_use_holds() {
                            "accepted"
                        } else {
                            "not accepted"
                        }
                    ));
                    let _ = local.send(allowed);
                }
                Err(why) => Out::note(&format!(
                    "config.toml was not taken, the settings in use stay: {why}"
                )),
            }
        }
    });
    #[cfg(not(unix))]
    {
        let _ = (dir, profile, flags, declare, policy, local);
    }
}

/// `iohr runner config show`: each value and where it came from.
#[allow(clippy::too_many_lines, reason = "one row per setting")]
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
    let kube = |v: &Option<settings::Setting<String>>| {
        v.as_ref().map_or_else(
            || ("not set".to_owned(), "default".to_owned()),
            |k| (k.value.clone(), k.source.to_string()),
        )
    };
    let (kube_context, kube_context_from) = kube(&e.kube_context);
    let (kubeconfig, kubeconfig_from) = kube(&e.kubeconfig);
    let (core_checkout, core_checkout_from) = kube(&e.core_checkout);
    let (user_units, user_units_from) = e.user_units.as_ref().map_or_else(
        || ("not set".to_owned(), "default".to_owned()),
        |u| (u.value.join(", "), u.source.to_string()),
    );
    let (max_gb, max_gb_from) = e.workspace_max_gb.as_ref().map_or_else(
        || ("not set".to_owned(), "default".to_owned()),
        |c| (format!("{} GB", c.value), c.source.to_string()),
    );
    let volume = floor.volume.display().to_string();
    let free_text = free.map_or_else(|| "unknown".to_owned(), |f| format!("{} GB", gb(f)));
    if out.json {
        Out::print_json(&serde_json::json!({
            "capacity": { "value": e.capacity.as_ref().map(|c| c.value), "source": capacity_from },
            "kinds": { "value": e.kinds.value, "source": e.kinds.source.to_string() },
            "always_on": { "value": e.always_on.value, "source": e.always_on.source.to_string() },
            "name": { "value": name, "source": name_from },
            "kube_context": { "value": e.kube_context.as_ref().map(|k| &k.value), "source": kube_context_from },
            "kubeconfig": { "value": e.kubeconfig.as_ref().map(|k| &k.value), "source": kubeconfig_from },
            "auto_upgrade": { "value": e.auto_upgrade.value, "source": e.auto_upgrade.source.to_string() },
            "upgrade_interval_minutes": { "value": e.upgrade_interval.value.as_secs() / 60, "source": e.upgrade_interval.source.to_string() },
            "upgrade_restart": { "value": e.upgrade_restart.value.as_str(), "source": e.upgrade_restart.source.to_string() },
            "upgrade_sign_identity": { "value": e.upgrade_sign.value.to_string(), "source": e.upgrade_sign.source.to_string() },
            "user_units": { "value": e.user_units.as_ref().map(|u| &u.value), "source": user_units_from },
            "core_checkout": { "value": e.core_checkout.as_ref().map(|c| &c.value), "source": core_checkout_from },
            "claude_auth": { "value": e.claude_auth.value.as_str(), "source": e.claude_auth.source.to_string() },
            "codex_auth": { "value": e.codex_auth.value.as_str(), "source": e.codex_auth.source.to_string() },
            "build_floor": { "volume": volume, "floor_bytes": floor.bytes, "free_bytes": free },
            "allow_host_git_credentials": { "value": e.allow_host_git_credentials.value, "source": e.allow_host_git_credentials.source.to_string() },
            "workspace_keep_days": { "value": e.workspace_keep_days.value, "source": e.workspace_keep_days.source.to_string() },
            "workspace_max_gb": { "value": e.workspace_max_gb.as_ref().map(|c| c.value), "source": max_gb_from },
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
        vec!["kube_context".to_owned(), kube_context, kube_context_from],
        vec!["kubeconfig".to_owned(), kubeconfig, kubeconfig_from],
        vec!["user_units".to_owned(), user_units, user_units_from],
        vec![
            "core_checkout".to_owned(),
            core_checkout,
            core_checkout_from,
        ],
        vec![
            "claude_auth".to_owned(),
            e.claude_auth.value.as_str().to_owned(),
            e.claude_auth.source.to_string(),
        ],
        vec![
            "codex_auth".to_owned(),
            e.codex_auth.value.as_str().to_owned(),
            e.codex_auth.source.to_string(),
        ],
        vec![
            "auto_upgrade".to_owned(),
            e.auto_upgrade.value.to_string(),
            e.auto_upgrade.source.to_string(),
        ],
        vec![
            "upgrade_interval".to_owned(),
            format!("{} minutes", e.upgrade_interval.value.as_secs() / 60),
            e.upgrade_interval.source.to_string(),
        ],
        vec![
            "upgrade_restart".to_owned(),
            e.upgrade_restart.value.as_str().to_owned(),
            e.upgrade_restart.source.to_string(),
        ],
        vec![
            "upgrade_sign_identity".to_owned(),
            e.upgrade_sign.value.to_string(),
            e.upgrade_sign.source.to_string(),
        ],
        vec![
            "build_floor".to_owned(),
            format!(
                "{volume}: {free_text} free, build turns need {} GB",
                gb(floor.bytes)
            ),
            "fixed".to_owned(),
        ],
        vec![
            "allow_host_git_credentials".to_owned(),
            e.allow_host_git_credentials.value.to_string(),
            e.allow_host_git_credentials.source.to_string(),
        ],
        vec![
            "workspace_keep_days".to_owned(),
            format!("{} days", e.workspace_keep_days.value),
            e.workspace_keep_days.source.to_string(),
        ],
        vec!["workspace_max_gb".to_owned(), max_gb, max_gb_from],
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

/// The profile a session host (RFC 0213) signs in as, with its own runner key. The live
/// runner's key sits in its own profile: sharing it would make the two overwrite one
/// heartbeat row and take each other's leases, so the host never reads that profile.
const HOST_PROFILE: &str = "session";

/// What a missing runner key says: the host's own key is made by its own `register`, with the
/// command spelled out; any other profile's by a plain `iohr runner register`.
fn no_key_message(profile: Option<&ProfileName>) -> String {
    if is_host_profile(profile) {
        format!(
            "the session host has no runner key of its own: run `{} --profile {HOST_PROFILE} runner register --name {}-session` once",
            crate::commands::shell::running_bin(),
            host_name()
        )
    } else {
        "this machine has no runner key: run `iohr runner register` first".into()
    }
}

fn is_host_profile(profile: Option<&ProfileName>) -> bool {
    profile.is_some_and(|p| p.as_str() == HOST_PROFILE)
}

/// The session of a session host: the `session` profile whatever `--profile` the person
/// started with (that flag is carried to the host, and would name the live runner's
/// profile). With no such profile the error names the commands that make it.
async fn host_session(g: &Global, env: &Env, ctx: &Ctx) -> Result<crate::context::Session, Error> {
    let name: ProfileName = HOST_PROFILE
        .parse()
        .map_err(|_| Error::Failed("the session host's profile name is not valid".into()))?;
    match session_for(g, env, ctx, Some(&name)).await {
        Err(Error::NotSignedIn(_)) if ctx.config.profile(&name).is_none() => {
            let bin = crate::commands::shell::running_bin();
            Err(Error::NotSignedIn(format!(
                "the session host has no profile of its own: run `{bin} login --profile {HOST_PROFILE}`, then `{bin} --profile {HOST_PROFILE} runner register --name {}-session`",
                host_name()
            )))
        }
        other => other,
    }
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
    let stored = StoredKey::open(ctx, s, account, "runner_key").await?;
    if let Some(seed) = stored.read("the stored runner key is damaged")? {
        return Signer::from_seed(&seed).map_err(Error::Failed);
    }
    if !create {
        return Err(Error::Failed(no_key_message(s.profile.as_ref())));
    }
    let (signer, seed) = Signer::generate().map_err(Error::Failed)?;
    stored.write(&seed).await?;
    Ok(signer)
}

/// This machine's run key (RFC 0153): the P-256 key that signs its reports on approved
/// pinned runs, kept beside the runner key under `{account}_approvals_runner_key`. The
/// approvals service keeps its own registry of these, apart from the coordinator's
/// runners (two registries, to fold into one later).
async fn run_key(
    ctx: &Ctx,
    s: &crate::context::Session,
    account: &str,
    create: bool,
) -> Result<Option<RunKey>, Error> {
    let stored = StoredKey::open(ctx, s, account, "approvals_runner_key").await?;
    if let Some(doc) = stored.read("the stored run key is damaged")? {
        return RunKey::from_pkcs8(&doc).map(Some).map_err(Error::Failed);
    }
    if !create {
        return Ok(None);
    }
    let (key, doc) = RunKey::generate().map_err(Error::Failed)?;
    stored.write(&doc).await?;
    Ok(Some(key))
}

/// A key this machine keeps in the profile's credential store, base64, under
/// `{account}_{name}`; read once on open.
struct StoredKey {
    store: Arc<dyn iohr_auth::Store>,
    key: EntryKey,
    value: Option<Redacted<String>>,
}

/// Removes the runner's keys of `account` (the runner key and the run key) from the
/// profile's credential store; the names of those that were there.
pub(super) async fn forget_keys(
    ctx: &Ctx,
    s: &crate::context::Session,
    account: &str,
) -> Result<Vec<String>, Error> {
    let mut gone = Vec::new();
    for name in ["runner_key", "approvals_runner_key"] {
        let (store, key) = StoredKey::entry(ctx, s, account, name)?;
        if blocking(store, move |st| st.delete(&key)).await? {
            gone.push(format!("{account}_{name}"));
        }
    }
    Ok(gone)
}

impl StoredKey {
    async fn open(
        ctx: &Ctx,
        s: &crate::context::Session,
        account: &str,
        name: &str,
    ) -> Result<Self, Error> {
        let (store, key) = Self::entry(ctx, s, account, name)?;
        let k = key.clone();
        let value = blocking(Arc::clone(&store), move |st| st.get(&k)).await?;
        Ok(Self { store, key, value })
    }

    /// The profile's credential store and the entry's key in it.
    fn entry(
        ctx: &Ctx,
        s: &crate::context::Session,
        account: &str,
        name: &str,
    ) -> Result<(Arc<dyn iohr_auth::Store>, EntryKey), Error> {
        let profile = s.profile.clone().ok_or_else(|| {
            Error::Failed(
                "the runner's keys are kept in your profile's credential store: sign in with `iohr login`, not with IOHR_TOKEN alone".into(),
            )
        })?;
        let storage = ctx
            .config
            .profile(&profile)
            .map(|p| p.storage)
            .ok_or_else(|| Error::Failed(format!("there is no profile {profile}")))?;
        let key = EntryKey::new(profile, &format!("{account}_{name}"))?;
        Ok((ctx.store(storage, Prompts::Allowed)?, key))
    }

    /// The stored bytes, if any.
    fn read(&self, damaged: &str) -> Result<Option<zeroize::Zeroizing<Vec<u8>>>, Error> {
        use base64::Engine as _;
        self.value
            .as_ref()
            .map(|v| {
                base64::engine::general_purpose::STANDARD
                    .decode(v.expose())
                    .map(zeroize::Zeroizing::new)
                    .map_err(|_| Error::Failed(damaged.to_owned()))
            })
            .transpose()
    }

    async fn write(self, bytes: &[u8]) -> Result<(), Error> {
        use base64::Engine as _;
        let secret = Redacted::new(base64::engine::general_purpose::STANDARD.encode(bytes));
        let key = self.key;
        blocking(self.store, move |st| st.set(&key, &secret)).await
    }

    /// Removes the entry; whether there was one.
    async fn delete(self) -> Result<bool, Error> {
        let key = self.key;
        blocking(self.store, move |st| st.delete(&key)).await
    }
}

/// The credential store entry (`{account}_claude_api_key`) of the Anthropic API key.
const API_KEY_ENTRY: &str = "claude_api_key";

/// The Anthropic API key this machine keeps for the account, if one is stored.
async fn stored_api_key(
    ctx: &Ctx,
    s: &crate::context::Session,
    account: &str,
) -> Result<Option<Redacted<String>>, Error> {
    let stored = StoredKey::open(ctx, s, account, API_KEY_ENTRY).await?;
    let Some(bytes) = stored.read("the stored Anthropic API key is damaged")? else {
        return Ok(None);
    };
    std::str::from_utf8(&bytes)
        .map(|k| Some(Redacted::new(k.to_owned())))
        .map_err(|_| Error::Failed("the stored Anthropic API key is damaged".into()))
}

/// What a runner on the person's own Claude plan says when it registers and starts.
const OWN_PLAN_NOTE: &str = "Claude: on your own Claude plan, personal use only: work you assign yourself, one turn at a time";

/// `exec` on the stored API key, when there is one; else on the person's own sign-in.
/// No profile (`IOHR_TOKEN` alone) means no store, and so no key.
async fn with_stored_key(
    exec: ClaudeExec,
    ctx: &Ctx,
    s: &crate::context::Session,
    account: &str,
) -> Result<ClaudeExec, Error> {
    if s.profile.is_none() {
        return Ok(exec);
    }
    Ok(match stored_api_key(ctx, s, account).await? {
        Some(k) => exec.with_auth(claude::Auth::ApiKey(k)),
        None => exec,
    })
}

/// What this machine allows (RFC 0214): the settings' `claude_auth` and `codex_auth`
/// and the personal-use acceptance.
fn local_auth(dir: &std::path::Path, set: &Effective) -> LocalAuth {
    LocalAuth {
        claude: set.auth_policy(claude::PROVIDER),
        codex: set.auth_policy("codex"),
        personal_use: personal_use::load(dir),
    }
}

/// Whether the runner may start on `state`: signed in, and allowed here. The account's
/// policy and who created each job are checked again on every turn.
fn check_auth(state: &AuthState, allowed: &LocalAuth) -> Result<(), Error> {
    if !state.signed_in() {
        return Err(provider_error(state));
    }
    match allowed.refusal(claude::PROVIDER, state, None, ("", "")) {
        Some((_, why)) => Err(Error::Failed(why)),
        None => Ok(()),
    }
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
            "claude has no sign-in the runner can use: store an Anthropic API key with `iohr runner api-key set` (the runner never signs in for you)".into(),
        ),
        AuthState::Refused(why) => Error::Failed(format!(
            "the runner runs on an Anthropic API key, or for personal use on your own Claude plan: {why}. Store a key with `iohr runner api-key set`."
        )),
        AuthState::ApiKey | AuthState::Subscription => Error::Failed("unreachable".into()),
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
                "this machine's key matches more than one runner ({}): pin one with `id = \"rnr_...\"` under [profiles.<name>.runner] in config.toml, or pass --runner <id> (IOHR_RUNNER_ID) or --name; then revoke the extra ones",
                ids.join(", ")
            )));
        }
        return Ok(all.into_iter().nth(first));
    }
    Ok(all.into_iter().find(|r| r.name == host))
}

/// The runner this machine's key already holds, whatever its name: the pinned id when
/// it is one of them, else the first (a pin this key does not hold never makes a second
/// runner for the key). `None` without a key or without a match.
fn held_runner<'a>(
    mine: &'a [RunnerInfo],
    local_key: Option<&str>,
    pinned: Option<&str>,
) -> Option<&'a RunnerInfo> {
    let key = local_key?;
    let held = || {
        mine.iter()
            .filter(move |r| !r.public_key.is_empty() && r.public_key == key)
    };
    pinned
        .and_then(|id| held().find(|r| r.runner_id == id))
        .or_else(|| held().next())
}

/// Says how to pin the runner in config.toml. Nothing here writes `[runner]` settings
/// (the config writer keeps them as read), so the line is given for the person to add.
fn pin_note(id: &str, profile: Option<&ProfileName>) {
    let table = profile.map_or_else(
        || "[runner]".to_owned(),
        |p| format!("[profiles.{p}.runner]"),
    );
    Out::note(&format!(
        "to pin this runner, set id = \"{id}\" under {table} in config.toml"
    ));
}

pub(super) fn host_name() -> String {
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

/// Stops the runner on SIGTERM, which a session host gets from `kill` and from a supervisor.
#[cfg(unix)]
fn stop_on_sigterm(tx: Arc<watch::Sender<bool>>) {
    use tokio::signal::unix::{SignalKind, signal};
    if let Ok(mut term) = signal(SignalKind::terminate()) {
        tokio::spawn(async move {
            term.recv().await;
            let _ = tx.send(true);
        });
    }
}

#[cfg(not(unix))]
fn stop_on_sigterm(_: Arc<watch::Sender<bool>>) {}

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

    /// The session host signs in as the `session` profile and keeps its key there: a missing
    /// key names the one command that makes it, and no other profile's message does.
    #[test]
    fn the_session_hosts_missing_key_names_the_register_command_for_its_own_profile() {
        let own: ProfileName = HOST_PROFILE.parse().unwrap();
        let live: ProfileName = "runner".parse().unwrap();
        let said = no_key_message(Some(&own));
        assert!(
            said.contains("--profile session runner register --name "),
            "{said}"
        );
        assert!(said.contains("-session` once"), "{said}");
        assert!(is_host_profile(Some(&own)));
        // The live runner's profile is never the host's, and says what it always did.
        assert!(!is_host_profile(Some(&live)));
        assert!(!is_host_profile(None));
        assert_eq!(
            no_key_message(Some(&live)),
            "this machine has no runner key: run `iohr runner register` first"
        );
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
    fn register_finds_the_runner_by_key_whatever_its_name() {
        let mine = vec![r("a", "other", "KEY_B"), r("b", "io-name", "KEY_A")];
        assert_eq!(
            held_runner(&mine, Some("KEY_A"), None).unwrap().runner_id,
            "b"
        );
        assert!(held_runner(&mine, None, None).is_none());
        let two = vec![r("a", "x", "KEY_A"), r("b", "x", "KEY_A")];
        assert_eq!(
            held_runner(&two, Some("KEY_A"), Some("b"))
                .unwrap()
                .runner_id,
            "b"
        );
        // A pin this key does not hold falls back to the key's runner, never a new one.
        assert_eq!(
            held_runner(&two, Some("KEY_A"), Some("zz"))
                .unwrap()
                .runner_id,
            "a"
        );
    }

    #[test]
    fn a_key_held_by_more_than_one_runner_is_an_error_not_a_pick() {
        let all = vec![r("a", "one", "KEY_A"), r("b", "two", "KEY_A")];
        let e = pick_runner(all.clone(), None, None, "KEY_A", &host_name()).unwrap_err();
        let msg = format!("{e}");
        assert!(
            msg.contains("more than one runner")
                && msg.contains("[profiles.<name>.runner]")
                && msg.contains("revoke the extra ones")
                && msg.contains("a, b")
        );
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
