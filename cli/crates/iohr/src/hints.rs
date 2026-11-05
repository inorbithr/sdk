//! What closes a `?` card: the full disclosure now and then, one short hint the rest of
//! the time (PRD 0187). The header's "AI answer" label is on every card; only the footer
//! varies, and the state it needs is one small file in the config directory.
//!
//! The decision is pure ([`decide`], with the clock passed in); [`load`] and [`store`] are the
//! file. Nothing here reads a question or an answer.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

/// The file, in the config directory beside `config.toml`. Written by `iohr ask` only.
const FILE: &str = "ask-card.json";

/// The full disclosure comes back this often (seconds): about once a day.
const FULL_EVERY: u64 = 24 * 60 * 60;

/// The short hints, shown in turn.
pub(crate) const HINTS: [&str; 3] = [
    "`??` follows up.",
    "`iohr ask --help` for flags.",
    "`[ask] model` in config picks the model.",
];

/// `[ask] hints`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Mode {
    /// The first card, a changed notice and about once a day: the full disclosure; a hint
    /// otherwise.
    #[default]
    Sometimes,
    /// The full disclosure on every card.
    Always,
    /// The full disclosure on the first card and when it changes; nothing otherwise.
    Never,
}

impl Mode {
    /// # Errors
    /// A value that is not `sometimes`, `always` or `never`.
    pub(crate) fn parse(value: Option<&str>) -> Result<Self, String> {
        match value.map(str::trim) {
            None | Some("" | "sometimes") => Ok(Self::Sometimes),
            Some("always") => Ok(Self::Always),
            Some("never") => Ok(Self::Never),
            Some(other) => Err(format!(
                "`[ask] hints` is `{other}`; use `sometimes`, `always` or `never`"
            )),
        }
    }
}

/// What the last cards left behind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct State {
    /// A fingerprint of the disclosure text the last full footer showed.
    notice: String,
    /// When it showed (Unix seconds).
    full_at: u64,
    /// Short footers shown since; picks the next hint.
    shorts: u64,
}

/// The footer a card gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Footer {
    /// "Read: ... An AI model ...".
    Full,
    /// One short hint, or none.
    Short(Option<&'static str>),
}

/// A fingerprint of the disclosure text: any change to the text shows it again.
#[must_use]
pub(crate) fn fingerprint(notice: &str) -> String {
    Sha256::digest(notice.as_bytes())
        .iter()
        .take(8)
        .fold(String::new(), |mut s, b| {
            use std::fmt::Write as _;
            let _ = write!(s, "{b:02x}");
            s
        })
}

/// The footer for the next card and the state to keep. `now` is Unix seconds.
#[must_use]
pub(crate) fn decide(state: Option<&State>, mode: Mode, notice: &str, now: u64) -> (Footer, State) {
    let notice = fingerprint(notice);
    let full = match state {
        None => true,
        Some(s) if s.notice != notice => true,
        Some(s) => match mode {
            Mode::Always => true,
            Mode::Never => false,
            Mode::Sometimes => now.saturating_sub(s.full_at) >= FULL_EVERY,
        },
    };
    if full {
        let shorts = state.map_or(0, |s| s.shorts);
        return (
            Footer::Full,
            State {
                notice,
                full_at: now,
                shorts,
            },
        );
    }
    let s = state.cloned().unwrap_or(State {
        notice,
        full_at: now,
        shorts: 0,
    });
    let hint = match mode {
        Mode::Never => None,
        _ => HINTS.get(usize::try_from(s.shorts).unwrap_or(0) % HINTS.len()),
    };
    let next = State {
        shorts: s.shorts.wrapping_add(1),
        ..s
    };
    (Footer::Short(hint.copied()), next)
}

fn path(dir: &Path) -> PathBuf {
    dir.join(FILE)
}

/// The saved state; none when the file is missing or unreadable (the next card is then
/// the first).
#[must_use]
pub(crate) fn load(dir: &Path) -> Option<State> {
    serde_json::from_slice(&std::fs::read(path(dir)).ok()?).ok()
}

