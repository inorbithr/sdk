//! What `init` offers once the project is understood: the catalogue's extensions that fit it.
//!
//! The matching is a small table ([`RULES`]): what init knows about the project (its
//! template and the person's description) against the first-party listings the
//! catalogue holds. The catalogue is read with the client `ext search` uses
//! (`ext_catalogue::listings`) and an extension is installed through `ext install`
//! (`ext::run`), so the trust prompt, the keep policy and the signature check are the
//! ones every install meets. A listing that is `coming` is shown as coming and never
//! offered. The offer defaults to No, and a run with `--yes`, no terminal or `--json`
//! installs nothing: it prints the `ext install` line instead.

use serde_json::Value;

use super::ext_catalogue::{FIRST_PARTY, ident, is_coming};
use crate::Env;
use crate::cli::{ExtCommand, Global};
use crate::error::Error;
use crate::output::Out;
use crate::ui::{Mark, Ui};

/// What init knows about the project.
pub(crate) struct Signals<'a> {
    /// The template id (`blank`, `demo`, `coordinated`, ...).
    pub template: &'a str,
    /// The person's description of the project, when they gave one.
    pub describe: Option<&'a str>,
}

/// One row of the table: a first-party listing and when it fits.
pub(crate) struct Rule {
    /// The listing's name under the `inorbit` publisher.
    pub listing: &'static str,
    /// Why it fits, in plain words, shown beside the offer.
    pub why: &'static str,
    /// Whether the project's signals call for it.
    pub fits: fn(&Signals<'_>) -> bool,
}

/// The table: signals to listing names. Every `listing` is a program in the platform's
/// seed (`crates/extensions/seed/inorbit.json`).
pub(crate) const RULES: &[Rule] = &[
    Rule {
        // The crawler ships inside the agent extension (`iohr agent atlas crawl <URL>`), so
        // this row points at the agent's listing; `plan` offers it once.
        listing: "agent",
        why: "crawls your public site on your machine for your atlas and memory; once it is installed, run `iohr agent atlas crawl <URL>`",
        fits: wants_crawler,
    },
    Rule {
        listing: "capture",
        why: "counts the traffic your service sees from inside your network",
        fits: wants_capture,
    },
    Rule {
        listing: "agent",
        why: "runs checks in your network and dials out to InOrbit",
        fits: wants_agent,
    },
];

fn wants_crawler(s: &Signals<'_>) -> bool {
    let Some(d) = s.describe else { return false };
    let lower = d.to_lowercase();
    lower.contains("http://")
        || lower.contains("https://")
        || lower.contains("www.")
        || has_word(
            d,
            &[
                "site", "sites", "website", "websites", "crawl", "crawls", "crawler", "scrape",
                "scraper",
            ],
        )
}

fn wants_capture(s: &Signals<'_>) -> bool {
    s.describe
        .is_some_and(|d| has_word(d, &["traffic", "ebpf", "pcap", "packets"]))
}

fn wants_agent(s: &Signals<'_>) -> bool {
    s.template == "coordinated"
        || s.describe
            .is_some_and(|d| has_word(d, &["fleet", "robot", "robots", "monitoring"]))
}

/// Whether any of `words` is a word of `text`, any case (so `site` is not `opposite`).
fn has_word(text: &str, words: &[&str]) -> bool {
    text.split(|c: char| !c.is_alphanumeric())
        .any(|t| words.iter().any(|w| t.eq_ignore_ascii_case(w)))
}

/// Whether a listing can be installed now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum State {
    /// Released: `ext install` takes it.
    Installable,
    /// Announced, not released: shown, never offered.
    Coming,
}

/// One extension that fits the project and is in the catalogue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Offer {
    /// `publisher/name`, as `ext install` takes it.
    pub id: String,
    pub why: &'static str,
    pub state: State,
}

/// The rules that fit these signals and are in `listings`, in table order. A rule whose
/// listing the catalogue does not show this account is left out, and two rules that name
/// one listing give one offer (the first rule's reason).
pub(crate) fn plan(signals: &Signals<'_>, listings: &[Value]) -> Vec<Offer> {
    let mut offers: Vec<Offer> = Vec::new();
    for r in RULES.iter().filter(|r| (r.fits)(signals)) {
        let id = format!("{FIRST_PARTY}/{}", r.listing);
        if offers.iter().any(|o| o.id == id) {
            continue;
        }
        let Some(found) = listings.iter().find(|x| ident(x) == id) else {
            continue;
        };
        offers.push(Offer {
            id,
            why: r.why,
            state: if is_coming(found) {
                State::Coming
            } else {
                State::Installable
            },
        });
    }
    offers
}

