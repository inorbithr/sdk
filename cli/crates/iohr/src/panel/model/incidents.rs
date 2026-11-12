//! Incidents in the panel: the switch for auto-resolution, an account setting the platform
//! keeps (`incidents.proto`, `auto_resolve`), the same one the console shows.
//!
//! It is off until someone turns it on, and turning it on changes how the account's incidents
//! are handled, so the first press asks for a second. Turning it off is one press. Both are
//! audited by the platform (who and when). A platform that does not serve the setting yet
//! shows as unavailable and nothing here changes it.

use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use super::auto::{ROOM, clock, enter_footer};
use super::{Effect, Key, Model, Parts, Source, View, back_footer};
use crate::card::Rgb;
use crate::commands::incident_settings::PanelIncidents;
use crate::panel::cells::{AMBER, FAINT, GREEN, Line, MUTE, TXT, bold, sp};

/// "2026-11-10 09:00 UTC" for an RFC 3339 time; the text itself when it does not parse.
fn when(at: &str) -> String {
    OffsetDateTime::parse(at, &Rfc3339).map_or_else(
        |_| at.to_owned(),
        |t| {
            let t = t.to_offset(time::UtcOffset::UTC);
            format!("{} {}", t.date(), clock(at))
        },
    )
}

impl Model {
    /// Sets the settings as read; a change that landed ends the "press again" wait.
    pub(in crate::panel) fn set_incidents(&mut self, incidents: Source<PanelIncidents>) {
        if matches!(&incidents, Source::Ready(i) if i.auto_resolve) {
            self.incidents_confirm = false;
        }
        self.incidents = incidents;
    }

    /// The menu's right-hand cell for Incidents.
    pub(super) fn incidents_menu_right(&self) -> (String, Rgb) {
        match &self.incidents {
            Source::Ready(i) if i.auto_resolve => ("● on".to_owned(), GREEN),
            Source::Ready(_) => ("○ off".to_owned(), MUTE),
            Source::Loading => ("…".to_owned(), MUTE),
            Source::Unavailable(_) => ("n/a".to_owned(), FAINT),
        }
    }

    pub(super) fn incidents_key(&mut self, key: Key) -> Effect {
        match key {
            Key::Esc => {
                self.view = View::Menu;
                self.note = None;
                self.incidents_confirm = false;
                Effect::None
            }
            Key::Enter | Key::Char(' ') => match &self.incidents {
                Source::Ready(i) if i.auto_resolve => {
                    self.note = Some("turning auto-resolve off".into());
                    Effect::IncidentsAutoResolve(false)
                }
                Source::Ready(_) if self.incidents_confirm => {
                    self.incidents_confirm = false;
                    self.note = Some("turning auto-resolve on".into());
                    Effect::IncidentsAutoResolve(true)
                }
                Source::Ready(_) => {
                    self.incidents_confirm = true;
                    self.note = Some("⏎ again to turn it on; esc keeps it off".into());
                    Effect::None
                }
                Source::Loading => {
                    self.note = Some("the setting is still being read".into());
                    Effect::None
                }
                Source::Unavailable(_) => {
                    self.note = Some("the setting cannot be read, so it is not changed".into());
                    Effect::None
                }
            },
            _ => Effect::None,
        }
    }

