//! The network half of the project's space: `iohr init` and `iohr project sync` make (or find)
//! the project's Decisions space in the chosen account and report what happened. The pure half,
//! the decision, the link files and the queue, is `crate::project::space`, whose module doc has
//! the state machine (RFC 0127.1, PRD 0088, RFC 0115).

use inorbithr::public::{CreateSpaceRequest, DecisionsListSpacesParams};
use serde_json::Value;

use super::accounts::Me;
use super::decisions::decisions;
use crate::Env;
use crate::cli::{Global, ProjectSyncArgs};
use crate::context::{Api, shared_session};
use crate::error::Error;
use crate::output::Out;
use crate::project::space::{self, Ask, Decision, Readers, Seen};
use crate::project::{self, Step};
use crate::ui::{Choice, Ui};

/// A space the sync made or found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Linked {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) account: String,
    pub(crate) created: bool,
    pub(crate) readers: Readers,
    pub(crate) personal: bool,
}

/// How one sync ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// The space exists and the folder can link to it.
    Linked(Linked),
    /// It could not happen yet; waiting can cure it. The words hold no account data.
    Queued(String),
    /// It cannot happen as asked (a name taken by another kind of space, a refused request).
    Failed(String),
}

/// What `iohr project sync` says to run later.
pub(crate) const LATER: &str = "run `iohr project sync` in the folder once you are signed in";

/// Whether a failed call is one that waiting or signing in again can cure: no answer, a refused
/// or forbidden sign-in (a token without the scope, a role that may not create spaces), the rate
/// limit and the server's own errors. Anything else the request itself got wrong.
fn waits(e: &inorbithr::Error) -> bool {
    match e.status() {
        None | Some(401 | 403 | 408 | 425 | 429) => true,
        Some(s) => s >= 500,
    }
}

/// The words for a failed call that waits, with no account data in them.
fn why(e: &inorbithr::Error) -> String {
    match e.status() {
        None => "InOrbit could not be reached".to_owned(),
        Some(401) => "the sign-in was refused: run `iohr login`".to_owned(),
        Some(403) => "this sign-in may not create projects there: it needs the decisions:write scope and a role that may create spaces".to_owned(),
        Some(_) => "InOrbit is busy or down".to_owned(),
    }
}

/// A failed call as an outcome: queued when waiting cures it, else failed with its message.
fn ended(e: &inorbithr::Error) -> Outcome {
    if waits(e) {
        Outcome::Queued(why(e))
    } else {
        Outcome::Failed(e.to_string())
    }
}

/// The spaces of an account whose name or slug holds `name`, reduced to what the decision reads.
async fn seen(api: &Api, account: &str, name: &str) -> Result<Vec<Seen>, inorbithr::Error> {
    let mut params = DecisionsListSpacesParams::default();
    params.account_id = Some(account.to_owned());
    params.query = Some(name.to_owned());
    let all = decisions(api).all_list_spaces(&params).collect().await?;
    Ok(all
        .into_iter()
        .map(|s| Seen {
            space_id: s.space_id,
            slug: s.slug,
            name: s.name,
            kind: s.kind,
        })
        .collect())
}

/// The account the space goes in, and whether it is the person's own: `--team` (refused when
/// the person is not in it), else the signed-in account. Without a readable account list the
/// team named is taken as given and the server decides.
fn pick_account(
    me: Option<&Me>,
    team: Option<&str>,
    signed_in: &str,
) -> Result<(String, bool), Error> {
    match (me, team) {
        (Some(mine), Some(wanted)) => {
            let found = mine.find(wanted).ok_or_else(|| {
                Error::Usage(format!(
                    "--team {wanted}: not an account you belong to; the ones you do: {}",
                    mine.listed()
                ))
            })?;
            Ok((found.id.clone(), found.id == mine.personal().id))
        }
        (Some(mine), None) => Ok((signed_in.to_owned(), signed_in == mine.personal().id)),
        (None, Some(wanted)) => Ok((wanted.to_owned(), false)),
        (None, None) => Ok((signed_in.to_owned(), false)),
    }
}

