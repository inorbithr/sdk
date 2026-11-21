//! Showing this panel's terminal on the phone (RFC 0193) and letting the phone type into it.
//!
//! The panel hands its shell to the Mac app's terminal host ([`crate::panel::phone`]); the app's
//! relay, pairing and consent do the rest. This file is the state machine the menu shows. It
//! decides nothing the app decides: a share needs the Mac's own "Share" answer, and typing needs
//! the person's second ask here plus the Mac's "Allow" answer (the desktop's `input.rs` rule,
//! unchanged). What this state says is only what the Mac told back.
//!
//! States: `Off` -> `Asking` (looking for the app, then waiting for its dialog) -> `Shared`
//! (with typing `Off`, `Asked`, `On` or `Refused`) and back to `Off` by the same menu row.
//! `Unavailable` holds one plain line (no app, an old app, a lost link); the row tries again.
//! Typing can only be asked for while shared, and a refusal, a stop or a lost link ends it.

use super::{Effect, Model};
use crate::card::Rgb;
use crate::panel::cells::{AMBER, FAINT, GREEN, MUTE, Span, sp};

/// Whether the phone may type, as far as the Mac has said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::panel) enum Typing {
    Off,
    /// Asked for in the panel; the Mac has not said yes.
    Asked,
    On,
    /// The Mac (or the build) said no; the line says why.
    Refused(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::panel) enum Share {
    Off,
    /// The link is opening. `app` is true once the Mac app has answered, so what it waits for is
    /// the person's answer to the Mac's question.
    Asking {
        app: bool,
    },
    Shared {
        typing: Typing,
    },
    Unavailable(String),
}

/// What the link says back, as the state machine takes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::panel) enum Told {
    /// The Mac app answered: it is running.
    App,
    /// The Mac's decision. `refused` is `(what, why)` of an ask it turned down.
    Flags {
        shared: bool,
        typing: bool,
        refused: Option<(String, String)>,
    },
    /// The link is gone, or there never was one: one plain line.
    Lost(String),
}

impl Model {
    #[cfg(test)]
    pub(in crate::panel) fn share(&self) -> &Share {
        &self.share
    }

    /// Whether anything is open to the phone now (the link is up or being opened).
    pub(super) fn sharing(&self) -> bool {
        matches!(self.share, Share::Asking { .. } | Share::Shared { .. })
    }

    /// Whether the person turned typing on in the panel (asked or granted): what the link offers.
    pub(in crate::panel) fn typing_wanted(&self) -> bool {
        matches!(
            self.share,
            Share::Shared {
                typing: Typing::Asked | Typing::On
            }
        )
    }

    /// The "Share to phone" row: on when off or unavailable, off when on.
    pub(super) fn share_toggle(&mut self) -> Effect {
        if self.stub {
            self.note = Some("demo: nothing is shown on a phone".into());
            return Effect::None;
        }
        if self.sharing() {
            self.share = Share::Off;
            self.note = Some("stopped showing this terminal".into());
            Effect::ShareOff
        } else {
            self.share = Share::Asking { app: false };
            self.note = Some("looking for the Mac app …".into());
            Effect::ShareOn
        }
    }

    /// The "Let phone type" row: only while shared. Asking turns it on once the Mac says yes.
    pub(super) fn typing_toggle(&mut self) -> Effect {
        let Share::Shared { typing } = &self.share else {
            self.note = Some("turn on Share to phone first".into());
            return Effect::None;
        };
        if matches!(typing, Typing::Asked | Typing::On) {
            self.share = Share::Shared {
                typing: Typing::Off,
            };
            self.note = Some("the phone cannot type any more".into());
            Effect::Typing(false)
        } else {
            self.share = Share::Shared {
                typing: Typing::Asked,
            };
            self.note = Some("asking on the Mac, answer there".into());
            Effect::Typing(true)
        }
    }

