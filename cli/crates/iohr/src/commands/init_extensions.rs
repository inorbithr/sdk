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
        listing: "crawler",
        why: "reads your public site on your machine and turns it into facts for your atlas and memory",
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
/// listing the catalogue does not show this account is left out.
pub(crate) fn plan(signals: &Signals<'_>, listings: &[Value]) -> Vec<Offer> {
    RULES
        .iter()
        .filter(|r| (r.fits)(signals))
        .filter_map(|r| {
            let id = format!("{FIRST_PARTY}/{}", r.listing);
            let found = listings.iter().find(|x| ident(x) == id)?;
            Some(Offer {
                id,
                why: r.why,
                state: if is_coming(found) {
                    State::Coming
                } else {
                    State::Installable
                },
            })
        })
        .collect()
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
                "{} is coming, not released yet: it {}.",
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
/// line itself carries `lines` (else they were printed as notes already).
pub(crate) fn summary(
    lines: &[String],
    installed: &[String],
    failed: &[String],
    drawn: bool,
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
        let base = "none installed".to_owned();
        let base = if drawn || lines.is_empty() {
            base
        } else {
            format!("{base}, the install line is printed above")
        };
        crate::project::step("extensions", "Extensions", "skipped", said(base), "")
    }
}

/// The one call init makes: reads the catalogue when something fits, offers or hints,
/// and installs what the person said yes to through `ext install`. It never fails the
/// init: a catalogue that cannot be read or an install that fails is one line. It returns
/// the report step to add when it offered or hinted at something, drawn here like init's
/// other steps on a terminal.
pub(crate) async fn offer(
    g: &Global,
    env: &Env,
    ui: Ui,
    out: Out,
    dry: bool,
    interactive: bool,
    signals: &Signals<'_>,
) -> Option<crate::project::Step> {
    let how = mode(dry, out.json, interactive);
    if how == Mode::Skip || !RULES.iter().any(|r| (r.fits)(signals)) {
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
    let offers = plan(signals, &listings);
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
        |o| ui.confirm(&format!("Install {}? It {}.", o.id, o.why), false),
        |line| {
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
    let mut installed = Vec::new();
    let mut failed = Vec::new();
    for id in ids {
        let cmd = ExtCommand::Install {
            extension: id.clone(),
            lock: None,
            yes: false,
            no_service: false,
            interfaces: Vec::new(),
            all_interfaces: false,
        };
        match super::ext::run(g, env, cmd, out).await {
            Ok(()) => installed.push(id),
            Err(e) => failed.push(format!(
                "could not install {id}: {e}; `{bin} ext install {id}` tries again"
            )),
        }
    }
    let st = summary(&lines, &installed, &failed, drawn);
    if drawn {
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
            json!({"publisher": "inorbit", "name": "crawler", "kind": "program", "coming": true}),
        ]
    }

    fn ids(o: &[Offer]) -> Vec<&str> {
        o.iter().map(|o| o.id.as_str()).collect()
    }

    #[test]
    fn the_rule_table_maps_signals_to_listings() {
        let c = catalogue();
        let site = plan(&sig("blank", Some("Read my site https://example.com")), &c);
        assert_eq!(ids(&site), ["inorbit/crawler"]);
        let traffic = plan(&sig("demo", Some("count the traffic per route")), &c);
        assert_eq!(ids(&traffic), ["inorbit/capture"]);
        let coordinated = plan(&sig("coordinated", None), &c);
        assert_eq!(ids(&coordinated), ["inorbit/agent"]);
        let both = plan(&sig("coordinated", Some("crawl our website")), &c);
        assert_eq!(ids(&both), ["inorbit/crawler", "inorbit/agent"]);
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
        let only_agent = vec![catalogue()[0].clone()];
        assert_eq!(
            ids(&plan(&sig("blank", Some("read my site")), &only_agent)),
            Vec::<&str>::new()
        );
        let other = vec![json!({"publisher": "acme", "name": "crawler", "coming": false})];
        assert_eq!(
            ids(&plan(&sig("blank", Some("read my site")), &other)),
            Vec::<&str>::new()
        );
    }

    #[test]
    fn a_coming_listing_is_marked_coming_and_never_offered_for_install() {
        let c = catalogue();
        let offers = plan(&sig("coordinated", Some("crawl our website")), &c);
        let crawler = offers.iter().find(|o| o.id == "inorbit/crawler");
        assert_eq!(crawler.map(|o| o.state), Some(State::Coming));
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
                        .any(|l| l.starts_with("inorbit/crawler is coming")
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
        let said = vec!["inorbit/crawler is coming, not released yet: it reads.".to_owned()];
        let ok = summary(&said, &["inorbit/agent".into()], &[], true);
        assert_eq!((ok.id.as_str(), ok.state.as_str()), ("extensions", "done"));
        assert!(
            ok.said.starts_with("installed inorbit/agent"),
            "{}",
            ok.said
        );
        let skipped = summary(&said, &[], &[], true);
        assert_eq!(skipped.state, "skipped");
        assert!(skipped.said.contains("coming"), "{}", skipped.said);
        let bad = summary(
            &[],
            &[],
            &["could not install inorbit/agent: no".into()],
            false,
        );
        assert_eq!(bad.state, "failed");
        // Printed as notes already: the step does not say the lines twice.
        let noted = summary(&said, &[], &[], false);
        assert!(!noted.said.contains("coming"), "{}", noted.said);
    }
}
