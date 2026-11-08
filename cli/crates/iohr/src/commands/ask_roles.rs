//! `?` and `??` go to the Roles too (RFC 0213, S2).
//!
//! When a session host runs for the folder, an ask is put in its durable queue once per Role
//! before Io starts answering, so every Role works while Io does. Io answers as it always
//! did, in the one card [`super::ask`] draws; after it, each Role's answer is drawn as its
//! own card the moment it lands, with the same renderer ([`crate::card`]) and the same
//! `◆ Io` header, the Role named in the meta. With no host, or a host that does not know the
//! request, nothing here runs and the ask is exactly what it was.
//!
//! What a Role answers is read-only: the turn runs in `manual` mode (or `plan` when asked
//! for), never `auto`, so Roles that answer side by side never edit the folder at once.

pub(crate) use imp::{plan, start};

#[cfg(unix)]
mod imp {
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};

    use crate::card::{self, Body, Card, Look};
    use crate::cli::{AskArgs, Global};
    use crate::commands::session;
    use crate::output::Out;
    use crate::session::lifecycle::Paths;
    use crate::session::route::{self, Landed, Outcome, Payload};
    use crate::session::turn::ROUTED_ENV;
    use crate::ui::Ui;

    /// How often the answers are read back.
    const POLL: Duration = Duration::from_millis(400);
    /// How long the card waits for the Roles before it leaves them to finish in the background.
    const WAIT: Duration = Duration::from_secs(180);

    /// What is needed to route an ask, before any socket is touched.
    pub(crate) struct Plan {
        data: PathBuf,
        cwd: PathBuf,
        named: Option<String>,
        payload: Payload,
    }

    /// An ask in the host's queue.
    pub(crate) struct Routed {
        paths: Paths,
        ask: String,
    }

    /// The Roles that answer: all the host runs, or the ones `--role` names. `None` when a
    /// named one is not hosted (then the ask goes as it always did, `--role` and all).
    pub(crate) fn chosen(named: Option<&str>, hosted: &[String]) -> Option<Vec<String>> {
        let Some(named) = named else {
            return Some(hosted.to_vec());
        };
        let mut picked: Vec<String> = Vec::new();
        for name in named.split(',').map(str::trim).filter(|n| !n.is_empty()) {
            if !hosted.iter().any(|h| h == name) {
                return None;
            }
            if !picked.iter().any(|p| p == name) {
                picked.push(name.to_owned());
            }
        }
        (!picked.is_empty()).then_some(picked)
    }

    /// Whether an ask may go to the Roles: a question asked from the card or the hook, which
    /// changes nothing and is not already a Role's own turn.
    pub(crate) fn eligible(args: &AskArgs, json: bool, in_a_turn: bool) -> bool {
        let h = &args.headless;
        !in_a_turn
            && !json
            && !args.menu
            && !args.dry_run
            && !args.allow_edits
            && (h.hook || h.card)
            && args.mode.as_deref() != Some("auto")
            && args.question.iter().any(|w| !w.trim().is_empty())
    }

    /// Decides whether this ask goes to the Roles; no socket is touched.
    pub(crate) fn plan(g: &Global, args: &AskArgs, json: bool) -> Option<Plan> {
        let in_a_turn = std::env::var_os(ROUTED_ENV).is_some();
        if !eligible(args, json, in_a_turn) {
            return None;
        }
        let data = session::data_dir(g).ok()?;
        let cwd = args
            .headless
            .cwd
            .as_ref()
            .map_or_else(|| std::env::current_dir().ok(), |c| Some(PathBuf::from(c)))?;
        let cwd = std::path::absolute(&cwd).unwrap_or(cwd);
        let mut payload = Payload::new(
            &route::new_ask_id(),
            &args.question.join(" "),
            &cwd.display().to_string(),
        );
        // Roles read and answer; a Role that edits beside the others is never asked for.
        payload.mode = Some(
            if args.mode.as_deref() == Some("plan") {
                "plan"
            } else {
                "manual"
            }
            .to_owned(),
        );
        payload.model.clone_from(&args.model);
        payload.agent.clone_from(&args.agent);
        payload.job.clone_from(&args.job);
        Some(Plan {
            data,
            cwd,
            named: args.role.clone(),
            payload,
        })
    }

    /// Finds the host and queues the ask for each Role; `None` when there is no host to
    /// answer it (the ask then goes as it always did).
    pub(crate) async fn start(plan: Plan) -> Option<Routed> {
        tokio::task::spawn_blocking(move || connect(&plan))
            .await
            .ok()
            .flatten()
    }

    fn connect(plan: &Plan) -> Option<Routed> {
        let paths = route::find_host(&plan.cwd, &plan.data)?;
        let hosted = route::hosted_roles(&paths)?;
        let roles = chosen(plan.named.as_deref(), &hosted)?;
        if let Err(why) = route::fan_out(&paths, &plan.payload, &roles) {
            Out::note(&format!("the Roles could not take this ask: {why}"));
            return None;
        }
        Some(Routed {
            paths,
            ask: plan.payload.ask.clone(),
        })
    }

    impl Routed {
        /// Draws each Role's answer as it lands, and returns when all have or the wait is over.
        pub(crate) async fn show(self) {
            let cancel = Arc::new(AtomicBool::new(false));
            let flag = cancel.clone();
            let mut task = tokio::task::spawn_blocking(move || self.watch(&flag));
            tokio::select! {
                _ = &mut task => {}
                _ = tokio::signal::ctrl_c() => {
                    cancel.store(true, Ordering::SeqCst);
                    let _ = task.await;
                }
            }
        }

        fn watch(&self, cancel: &AtomicBool) {
            let fancy = Ui::detect(false).fancy();
            let look = Look::detect(fancy);
            let width = card::stderr_width();
            let mut drawn: Vec<String> = Vec::new();
            let started = Instant::now();
            let mut waiting = false;
            loop {
                let rows = match route::poll(&self.paths, &self.ask) {
                    Ok(rows) => rows,
                    Err(why) => {
                        clear(waiting);
                        Out::note(&format!("the Roles' answers could not be read: {why}"));
                        return;
                    }
                };
                for row in &rows {
                    if drawn.contains(&row.role) || matches!(row.outcome, Outcome::Working) {
                        continue;
                    }
                    clear(waiting);
                    let owned = card_of(row);
                    card::draw(&card::render(&owned.card(), width, look));
                    drawn.push(row.role.clone());
                    waiting = false;
                }
                let left: Vec<&str> = rows
                    .iter()
                    .filter(|r| !drawn.contains(&r.role))
                    .map(|r| r.role.as_str())
                    .collect();
                if left.is_empty() {
                    return;
                }
                if cancel.load(Ordering::SeqCst) || started.elapsed() > WAIT {
                    clear(waiting);
                    Out::note(&format!(
                        "{} still working in the session host: {}; `iohr stop` ends them",
                        if left.len() == 1 {
                            "1 Role is"
                        } else {
                            "Roles are"
                        },
                        left.join(", ")
                    ));
                    return;
                }
                if fancy {
                    card::say(
                        &look
                            .dim()
                            .apply_to(format!(
                                "{} of {} Roles answered · waiting for {}",
                                drawn.len(),
                                rows.len(),
                                left.join(", ")
                            ))
                            .to_string(),
                    );
                    waiting = true;
                }
                std::thread::sleep(POLL);
            }
        }
    }

    fn clear(waiting: bool) {
        if waiting {
            card::spin_clear();
        }
    }

    /// One Role's card: the Io card, the Role and its model in the meta.
    fn card_of(row: &Landed) -> OwnedCard {
        let agent = format!("Role {}", row.role);
        let label = format!("{agent}:");
        match &row.outcome {
            Outcome::Answered(r) => OwnedCard {
                agent,
                label,
                model: r.model.clone(),
                elapsed: Some(Duration::from_millis(r.duration_ms)),
                text: r.text.clone(),
                failed: false,
            },
            Outcome::Failed(why) => OwnedCard {
                agent,
                label,
                model: String::new(),
                elapsed: None,
                text: why.clone(),
                failed: true,
            },
            Outcome::Working => OwnedCard {
                agent,
                label,
                model: String::new(),
                elapsed: None,
                text: String::new(),
                failed: true,
            },
        }
    }

    /// The strings a [`Card`] borrows.
    struct OwnedCard {
        agent: String,
        label: String,
        model: String,
        elapsed: Option<Duration>,
        text: String,
        failed: bool,
    }

    impl OwnedCard {
        fn card(&self) -> Card<'_> {
            Card {
                agent: &self.agent,
                model: &self.model,
                mode: None,
                elapsed: self.elapsed,
                follows: None,
                read: "",
                notice: "",
                full: false,
                label: &self.label,
                hint: None,
                row: None,
                status: &[],
                body: if self.failed {
                    Body::Failed(&self.text)
                } else {
                    Body::Answer(&self.text)
                },
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn names(v: &[&str]) -> Vec<String> {
            v.iter().map(|s| (*s).to_owned()).collect()
        }

        #[test]
        fn every_hosted_role_answers_unless_role_names_some() {
            let hosted = names(&["rust-engineer", "security", "sdk-api"]);
            assert_eq!(chosen(None, &hosted), Some(hosted.clone()));
            assert_eq!(
                chosen(Some("security, sdk-api,security"), &hosted),
                Some(names(&["security", "sdk-api"]))
            );
            assert_eq!(chosen(Some("security,ghost"), &hosted), None);
            assert_eq!(chosen(Some(" , "), &hosted), None);
        }
    }
}

#[cfg(not(unix))]
mod imp {
    use crate::cli::{AskArgs, Global};

    pub(crate) struct Plan;
    pub(crate) struct Routed;

    pub(crate) fn plan(_: &Global, _: &AskArgs, _: bool) -> Option<Plan> {
        None
    }

    pub(crate) async fn start(_: Plan) -> Option<Routed> {
        None
    }

    impl Routed {
        pub(crate) async fn show(self) {}
    }
}