    /// What the link told. Returns [`Effect::ShareOff`] when the link should be closed (the Mac
    /// said no to the share); the person sees why in the menu.
    pub(in crate::panel) fn share_told(&mut self, told: Told) -> Effect {
        match told {
            Told::Lost(why) => {
                if self.sharing() {
                    self.share = Share::Unavailable(why);
                    self.note = None;
                }
                Effect::None
            }
            _ if !self.sharing() => Effect::None,
            Told::App => {
                if matches!(self.share, Share::Asking { .. }) {
                    self.share = Share::Asking { app: true };
                    self.note = Some("answer the question on the Mac".into());
                }
                Effect::None
            }
            Told::Flags {
                shared: false,
                refused,
                ..
            } => {
                if let Some((_, why)) = refused.filter(|(what, _)| what == "share") {
                    self.share = Share::Off;
                    self.note = Some(why);
                    Effect::ShareOff
                } else {
                    // The Mac has not decided (or dropped it): it is asking again.
                    self.share = Share::Asking { app: true };
                    Effect::None
                }
            }
            Told::Flags {
                shared: true,
                typing,
                refused,
            } => {
                let before = match &self.share {
                    Share::Shared { typing } => typing.clone(),
                    _ => Typing::Off,
                };
                let wanted = matches!(before, Typing::Asked | Typing::On);
                let typing = if wanted && typing {
                    Typing::On
                } else if let Some((_, why)) = refused.filter(|(what, _)| what == "typing") {
                    self.note = Some(why.clone());
                    Typing::Refused(why)
                } else if wanted {
                    Typing::Asked
                } else {
                    Typing::Off
                };
                if matches!(self.share, Share::Asking { .. }) {
                    self.note = Some("this terminal is on your phone".into());
                }
                self.share = Share::Shared { typing };
                Effect::None
            }
        }
    }

    /// The share row's right-hand cell.
    pub(super) fn share_menu_right(&self) -> (String, Rgb) {
        match &self.share {
            Share::Off => ("○ off".to_owned(), MUTE),
            Share::Asking { .. } => ("… asking".to_owned(), AMBER),
            Share::Shared { .. } => ("● shared".to_owned(), GREEN),
            Share::Unavailable(_) => ("n/a".to_owned(), FAINT),
        }
    }

    /// The typing row's right-hand cell.
    pub(super) fn typing_menu_right(&self) -> (String, Rgb) {
        match &self.share {
            Share::Shared { typing } => match typing {
                Typing::Off | Typing::Refused(_) => ("○ off".to_owned(), MUTE),
                Typing::Asked => ("… asking".to_owned(), AMBER),
                Typing::On => ("● on".to_owned(), GREEN),
            },
            _ => ("–".to_owned(), FAINT),
        }
    }

    /// The pill's cell: the "shared" mark while on (nothing when not shared).
    pub(super) fn share_spans(&self) -> Vec<Span> {
        match &self.share {
            Share::Shared { typing } => {
                let mut s = vec![sp("phone ", MUTE), sp("●", GREEN), sp(" shared", GREEN)];
                if *typing == Typing::On {
                    s.push(sp(" · types", AMBER));
                }
                s
            }
            Share::Asking { .. } => vec![sp("phone ", MUTE), sp("…", AMBER)],
            _ => Vec::new(),
        }
    }