/// Keeps the state. A directory that cannot be written costs only the rotation: the card
/// is still drawn, so the error is dropped.
pub(crate) fn store(dir: &Path, state: &State) {
    let Ok(bytes) = serde_json::to_vec(state) else {
        return;
    };
    let _ = std::fs::create_dir_all(dir);
    let tmp = path(dir).with_extension("json.tmp");
    if std::fs::write(&tmp, bytes).is_ok() && std::fs::rename(&tmp, path(dir)).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

/// The footer of the next card from the file, and the file updated.
#[must_use]
pub(crate) fn next(dir: &Path, mode: Mode, notice: &str, now: u64) -> Footer {
    let (footer, state) = decide(load(dir).as_ref(), mode, notice, now);
    store(dir, &state);
    footer
}

/// Now, in Unix seconds.
#[must_use]
pub(crate) fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    const N: &str = "An AI model under your own subscription. Check it.";
    const T0: u64 = 1_800_000_000;

    fn step(s: &mut Option<State>, mode: Mode, notice: &str, now: u64) -> Footer {
        let (f, next) = decide(s.as_ref(), mode, notice, now);
        *s = Some(next);
        f
    }

    #[test]
    fn the_first_card_is_full_the_second_is_a_short_hint() {
        let mut s = None;
        assert_eq!(step(&mut s, Mode::Sometimes, N, T0), Footer::Full);
        assert_eq!(
            step(&mut s, Mode::Sometimes, N, T0 + 60),
            Footer::Short(Some(HINTS[0]))
        );
        assert_eq!(
            step(&mut s, Mode::Sometimes, N, T0 + 120),
            Footer::Short(Some(HINTS[1]))
        );
    }

    #[test]
    fn the_hints_go_round_in_turn() {
        let mut s = None;
        step(&mut s, Mode::Sometimes, N, T0);
        let seen: Vec<_> = (1..=4)
            .map(|i| step(&mut s, Mode::Sometimes, N, T0 + i))
            .collect();
        assert_eq!(seen[2], Footer::Short(Some(HINTS[2])));
        assert_eq!(seen[3], Footer::Short(Some(HINTS[0])), "back to the first");
    }

    #[test]
    fn the_full_notice_returns_when_the_text_changes_or_a_day_has_passed() {
        let mut s = None;
        step(&mut s, Mode::Sometimes, N, T0);
        assert!(matches!(
            step(&mut s, Mode::Sometimes, N, T0 + 3600),
            Footer::Short(_)
        ));
        assert_eq!(
            step(&mut s, Mode::Sometimes, "A different notice.", T0 + 3700),
            Footer::Full,
            "the text changed"
        );
        assert!(matches!(
            step(&mut s, Mode::Sometimes, "A different notice.", T0 + 3800),
            Footer::Short(_)
        ));
        assert!(matches!(
            step(
                &mut s,
                Mode::Sometimes,
                "A different notice.",
                T0 + 3700 + FULL_EVERY - 1
            ),
            Footer::Short(_)
        ));
        assert_eq!(
            step(
                &mut s,
                Mode::Sometimes,
                "A different notice.",
                T0 + 3700 + FULL_EVERY
            ),
            Footer::Full,
            "a day since the last full one"
        );
        assert!(matches!(
            step(
                &mut s,
                Mode::Sometimes,
                "A different notice.",
                T0 + 3700 + FULL_EVERY + 1
            ),
            Footer::Short(_)
        ));
    }

    #[test]
    fn always_is_full_every_time_and_never_only_on_the_first_and_a_change() {
        let mut s = None;
        for i in 0..3 {
            assert_eq!(step(&mut s, Mode::Always, N, T0 + i), Footer::Full);
        }
        let mut s = None;
        assert_eq!(step(&mut s, Mode::Never, N, T0), Footer::Full);
        assert_eq!(step(&mut s, Mode::Never, N, T0 + 5), Footer::Short(None));
        assert_eq!(
            step(&mut s, Mode::Never, N, T0 + 10 * FULL_EVERY),
            Footer::Short(None),
            "a day is not a reason under never"
        );
        assert_eq!(step(&mut s, Mode::Never, "Changed.", T0 + 11), Footer::Full);
    }

    #[test]
    fn the_state_lives_in_one_file_and_a_missing_or_bad_file_means_the_first_card() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(next(d.path(), Mode::Sometimes, N, T0), Footer::Full);
        assert_eq!(
            next(d.path(), Mode::Sometimes, N, T0 + 1),
            Footer::Short(Some(HINTS[0]))
        );
        assert_eq!(
            next(d.path(), Mode::Sometimes, N, T0 + 2 + FULL_EVERY),
            Footer::Full
        );
        std::fs::write(d.path().join(FILE), "not json").unwrap();
        assert_eq!(next(d.path(), Mode::Sometimes, N, T0 + 3), Footer::Full);
        // A folder that cannot exist is not an error.
        let _ = next(Path::new("/proc/none/x"), Mode::Sometimes, N, T0);
    }

    #[test]
    fn no_hint_names_the_off_switch() {
        assert!(HINTS.iter().all(|h| !h.contains("INORBIT_ASK")));
    }

    #[test]
    fn the_mode_reads_the_three_words() {
        assert_eq!(Mode::parse(None), Ok(Mode::Sometimes));
        assert_eq!(Mode::parse(Some("always")), Ok(Mode::Always));
        assert_eq!(Mode::parse(Some(" never ")), Ok(Mode::Never));
        assert!(Mode::parse(Some("often")).is_err());
    }
}