/// Makes or finds the project's space in the asked account. Idempotent: the slug is the
/// project's name and the list runs first, so a repeat links to what the first made.
///
/// # Errors
///
/// `--team` names an account the person is not in: refused before anything is made. Every
/// other failure is an [`Outcome`], never an error, so `init` goes on.
pub(crate) async fn sync(g: &Global, env: &Env, name: &str, ask: &Ask) -> Result<Outcome, Error> {
    let Ok(s) = shared_session(g, env).await else {
        return Ok(Outcome::Queued("you are not signed in".to_owned()));
    };
    let me = match s.api.get::<Value>("/v1/accounts/me", &[]).await {
        Ok(v) => Me::read(&v),
        // A token without account:read still creates a space in its own account; a team
        // it names cannot be checked, so the server decides.
        Err(e) if e.status() == Some(403) => None,
        Err(e) => return Ok(ended(&e)),
    };
    let (account, personal) = pick_account(me.as_ref(), ask.team.as_deref(), &s.account)?;
    let mut created = false;
    let mut tried = false;
    let id_name = loop {
        let list = match seen(&s.api, &account, name).await {
            Ok(l) => l,
            Err(e) => return Ok(ended(&e)),
        };
        match space::decide(&list, name) {
            Decision::Reuse { id, name } => break (id, name),
            Decision::Taken { kind } => {
                return Ok(Outcome::Failed(format!(
                    "the name {name} is already a {kind} in that account: pick another name, or `iohr project sync` after renaming the folder"
                )));
            }
            Decision::Create if tried => {
                return Ok(Outcome::Failed(
                    "the space was refused as existing yet is not listed: try again later".into(),
                ));
            }
            Decision::Create => {}
        }
        let mut body = CreateSpaceRequest::default();
        body.account_id = Some(account.clone());
        body.name = Some(name.to_owned());
        body.slug = Some(name.to_owned());
        body.kind = Some(space::KIND.to_owned());
        body.access_floor = Some(ask.readers.floor().to_owned());
        body.description = Some(format!("The project {name}, made by `iohr init`."));
        match decisions(&s.api).create_space(&body).await {
            Ok(r) => {
                let Some(sp) = r.value.space else {
                    return Ok(Outcome::Failed(
                        "InOrbit made the space but named none".to_owned(),
                    ));
                };
                created = true;
                break (
                    sp.space_id,
                    if sp.name.is_empty() {
                        name.to_owned()
                    } else {
                        sp.name
                    },
                );
            }
            // Someone made it between the list and the create: list again and reuse it.
            Err(e) if e.status() == Some(409) => tried = true,
            Err(e) => return Ok(ended(&e)),
        }
    };
    Ok(Outcome::Linked(Linked {
        id: id_name.0,
        name: id_name.1,
        account,
        created,
        readers: ask.readers,
        personal,
    }))
}

/// The report's `space` step for an outcome (`None`: `--no-sync`).
pub(crate) fn step(outcome: Option<&Outcome>, env: &str) -> Step {
    const TITLE: &str = "The project's space in InOrbit";
    match outcome {
        None => project::step(
            "space",
            TITLE,
            "skipped",
            "--no-sync: nothing was made in InOrbit",
            "",
        ),
        Some(Outcome::Linked(l)) => project::step(
            "space",
            TITLE,
            "done",
            format!(
                "{} the {} space {} ({}) on {env}; readable by {}; the team opens it in the app and the console under Projects",
                if l.created { "made" } else { "found" },
                if l.personal { "personal" } else { "team" },
                l.id,
                l.name,
                l.readers.said(l.personal),
            ),
            "",
        ),
        Some(Outcome::Queued(why)) => project::step(
            "space",
            TITLE,
            "queued",
            format!("not made yet: {why}"),
            LATER,
        ),
        Some(Outcome::Failed(why)) => project::step("space", TITLE, "failed", why.clone(), ""),
    }
}