/// What init does with the offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    /// A dry run or the review pass: nothing, not even a catalogue read.
    Skip,
    /// `--yes`, no terminal or `--json`: print the install line, install nothing.
    Hint,
    /// A terminal: ask once per installable extension, default No.
    Ask,
}

/// The mode for how init runs. `interactive` is init's own (a terminal and no `--yes`).
pub(crate) fn mode(dry: bool, json: bool, interactive: bool) -> Mode {
    if dry {
        Mode::Skip
    } else if json || !interactive {
        Mode::Hint
    } else {
        Mode::Ask
    }
}

/// Walks the offers: says a line for each, asks about the installable ones in `Ask`
/// mode, and returns the ids to install. In `Hint` and `Skip` nothing is asked and
/// nothing is returned; a coming listing is never returned in any mode.
///
/// # Errors
/// The person cancelled the prompt.
pub(crate) fn respond(
    mode: Mode,
    bin: &str,
    offers: &[Offer],
    mut ask: impl FnMut(&Offer) -> Result<bool, Error>,
    mut say: impl FnMut(&str),
) -> Result<Vec<String>, Error> {
    let mut install = Vec::new();
    if mode == Mode::Skip {
        return Ok(install);
    }
    for o in offers {
        match (o.state, mode) {
            (State::Coming, _) => say(&format!(
                "{} is not released yet, so there is nothing to install. Once it is, it {}.",
                o.id, o.why
            )),
            (State::Installable, Mode::Ask) => {
                if ask(o)? {
                    install.push(o.id.clone());
                }
            }
            (State::Installable, _) => say(&format!(
                "{} fits this project: it {}. Install it with `{bin} ext install {}`.",
                o.id, o.why, o.id
            )),
        }
    }
    Ok(install)
}

/// How the offer ended, as init's report step (`done`, `skipped` or `failed`), which init
/// draws as a check, a warning or a cross like its other steps. `lines` is what the offer
/// said; `installed` and `failed` are what `ext install` did. `drawn` is whether the step
/// line itself carries `lines` (else they were printed as notes already). `installable` is
/// whether any offer could be installed at all: when none could (all still coming), the
/// step says there was nothing to install, not that something was skipped.
pub(crate) fn summary(
    lines: &[String],
    installed: &[String],
    failed: &[String],
    drawn: bool,
    installable: bool,
) -> crate::project::Step {
    let said = |base: String| {
        if drawn && !lines.is_empty() {
            format!("{base}; {}", lines.join(" "))
        } else {
            base
        }
    };
    if !failed.is_empty() {
        crate::project::step(
            "extensions",
            "Extensions",
            "failed",
            said(failed.join("; ")),
            "",
        )
    } else if !installed.is_empty() {
        let base = format!("installed {}", installed.join(", "));
        crate::project::step("extensions", "Extensions", "done", said(base), "")
    } else {
        let base = if installable {
            "none installed"
        } else {
            "nothing released to install"
        }
        .to_owned();
        let base = if drawn || lines.is_empty() {
            base
        } else {
            format!("{base}, the install line is printed above")
        };
        crate::project::step("extensions", "Extensions", "skipped", said(base), "")
    }
}

/// Installs one extension the way `ext install` does, trust prompt and signature check
/// included. Init's offer and init's crawl both go through here.
pub(crate) async fn install_one(g: &Global, env: &Env, out: Out, id: &str) -> Result<(), Error> {
    let cmd = ExtCommand::Install {
        extension: id.to_owned(),
        lock: None,
        yes: false,
        no_service: false,
        interfaces: Vec::new(),
        all_interfaces: false,
    };
    super::ext::run(g, env, cmd, out).await
}