    /// The line the menu shows under its rows: the last thing said, else why sharing is not
    /// available. Wrapped by the caller, never cut.
    pub(super) fn share_line(&self) -> Option<String> {
        if let Some(n) = &self.note {
            return Some(n.clone());
        }
        match &self.share {
            Share::Unavailable(why)
            | Share::Shared {
                typing: Typing::Refused(why),
            } => Some(why.clone()),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::Key;
    use super::*;

    fn flags(shared: bool, typing: bool, refused: Option<(&str, &str)>) -> Told {
        Told::Flags {
            shared,
            typing,
            refused: refused.map(|(a, b)| (a.into(), b.into())),
        }
    }

    fn open() -> Model {
        let mut m = Model::new(false);
        m.key(Key::Menu);
        m
    }

    #[test]
    fn the_menu_option_turns_sharing_on_and_off_and_marks_it_while_on() {
        let mut m = open();
        assert_eq!(*m.share(), Share::Off);
        assert_eq!(m.share_spans().len(), 0, "no mark while off");
        assert_eq!(m.key(Key::Char('h')), Effect::ShareOn);
        assert_eq!(*m.share(), Share::Asking { app: false });
        assert_eq!(m.share_told(Told::App), Effect::None);
        assert_eq!(*m.share(), Share::Asking { app: true });
        assert_eq!(m.share_told(flags(true, false, None)), Effect::None);
        assert_eq!(
            *m.share(),
            Share::Shared {
                typing: Typing::Off
            }
        );
        let mark: String = m.share_spans().iter().map(|s| s.text.as_str()).collect();
        assert!(mark.contains("shared"), "the pill says it: {mark}");
        assert_eq!(m.share_menu_right().0, "● shared");
        assert_eq!(m.key(Key::Char('h')), Effect::ShareOff);
        assert_eq!(*m.share(), Share::Off);
        assert_eq!(m.share_spans().len(), 0);
    }

    #[test]
    fn the_mac_saying_no_closes_the_link_and_says_why() {
        let mut m = open();
        m.key(Key::Char('h'));
        m.share_told(Told::App);
        let why = "Not shared: the question on the Mac was not answered with Share.";
        assert_eq!(
            m.share_told(flags(false, false, Some(("share", why)))),
            Effect::ShareOff
        );
        assert_eq!(*m.share(), Share::Off);
        assert_eq!(m.share_line().as_deref(), Some(why));
    }

    #[test]
    fn typing_is_off_until_asked_for_and_the_mac_allows_it() {
        let mut m = open();
        // Not shared yet: the row asks for nothing.
        assert_eq!(m.key(Key::Char('t')), Effect::None);
        assert!(!m.typing_wanted());
        m.key(Key::Char('h'));
        m.share_told(flags(true, false, None));
        assert!(!m.typing_wanted(), "shared does not mean typing");
        // Even a Mac that says typing is on changes nothing the person did not ask for.
        m.share_told(flags(true, true, None));
        assert_eq!(
            *m.share(),
            Share::Shared {
                typing: Typing::Off
            }
        );
        assert_eq!(m.key(Key::Char('t')), Effect::Typing(true));
        assert!(m.typing_wanted());
        assert_eq!(m.typing_menu_right().0, "… asking");
        m.share_told(flags(true, false, None));
        assert_eq!(m.typing_menu_right().0, "… asking", "no yes yet");
        m.share_told(flags(true, true, None));
        assert_eq!(m.typing_menu_right().0, "● on");
        let mark: String = m.share_spans().iter().map(|s| s.text.as_str()).collect();
        assert!(mark.contains("types"));
        assert_eq!(m.key(Key::Char('t')), Effect::Typing(false));
        assert_eq!(m.typing_menu_right().0, "○ off");
    }

    #[test]
    fn a_refused_typing_ask_shows_why_and_stops_asking_until_asked_again() {
        let mut m = open();
        m.key(Key::Char('h'));
        m.share_told(flags(true, false, None));
        m.key(Key::Char('t'));
        let why = "Typing from the phone is not turned on in this build.";
        m.share_told(flags(true, false, Some(("typing", why))));
        assert_eq!(
            *m.share(),
            Share::Shared {
                typing: Typing::Refused(why.into())
            }
        );
        assert!(!m.typing_wanted());
        assert_eq!(m.share_line().as_deref().map(str::len), Some(why.len()));
        // The same row asks again.
        assert_eq!(m.key(Key::Char('t')), Effect::Typing(true));
    }

    #[test]
    fn without_the_app_one_plain_line_says_so_and_the_row_tries_again() {
        let mut m = open();
        m.key(Key::Char('h'));
        let line = "The InOrbit Mac app is not running.";
        assert_eq!(m.share_told(Told::Lost(line.into())), Effect::None);
        assert_eq!(*m.share(), Share::Unavailable(line.into()));
        assert_eq!(m.share_menu_right().0, "n/a");
        assert_eq!(m.share_line().as_deref(), Some(line));
        assert_eq!(m.share_spans().len(), 0);
        // Typing cannot be asked for while there is no share.
        assert_eq!(m.key(Key::Char('t')), Effect::None);
        assert_eq!(m.key(Key::Char('h')), Effect::ShareOn, "try again");
        // A late word from a link already given up changes nothing.
        let mut off = open();
        assert_eq!(off.share_told(flags(true, true, None)), Effect::None);
        assert_eq!(*off.share(), Share::Off);
    }

    #[test]
    fn a_lost_link_ends_the_share_and_the_typing() {
        let mut m = open();
        m.key(Key::Char('h'));
        m.share_told(flags(true, false, None));
        m.key(Key::Char('t'));
        m.share_told(flags(true, true, None));
        m.share_told(Told::Lost(
            "The link to the Mac app was lost. Turn it on again.".into(),
        ));
        assert!(matches!(m.share(), Share::Unavailable(_)));
        assert!(!m.typing_wanted());
        assert!(!m.sharing());
    }

    #[test]
    fn the_demo_panel_shares_nothing() {
        let mut m = Model::new(true);
        m.key(Key::Menu);
        assert_eq!(m.key(Key::Char('h')), Effect::None);
        assert_eq!(*m.share(), Share::Off);
    }
}