/// What init asks about the space at a terminal: the team (when the person is in more than one
/// account) and who reads it (when it is a team's). Flags answer without a question; any failure
/// to read the accounts leaves the signed-in account, and the sync says what went wrong.
///
/// # Errors
///
/// `--visibility` is not a value, or the person cancelled a menu.
pub(crate) async fn ask(
    g: &Global,
    env: &Env,
    ui: Ui,
    team: Option<&str>,
    visibility: Option<&str>,
    interactive: bool,
) -> Result<Ask, Error> {
    let flagged = visibility
        .map(Readers::parse)
        .transpose()
        .map_err(Error::Usage)?;
    let mut out = Ask {
        team: team.map(str::to_owned),
        readers: flagged.unwrap_or_default(),
    };
    if !interactive || (team.is_some() && flagged.is_some()) {
        return Ok(out);
    }
    let Ok(s) = shared_session(g, env).await else {
        return Ok(out);
    };
    let Some(me) = s
        .api
        .get::<Value>("/v1/accounts/me", &[])
        .await
        .ok()
        .and_then(|v| Me::read(&v))
    else {
        return Ok(out);
    };
    let account = match team {
        Some(t) => me.find(t).map(|k| k.id.clone()),
        None if me.accounts.len() > 1 => {
            let choices: Vec<Choice> = me
                .accounts
                .iter()
                .map(|k| Choice {
                    value: k.id.clone(),
                    label: if k.name.is_empty() {
                        k.slug.clone()
                    } else {
                        k.name.clone()
                    },
                    hint: if k.id == me.personal().id {
                        "your own, only you".to_owned()
                    } else {
                        "a team".to_owned()
                    },
                    available: true,
                    key: None,
                })
                .collect();
            let initial = if me.find(&s.account).is_some() {
                s.account.clone()
            } else {
                me.personal().id.clone()
            };
            let id = ui.select("Which account owns the project?", &choices, &initial)?;
            out.team = Some(id.clone());
            Some(id)
        }
        None => Some(s.account.clone()),
    };
    let personal = account.as_deref() == Some(me.personal().id.as_str());
    if flagged.is_none() && !personal && account.is_some() {
        let choices: Vec<Choice> = Readers::NAMES
            .iter()
            .filter_map(|n| Readers::parse(n).ok())
            .map(|r| Choice {
                value: Readers::NAMES[usize::from(r != Readers::Team)].to_owned(),
                label: if r == Readers::Team {
                    "Everyone in the team".to_owned()
                } else {
                    "Owners and admins only".to_owned()
                },
                hint: if r == Readers::Team {
                    "recommended: the team opens it from the app".to_owned()
                } else {
                    "a smaller circle; more people can be let in later".to_owned()
                },
                available: true,
                key: None,
            })
            .collect();
        let v = ui.select("Who can read the project?", &choices, &choices[0].value)?;
        out.readers = Readers::parse(&v).map_err(Error::Usage)?;
    }
    Ok(out)
}