    pub(super) fn incidents_view(&self) -> Parts {
        let plain = |t: &str, fg| Line {
            spans: vec![sp(t, fg)],
            selected: false,
        };
        let para = |body: &mut Vec<Line>, t: &str, fg| {
            body.extend(super::reader::wrap(t, ROOM).iter().map(|r| plain(r, fg)));
        };
        let mut body = Vec::new();
        let mut right = Vec::new();
        let mut footer = back_footer();
        match &self.incidents {
            Source::Loading => body.push(plain("Reading the incident settings …", MUTE)),
            Source::Unavailable(why) => {
                body.push(plain("Auto-resolve unavailable.", AMBER));
                para(&mut body, why, MUTE);
            }
            Source::Ready(i) => {
                let on = i.auto_resolve;
                right = if on {
                    vec![sp("● on", GREEN)]
                } else {
                    vec![sp("○ off", MUTE)]
                };
                para(
                    &mut body,
                    if on {
                        "Auto-resolve is on."
                    } else {
                        "Auto-resolve is off."
                    },
                    TXT,
                );
                para(
                    &mut body,
                    "When every monitor on an incident the platform opened from an alert is up \
                     again, the incident is resolved without a person. Incidents a person \
                     opened are never resolved by it.",
                    MUTE,
                );
                para(
                    &mut body,
                    "Off until an owner or admin turns it on. The platform records who changed \
                     it and when.",
                    MUTE,
                );
                if i.updated_by.is_empty() {
                    para(
                        &mut body,
                        "Never changed: the platform default holds.",
                        FAINT,
                    );
                } else {
                    let by = format!(
                        "Last changed by {} on {}.",
                        i.updated_by,
                        when(&i.updated_at)
                    );
                    para(&mut body, &by, FAINT);
                }
                footer = enter_footer(if on { "turn off" } else { "turn on" });
            }
        }
        if let Some(n) = &self.note {
            para(&mut body, n, AMBER);
        }
        (
            vec![sp(" › ", FAINT), bold("Incidents", TXT)],
            right,
            body,
            Some(footer),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{model, rows};
    use super::*;

    fn settings(on: bool, by: &str) -> PanelIncidents {
        PanelIncidents {
            auto_resolve: on,
            updated_by: by.into(),
            updated_at: if by.is_empty() {
                String::new()
            } else {
                "2026-11-10T09:00:00Z".into()
            },
        }
    }

    #[test]
    fn auto_resolve_is_off_until_turned_on_with_a_second_press_and_off_with_one() {
        let mut m = model(1);
        m.set_incidents(Source::Ready(settings(false, "")));
        m.key(Key::Menu);
        m.key(Key::Char('i'));
        assert_eq!(m.view, View::Incidents);
        let t = rows(&m).join("\n");
        assert!(t.contains("Auto-resolve is off."), "{t}");
        assert!(t.contains("never resolved by it"), "{t}");
        assert!(t.contains("platform default holds"), "{t}");
        // The first press changes nothing; the second turns it on.
        assert_eq!(m.key(Key::Enter), Effect::None);
        assert!(rows(&m).join("\n").contains("again to turn it on"));
        assert_eq!(m.key(Key::Enter), Effect::IncidentsAutoResolve(true));
        // Leaving drops the wait: coming back asks twice again.
        m.key(Key::Esc);
        m.key(Key::Char('i'));
        assert_eq!(m.key(Key::Enter), Effect::None);
        // It landed: on shows who and when, and one press turns it off.
        m.set_incidents(Source::Ready(settings(true, "usr_1")));
        let t = rows(&m).join("\n");
        assert!(t.contains("Auto-resolve is on."), "{t}");
        assert!(t.contains("usr_1 on 2026-11-10 09:00 UTC"), "{t}");
        assert_eq!(m.key(Key::Enter), Effect::IncidentsAutoResolve(false));
    }

    #[test]
    fn a_platform_without_the_setting_shows_unavailable_and_is_never_changed() {
        let mut m = model(1);
        m.set_incidents(Source::Unavailable(
            "this platform does not serve auto-resolve yet".into(),
        ));
        m.key(Key::Menu);
        assert!(rows(&m).join("\n").contains("n/a"));
        m.key(Key::Char('i'));
        let t = rows(&m).join("\n");
        assert!(t.contains("Auto-resolve unavailable."), "{t}");
        assert_eq!(m.key(Key::Enter), Effect::None);
        assert_eq!(m.key(Key::Enter), Effect::None);
        m.set_incidents(Source::Loading);
        assert_eq!(m.key(Key::Enter), Effect::None);
    }
}
