//! Auto mode in the panel: the account's auto-approve policy (ADR 0142), the one setting the
//! Mac app, the phone and the service share.
//!
//! The pill and the menu show whether it is on, and the Auto view turns it on or off. Turning it
//! off needs no proof and is done at once. Turning it on does not turn it on: it asks the
//! phone (the ask carries the window, 8 hours), and the phone's device key sets the policy;
//! the panel says so and shows it on when the policy reads active.

use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use super::{Effect, Key, Model, Parts, Source, View, back_footer};
use crate::card::Rgb;
use crate::commands::asks::{AUTO_WINDOW_HOURS, PanelAuto};
use crate::panel::cells::{AMBER, FAINT, GREEN, KEY, Line, MUTE, Span, TXT, bold, one_wide, sp};

/// "18:00 UTC" for an RFC 3339 time; the text itself when it does not parse.
pub(super) fn clock(at: &str) -> String {
    OffsetDateTime::parse(at, &Rfc3339).map_or_else(
        |_| one_wide(at),
        |t| {
            let t = t.to_offset(time::UtcOffset::UTC);
            format!("{:02}:{:02} UTC", t.hour(), t.minute())
        },
    )
}

impl Model {
    /// Sets the policy as read; an active policy ends the "asked the phone" wait.
    pub(in crate::panel) fn set_auto(&mut self, auto: Source<PanelAuto>) {
        if matches!(&auto, Source::Ready(a) if a.active) {
            self.auto_asked = false;
        }
        self.auto = auto;
    }

    /// The pill's auto cell.
    pub(super) fn mode_spans(&self) -> Vec<Span> {
        match &self.auto {
            Source::Ready(a) if a.active => {
                vec![sp("auto ", MUTE), sp("●", GREEN), sp(" on", GREEN)]
            }
            Source::Ready(_) => vec![sp("auto ", MUTE), sp("○", MUTE), sp(" off", MUTE)],
            Source::Loading => vec![sp("auto ", MUTE), sp("…", MUTE)],
            Source::Unavailable(_) => vec![sp("auto ", MUTE), sp("unavailable", FAINT)],
        }
    }

    /// The menu's right-hand cell for Auto mode.
    pub(super) fn auto_menu_right(&self) -> (String, Rgb) {
        match &self.auto {
            Source::Ready(a) if a.active => ("● on".to_owned(), GREEN),
            Source::Ready(_) => ("○ off".to_owned(), MUTE),
            Source::Loading => ("…".to_owned(), MUTE),
            Source::Unavailable(_) => ("n/a".to_owned(), FAINT),
        }
    }

    pub(super) fn auto_key(&mut self, key: Key) -> Effect {
        match key {
            Key::Esc => {
                self.view = View::Menu;
                self.note = None;
                Effect::None
            }
            Key::Enter | Key::Char(' ') => match &self.auto {
                Source::Ready(a) if a.active => {
                    self.note = Some("turning auto mode off".into());
                    Effect::AutoOff
                }
                Source::Ready(_) if self.auto_asked => {
                    self.auto_asked = false;
                    self.note = Some("your phone was asked already; ⏎ again asks once more".into());
                    Effect::None
                }
                Source::Ready(_) => {
                    self.auto_asked = true;
                    self.note = Some("asking your phone …".into());
                    Effect::AutoOn
                }
                Source::Loading => {
                    self.note = Some("auto mode is still being read".into());
                    Effect::None
                }
                Source::Unavailable(_) => {
                    self.note = Some("auto mode cannot be read, so it is not changed".into());
                    Effect::None
                }
            },
            _ => Effect::None,
        }
    }

    pub(super) fn auto_view(&self) -> Parts {
        let plain = |t: &str, fg| Line {
            spans: vec![sp(t, fg)],
            selected: false,
        };
        // Text longer than a row wraps; nothing is cut off silently.
        let para = |body: &mut Vec<Line>, t: &str, fg| {
            body.extend(super::reader::wrap(t, ROOM).iter().map(|r| plain(r, fg)));
        };
        let mut body = Vec::new();
        let mut right = Vec::new();
        let mut footer = back_footer();
        match &self.auto {
            Source::Loading => body.push(plain("Reading the auto-approve policy …", MUTE)),
            Source::Unavailable(why) => {
                body.push(plain("Auto mode unavailable.", AMBER));
                para(&mut body, why, MUTE);
            }
            Source::Ready(a) if a.active => {
                right = vec![sp("● on", GREEN)];
                para(
                    &mut body,
                    &format!("On until {}.", clock(&a.expires_at)),
                    TXT,
                );
                para(
                    &mut body,
                    &format!(
                        "{} ask{} decided by it so far.",
                        a.approved,
                        plural(a.approved)
                    ),
                    MUTE,
                );
                Self::never(&mut body, a);
                footer = enter_footer("turn off");
            }
            Source::Ready(a) => {
                right = vec![sp("○ off", MUTE)];
                para(&mut body, "Off. Nothing is approved without you.", TXT);
                para(
                    &mut body,
                    &format!(
                        "Turning it on asks your phone for {AUTO_WINDOW_HOURS} hours; its key signs."
                    ),
                    MUTE,
                );
                Self::never(&mut body, a);
                footer = enter_footer("ask your phone");
            }
        }
        if let Some(n) = &self.note {
            para(&mut body, n, AMBER);
        }
        (
            vec![sp(" › ", FAINT), bold("Auto mode", TXT)],
            right,
            body,
            Some(footer),
        )
    }

    fn never(body: &mut Vec<Line>, a: &PanelAuto) {
        if !a.never_kinds.is_empty() {
            let text = format!("Never by it: {}", a.never_kinds.join(", "));
            body.extend(super::reader::wrap(&text, ROOM).iter().map(|r| Line {
                spans: vec![sp(r, FAINT)],
                selected: false,
            }));
        }
    }
}

/// The text room of the Auto view, inside its box.
pub(super) const ROOM: usize = 50;

fn plural(n: u64) -> &'static str {
    if n == 1 { "" } else { "s" }
}

pub(super) fn enter_footer(what: &str) -> Vec<Span> {
    vec![
        sp("⏎", KEY),
        sp(&format!(" {what}   "), MUTE),
        sp("esc", KEY),
        sp(" back", MUTE),
    ]
}