/// The project's name: `name` in `inorbit.toml`, else the folder's own name.
fn name_of(dir: &std::path::Path) -> Result<String, Error> {
    let named = std::fs::read_to_string(dir.join("inorbit.toml"))
        .ok()
        .and_then(|t| t.parse::<toml::Table>().ok())
        .and_then(|t| {
            t.get("name")
                .and_then(toml::Value::as_str)
                .map(str::to_owned)
        });
    let name = named
        .or_else(|| dir.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_default();
    project::check_name(&name).map_err(Error::Usage)?;
    Ok(name)
}

/// `iohr project sync`: makes or finds the project's space and links the folder to it, with the
/// answers `init` queued when it could not reach InOrbit. A folder that is already linked (a
/// teammate cloned it) is checked, never given a second space: either this sign-in opens the
/// linked space, or the person is told whom to ask.
///
/// # Errors
///
/// The folder is not a project, `--team` is not an account of the person, or the space could not
/// be made yet (the same words `init` queued, and a non-zero exit, because the person asked).
pub(crate) async fn sync_command(
    g: &Global,
    env: &Env,
    a: ProjectSyncArgs,
    out: Out,
) -> Result<(), Error> {
    let dir = match &a.dir {
        Some(d) => d.clone(),
        None => std::env::current_dir()
            .map_err(|e| Error::Failed(format!("cannot read the current folder: {e}")))?,
    };
    let name = name_of(&dir)?;
    let env_name = g.environment_name().to_owned();
    let state = super::project::state_root()?.join(&name);
    let queued = space::queued(&state, &env_name);
    let flagged = a
        .visibility
        .as_deref()
        .map(Readers::parse)
        .transpose()
        .map_err(Error::Usage)?;
    let mut ask = queued.as_ref().map(|q| q.ask.clone()).unwrap_or_default();
    if let Some(t) = &a.team {
        ask.team = Some(t.clone());
    }
    if let Some(r) = flagged {
        ask.readers = r;
    }
    if a.dry_run {
        let said = format!(
            "would: make or find the space `{name}` in {} on {env_name} and link {} to it",
            ask.team.as_deref().unwrap_or("the signed-in account"),
            dir.display()
        );
        if out.json {
            Out::print_json(&serde_json::json!({ "dry_run": true, "would": said }));
        } else {
            Out::note(&said);
        }
        return Ok(());
    }
    let linked = space::read_link_for(&dir, &env_name);
    let outcome = match linked {
        Some((id, _)) if queued.is_none() && a.team.is_none() => check_linked(g, env, &id).await?,
        _ => sync(g, env, &name, &ask).await?,
    };
    if let Outcome::Linked(l) = &outcome {
        space::write_link(&dir, &env_name, &l.id, &l.name, &l.account).map_err(Error::Failed)?;
        if env_name == space::PRODUCTION {
            let path = dir.join("inorbit.toml");
            if let Ok(text) = std::fs::read_to_string(&path) {
                let filled = space::fill_manifest(&text, &l.id);
                if filled != text {
                    std::fs::write(&path, filled).map_err(|e| {
                        Error::Failed(format!("cannot write {}: {e}", path.display()))
                    })?;
                }
            }
        }
        space::dequeue(&state, &env_name);
    }
    let st = step(Some(&outcome), &env_name);
    if out.json {
        Out::print_json(&st);
    } else {
        Out::note(&format!("{}: {}", st.state, st.said));
    }
    match outcome {
        Outcome::Linked(_) => Ok(()),
        Outcome::Queued(why) => {
            if let Some(q) = queued {
                let _ = space::queue(&state, &space::Queued { why, ..q });
            }
            Err(Error::Failed(format!("{}: {LATER}", st.said)))
        }
        Outcome::Failed(why) => Err(Error::Failed(why)),
    }
}

/// A folder that is linked already: the linked space must open for this sign-in.
async fn check_linked(g: &Global, env: &Env, id: &str) -> Result<Outcome, Error> {
    let Ok(s) = shared_session(g, env).await else {
        return Ok(Outcome::Queued("you are not signed in".to_owned()));
    };
    match decisions(&s.api).get_space(id).await {
        Ok(r) => {
            let sp = r.value.space.unwrap_or_default();
            Ok(Outcome::Linked(Linked {
                id: id.to_owned(),
                name: sp.name,
                account: sp.account_id,
                created: false,
                readers: if sp.access_floor.is_empty() {
                    Readers::Team
                } else {
                    Readers::Managers
                },
                personal: false,
            }))
        }
        Err(e) if matches!(e.status(), Some(403 | 404)) => Ok(Outcome::Failed(format!(
            "this folder is linked to the space {id}, which this sign-in cannot open: ask a team owner or admin to add you to the team, then run `iohr project sync` again"
        ))),
        Err(e) => Ok(ended(&e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn api(status: u16) -> inorbithr::Error {
        inorbithr::Error::Connection {
            host: format!("h{status}"),
            reason: String::new(),
        }
    }

    #[test]
    fn no_answer_waits_and_names_no_account() {
        let e = api(0);
        assert!(waits(&e));
        assert_eq!(ended(&e), Outcome::Queued(why(&e)));
        assert!(!why(&e).contains("acc_"));
    }

    fn me() -> Me {
        let v = serde_json::json!({
            "account": {"id": "acc_me", "slug": "me", "name": "Me", "plan": "personal"},
            "teams": [{"id": "acc_t", "slug": "team", "name": "Team", "plan": "team"}]
        });
        Me::read(&v).expect("a readable answer")
    }

    #[test]
    fn the_team_flag_picks_an_account_the_person_is_in_and_refuses_another() {
        let m = me();
        assert_eq!(
            pick_account(Some(&m), Some("team"), "acc_me").expect("a team"),
            ("acc_t".to_owned(), false)
        );
        assert_eq!(
            pick_account(Some(&m), None, "acc_me").expect("own"),
            ("acc_me".to_owned(), true)
        );
        let refused = pick_account(Some(&m), Some("acc_x"), "acc_me").expect_err("refused");
        assert!(refused.to_string().contains("acc_t"));
        assert_eq!(
            pick_account(None, Some("acc_x"), "acc_me").expect("given"),
            ("acc_x".to_owned(), false)
        );
    }

    #[test]
    fn the_queued_step_says_what_runs_later() {
        let s = step(
            Some(&Outcome::Queued("InOrbit could not be reached".into())),
            "production",
        );
        assert_eq!(s.state, "queued");
        assert!(s.needs.contains("iohr project sync"));
        assert_eq!(step(None, "production").state, "skipped");
    }

    #[test]
    fn a_linked_step_names_the_space_and_who_reads_it() {
        let l = Linked {
            id: "lab_1".into(),
            name: "shop".into(),
            account: "acc_1".into(),
            created: true,
            readers: Readers::Managers,
            personal: false,
        };
        let s = step(Some(&Outcome::Linked(l)), "local");
        assert_eq!(s.state, "done");
        assert!(s.said.contains("lab_1") && s.said.contains("owners and admins"));
        assert!(s.said.contains("on local"));
    }
}