/// The one call init makes: reads the catalogue when something fits, offers or hints,
/// and installs what the person said yes to through `ext install`. It never fails the
/// init: a catalogue that cannot be read or an install that fails is one line. It returns
/// the report step to add when it offered or hinted at something, drawn here like init's
/// other steps on a terminal.
///
/// At a terminal the question is put in the planning pass before the review (`preview`), the
/// ids the person said yes to are kept in `kept`, and the run after Create only installs
/// those: it asks nothing. A run that finds no kept answer prints the install line instead.
#[allow(
    clippy::too_many_arguments,
    reason = "init's own context, as its other steps take it"
)]
pub(crate) async fn offer(
    g: &Global,
    env: &Env,
    ui: Ui,
    out: Out,
    dry: bool,
    preview: bool,
    interactive: bool,
    signals: &Signals<'_>,
    kept: &mut Option<Vec<String>>,
) -> Option<crate::project::Step> {
    let asking = preview && interactive && !out.json;
    let how = if asking {
        Mode::Ask
    } else if !dry && interactive && !out.json && kept.is_none() {
        Mode::Hint
    } else {
        mode(dry, out.json, interactive)
    };
    if how == Mode::Skip || !RULES.iter().any(|r| (r.fits)(signals)) || (asking && kept.is_some())
    {
        return None;
    }
    let bin = super::shell::running_bin();
    let listings = match super::ext_catalogue::listings(g, env).await {
        Ok(l) => l,
        Err(e) => {
            Out::note(&format!(
                "Could not read the extensions catalogue ({e}); `{bin} ext search` lists what fits."
            ));
            return None;
        }
    };
    let mut offers = plan(signals, &listings);
    // What is installed already (the crawl step may have just installed the agent) is not
    // offered again.
    if let Ok(store) = super::ext::store(g) {
        offers.retain(|o| {
            let name = o.id.rsplit('/').next().unwrap_or(&o.id);
            !matches!(store.installed(name), Ok(Some(_)))
        });
    }
    if offers.is_empty() {
        return None;
    }
    // On a drawn terminal the step line carries what was said; elsewhere each line is a
    // note, the stable text scripts read.
    let drawn = ui.fancy() && !out.json;
    let mut lines: Vec<String> = Vec::new();
    let ids = respond(
        how,
        &bin,
        &offers,
        // Installing is never the default.
        |o| {
            if asking {
                ui.confirm(&format!("Install {}? It {}.", o.id, o.why), false)
            } else {
                Ok(kept.as_ref().is_some_and(|k| k.contains(&o.id)))
            }
        },
        |line| {
            if asking {
                return;
            }
            if !drawn {
                Out::note(line);
            }
            lines.push(line.to_owned());
        },
    );
    let ids = match ids {
        Ok(ids) => ids,
        Err(e) => {
            Out::note(&format!("Skipped the extension offer: {e}"));
            return None;
        }
    };
    if asking {
        *kept = Some(ids);
        return None;
    }
    let mut installed = Vec::new();
    let mut failed = Vec::new();
    for id in ids {
        match install_one(g, env, out, &id).await {
            Ok(()) => installed.push(id),
            Err(e) => failed.push(format!(
                "could not install {id}: {e}; `{bin} ext install {id}` tries again"
            )),
        }
    }
    let installable = offers.iter().any(|o| o.state == State::Installable);
    let st = summary(&lines, &installed, &failed, drawn, installable);
    if drawn && !installable && failed.is_empty() {
        // Only listings still coming: a fact, not a skipped step, so no warning mark.
        ui.info(&lines.join(" "));
    } else if drawn {
        let (mark, said) = super::project::step_line(&st);
        match mark {
            Mark::Ok => ui.done(&said),
            Mark::Warn => ui.warn(&said),
            Mark::Fail => ui.failed(&said),
        }
    }
    Some(st)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sig<'a>(template: &'a str, describe: Option<&'a str>) -> Signals<'a> {
        Signals { template, describe }
    }

    fn catalogue() -> Vec<Value> {
        vec![
            json!({"publisher": "inorbit", "name": "agent", "kind": "program", "coming": false}),
            json!({"publisher": "inorbit", "name": "capture", "kind": "program", "coming": false}),
        ]
    }

    fn coming(id: &str) -> Offer {
        Offer {
            id: id.to_owned(),
            why: "does a thing",
            state: State::Coming,
        }
    }

    fn ids(o: &[Offer]) -> Vec<&str> {
        o.iter().map(|o| o.id.as_str()).collect()
    }

    #[test]
    fn the_rule_table_maps_signals_to_listings() {
        let c = catalogue();
        let site = plan(&sig("blank", Some("Read my site https://example.com")), &c);
        assert_eq!(ids(&site), ["inorbit/agent"]);
        let why = site[0].why;
        assert!(why.contains("iohr agent atlas crawl <URL>"), "{why}");
        let traffic = plan(&sig("demo", Some("count the traffic per route")), &c);
        assert_eq!(ids(&traffic), ["inorbit/capture"]);
        let coordinated = plan(&sig("coordinated", None), &c);
        assert_eq!(ids(&coordinated), ["inorbit/agent"]);
        let both = plan(&sig("coordinated", Some("crawl our website")), &c);
        assert_eq!(
            ids(&both),
            ["inorbit/agent"],
            "the crawler is the agent's: one offer"
        );
        // Nothing in the signals, nothing offered; a word is a whole word.
        assert_eq!(ids(&plan(&sig("blank", None), &c)), Vec::<&str>::new());
        assert_eq!(
            ids(&plan(
                &sig("blank", Some("the opposite of a todo list")),
                &c
            )),
            Vec::<&str>::new()
        );
    }

    #[test]
    fn a_rule_whose_listing_the_catalogue_does_not_show_is_left_out() {
        let only_capture = vec![catalogue()[1].clone()];
        assert_eq!(
            ids(&plan(&sig("blank", Some("read my site")), &only_capture)),
            Vec::<&str>::new()
        );
        let other = vec![json!({"publisher": "acme", "name": "agent", "coming": false})];
        assert_eq!(
            ids(&plan(&sig("blank", Some("read my site")), &other)),
            Vec::<&str>::new()
        );
    }

    #[test]
    fn a_coming_listing_is_marked_coming_and_never_offered_for_install() {
        let offers = [coming("inorbit/crawler")];
        for m in [Mode::Ask, Mode::Hint, Mode::Skip] {
            let mut asked = Vec::new();
            let mut said = Vec::new();
            let install = respond(
                m,
                "iohr",
                &offers,
                |o| {
                    asked.push(o.id.clone());
                    Ok(true)
                },
                |l| said.push(l.to_owned()),
            )
            .unwrap_or_default();
            assert!(!install.iter().any(|i| i == "inorbit/crawler"), "{m:?}");
            assert!(!asked.iter().any(|i| i == "inorbit/crawler"), "{m:?}");
            if m != Mode::Skip {
                assert!(
                    said.iter()
                        .any(|l| l.starts_with("inorbit/crawler is not released yet")
                            && !l.contains("ext install")),
                    "{m:?}: {said:?}"
                );
            }
        }
    }

    #[test]
    fn a_run_without_a_terminal_prints_the_hint_and_installs_nothing() {
        assert_eq!(mode(false, false, false), Mode::Hint);
        assert_eq!(mode(false, true, true), Mode::Hint);
        assert_eq!(mode(true, false, true), Mode::Skip);
        assert_eq!(mode(false, false, true), Mode::Ask);
        let offers = plan(&sig("coordinated", None), &catalogue());
        let mut said = Vec::new();
        let install = respond(
            Mode::Hint,
            "iohr",
            &offers,
            |_| Err(Error::Usage("asked without a terminal".into())),
            |l| said.push(l.to_owned()),
        );
        assert_eq!(
            install.unwrap_or_else(|_| vec!["error".into()]),
            Vec::<String>::new()
        );
        assert_eq!(said.len(), 1);
        assert!(
            said[0].contains("`iohr ext install inorbit/agent`"),
            "{said:?}"
        );
    }

    #[test]
    fn the_offer_installs_only_what_the_person_accepts() {
        let offers = plan(&sig("coordinated", Some("count the traffic")), &catalogue());
        assert_eq!(ids(&offers), ["inorbit/capture", "inorbit/agent"]);
        let install = respond(
            Mode::Ask,
            "iohr",
            &offers,
            |o| Ok(o.id == "inorbit/agent"),
            |_| {},
        );
        assert_eq!(install.unwrap_or_default(), ["inorbit/agent"]);
    }

    #[test]
    fn the_offer_ends_as_one_report_step_with_the_mark_of_what_happened() {
        let said =
            vec!["inorbit/crawler is not released yet, so there is nothing to install.".to_owned()];
        let ok = summary(&said, &["inorbit/agent".into()], &[], true, true);
        assert_eq!((ok.id.as_str(), ok.state.as_str()), ("extensions", "done"));
        assert!(
            ok.said.starts_with("installed inorbit/agent"),
            "{}",
            ok.said
        );
        let skipped = summary(&said, &[], &[], true, true);
        assert_eq!(skipped.state, "skipped");
        assert!(
            skipped.said.starts_with("none installed"),
            "{}",
            skipped.said
        );
        // Only listings still coming: nothing was skipped, there was nothing to install.
        let none_yet = summary(&said, &[], &[], true, false);
        assert!(
            none_yet.said.starts_with("nothing released to install"),
            "{}",
            none_yet.said
        );
        assert!(
            !none_yet.said.contains("none installed"),
            "{}",
            none_yet.said
        );
        let bad = summary(
            &[],
            &[],
            &["could not install inorbit/agent: no".into()],
            false,
            true,
        );
        assert_eq!(bad.state, "failed");
        // Printed as notes already: the step does not say the lines twice.
        let noted = summary(&said, &[], &[], false, true);
        assert!(!noted.said.contains("not released"), "{}", noted.said);
    }
}
