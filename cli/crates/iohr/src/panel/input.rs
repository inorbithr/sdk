//! The keys and the mouse the panel reads from the terminal, parsed from the bytes the person
//! types and the terminal reports.

use super::model::Key;

/// The panel's key as terminals send it. Option+i: Esc then `i` (Option as Meta), the macOS
/// dead-key `ˆ` (U+02C6), or the CSI-u form (`ESC [ 105 ; 3 u`). Ctrl-G: the byte 7, the key
/// that works where Option+i does not.
const MENU_KEYS: [&[u8]; 4] = [b"\x1bi", "\u{2c6}".as_bytes(), b"\x1b[105;3u", b"\x07"];

/// The byte Ctrl-G sends.
const CTRL_G: &[u8] = b"\x07";

/// Terminals (`TERM_PROGRAM`) whose default makes Option+i a dead key: it types `ˆ` and the
/// terminal never sends Esc. Apple's Terminal needs "Use Option as Meta key" for it, and iTerm2,
/// the VS Code terminal, Ghostty, `WezTerm` and Hyper ship with Option composing characters too.
const DEAD_KEY_TERMINALS: [&str; 6] = [
    "Apple_Terminal",
    "iTerm.app",
    "vscode",
    "ghostty",
    "WezTerm",
    "Hyper",
];

/// What opens the panel's menu here, and what the panel calls it.
///
/// Ctrl-G always opens it (no terminal setting, no tty meaning on macOS or Linux, and readline
/// and zsh only bind it to "abort"), except when `[ask] menu_key = "ctrl-g"` gives it to the
/// `?` card. Option+i opens it too wherever the terminal sends it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct OpenKey {
    ctrl_g: bool,
    label: &'static str,
}

impl OpenKey {
    /// The key for this terminal: `term_program` is `$TERM_PROGRAM`, `ask_ctrl_g` is whether
    /// `[ask] menu_key` is `ctrl-g`.
    pub(super) fn detect(term_program: Option<&str>, ask_ctrl_g: bool) -> Self {
        let dead = term_program.is_some_and(|t| DEAD_KEY_TERMINALS.contains(&t));
        Self {
            ctrl_g: !ask_ctrl_g,
            label: if dead && !ask_ctrl_g {
                "^G"
            } else {
                "\u{2325}i"
            },
        }
    }

    /// The key for the terminal this process runs in (`$TERM_PROGRAM`).
    pub(super) fn here(ask_ctrl_g: bool) -> Self {
        Self::detect(std::env::var("TERM_PROGRAM").ok().as_deref(), ask_ctrl_g)
    }

    /// The key as the pill and the help name it.
    pub(super) fn label(self) -> &'static str {
        self.label
    }

    /// Whether this input is exactly the panel's key (the only keyboard input the panel reads
    /// at rest: anything else belongs to the shell and is forwarded whole).
    pub(super) fn is(self, bytes: &[u8]) -> bool {
        MENU_KEYS.contains(&bytes) && (self.ctrl_g || bytes != CTRL_G)
    }
}

impl Default for OpenKey {
    fn default() -> Self {
        Self::detect(None, false)
    }
}

/// When the panel reads the mouse (`[panel] mouse`, `IOHR_PANEL_MOUSE`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum MouseMode {
    /// Only while the menu (or a view) is open: the terminal keeps selecting text with a drag.
    #[default]
    Menu,
    /// Also at rest: a click on the pill opens the menu. The terminal selects text with
    /// Option (Terminal.app, iTerm2) or Shift held.
    Always,
    /// Never: the panel is keys only.
    Off,
}

impl MouseMode {
    /// The mode a setting names; a word that is not one of `menu`, `always`, `off` is the default.
    pub(super) fn parse(value: Option<&str>) -> Self {
        match value.map(|v| v.trim().to_ascii_lowercase()).as_deref() {
            Some("always") => Self::Always,
            Some("off") => Self::Off,
            _ => Self::Menu,
        }
    }
}

/// A mouse report, with cells counted from the top left corner (0, 0).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Mouse {
    /// The left button went down.
    Press {
        col: usize,
        row: usize,
    },
    /// A button came up.
    Release {
        col: usize,
        row: usize,
    },
    WheelUp {
        col: usize,
        row: usize,
    },
    WheelDown {
        col: usize,
        row: usize,
    },
}

/// One thing the person did: a key or a mouse report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Input {
    Key(Key),
    Mouse(Mouse),
}

/// Whether a read starts with an SGR mouse report (`ESC [ <`).
pub(super) fn is_mouse_report(bytes: &[u8]) -> bool {
    bytes.starts_with(b"\x1b[<")
}

/// Turns on the terminal's mouse reports: button presses and releases (`?1000`) and the wheel,
/// in the SGR form (`?1006`) that has no limit on columns. Never `?1002`: no drags, so the
/// terminal keeps selecting text with a drag.
pub(super) const MOUSE_ON: &[u8] = b"\x1b[?1000h\x1b[?1006h";
/// Turns the reports off again.
pub(super) const MOUSE_OFF: &[u8] = b"\x1b[?1006l\x1b[?1000l";

/// The longest unfinished sequence kept for the next read.
const MAX_HELD: usize = 32;

/// Reads the person's input, holding back a sequence a read ended in the middle of.
#[derive(Debug, Default)]
pub(super) struct Reader {
    held: Vec<u8>,
}

impl Reader {
    /// What one read holds, after whatever the last one left unfinished.
    pub(super) fn feed(&mut self, bytes: &[u8]) -> Vec<Input> {
        let mut all = std::mem::take(&mut self.held);
        all.extend_from_slice(bytes);
        let (inputs, held) = scan(&all);
        if held <= MAX_HELD {
            self.held = all[all.len() - held..].to_vec();
        }
        inputs
    }

    /// Whether a sequence is waiting for its end.
    pub(super) fn pending(&self) -> bool {
        !self.held.is_empty()
    }
}

/// An SGR report (`ESC [ < b ; x ; y M` or `m`), its parameters being the `<b;x;y` part.
fn mouse(params: &str, last: char) -> Option<Mouse> {
    let mut n = params.strip_prefix('<')?.split(';');
    let b: usize = n.next()?.parse().ok()?;
    let col = n.next()?.parse::<usize>().ok()?.saturating_sub(1);
    let row = n.next()?.parse::<usize>().ok()?.saturating_sub(1);
    if n.next().is_some() {
        return None;
    }
    // Bit 6 is the wheel, bit 5 motion; bits 2 to 4 are Shift, Meta and Ctrl.
    if b & 64 != 0 {
        return match b & 3 {
            0 => Some(Mouse::WheelUp { col, row }),
            1 => Some(Mouse::WheelDown { col, row }),
            _ => None,
        };
    }
    match (last, b & 32 != 0, b & 3) {
        ('M', false, 0) => Some(Mouse::Press { col, row }),
        ('m', false, _) => Some(Mouse::Release { col, row }),
        _ => None,
    }
}

/// Whether the parameters of a cursor-key sequence are the ones a terminal puts there: none,
/// or numbers split by `;` and `:` (`1;2` Shift, `1;3` Option, `1;1:3` a key released under the
/// kitty keyboard protocol). Whatever the modifier, the key is the arrow.
fn arrow_params(params: &str) -> bool {
    params
        .bytes()
        .all(|b| b.is_ascii_digit() || b == b';' || b == b':')
}

/// The arrow `key`, unless the parameters say the key was released (`:3` on the modifier field,
/// kitty protocol), which is not a second press.
fn arrow(params: &str, key: Key) -> Option<Input> {
    let released = params
        .rsplit(';')
        .next()
        .and_then(|field| field.split_once(':'))
        .is_some_and(|(_, event)| event == "3");
    (!released).then_some(Input::Key(key))
}

/// The inputs in `bytes` (a lone Esc is Esc; Backspace is DEL or BS; arrows are `ESC [ A` or
/// `ESC O A`; Enter is a carriage return or a newline; every other sequence is skipped; text is
/// one `Char` per character), and how many bytes at the end are a sequence that is not finished.
fn scan(bytes: &[u8]) -> (Vec<Input>, usize) {
    let mut out = Vec::new();
    let mut held = 0;
    let text = String::from_utf8_lossy(bytes);
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\x1b' => match chars.peek().copied() {
                None => out.push(Input::Key(Key::Esc)),
                Some('i') => {
                    chars.next();
                    out.push(Input::Key(Key::Menu));
                }
                Some(intro @ ('[' | 'O')) => {
                    chars.next();
                    let mut params = String::new();
                    let mut last = None;
                    for n in chars.by_ref() {
                        if ('\u{40}'..='\u{7e}').contains(&n) {
                            last = Some(n);
                            break;
                        }
                        params.push(n);
                    }
                    let Some(last) = last else {
                        // The read ended inside the sequence: the rest comes with the next.
                        held = bytes.len() - bytes.iter().rposition(|b| *b == 0x1b).unwrap_or(0);
                        break;
                    };
                    let found = match (intro, last, params.as_str()) {
                        (_, 'A', p) if arrow_params(p) => arrow(p, Key::Up),
                        (_, 'B', p) if arrow_params(p) => arrow(p, Key::Down),
                        (_, 'C', p) if arrow_params(p) => arrow(p, Key::Right),
                        (_, 'D', p) if arrow_params(p) => arrow(p, Key::Left),
                        ('[', 'u', "105;3") => Some(Input::Key(Key::Menu)),
                        ('[', 'M' | 'm', p) => mouse(p, last).map(Input::Mouse),
                        _ => None,
                    };
                    out.extend(found);
                }
                // Alt plus another key: not the panel's.
                Some(_) => {
                    chars.next();
                }
            },
            '\u{2c6}' | '\x07' => out.push(Input::Key(Key::Menu)),
            '\r' | '\n' => out.push(Input::Key(Key::Enter)),
            '\x7f' | '\x08' => out.push(Input::Key(Key::Backspace)),
            c if c.is_control() => {}
            c => out.push(Input::Key(Key::Char(c))),
        }
    }
    (out, held)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The keys in one read, mouse reports skipped.
    fn parse(bytes: &[u8]) -> Vec<Key> {
        scan(bytes)
            .0
            .into_iter()
            .filter_map(|i| match i {
                Input::Key(k) => Some(k),
                Input::Mouse(_) => None,
            })
            .collect()
    }

    #[test]
    fn the_menu_key_is_recognised_in_its_forms_and_only_alone() {
        let k = OpenKey::default();
        assert!(k.is(b"\x1bi"));
        assert!(k.is("\u{2c6}".as_bytes()));
        assert!(k.is(b"\x1b[105;3u"));
        assert!(k.is(b"\x07"), "Ctrl-G opens the menu");
        assert!(!k.is(b"i"));
        assert!(!k.is(b"ls\x1bi"));
        assert!(!k.is(b"\x1b"));
        assert!(!k.is(b"\x07\x07"));
    }

    #[test]
    fn the_key_is_named_for_the_terminal_that_runs_the_panel() {
        let apple = OpenKey::detect(Some("Apple_Terminal"), false);
        assert_eq!(apple.label(), "^G", "Option+i is a dead key there");
        assert!(apple.is(b"\x07") && apple.is(b"\x1bi"));
        assert_eq!(OpenKey::detect(Some("tmux"), false).label(), "\u{2325}i");
        assert_eq!(OpenKey::detect(None, false).label(), "\u{2325}i");
        let taken = OpenKey::detect(Some("Apple_Terminal"), true);
        assert!(!taken.is(b"\x07"), "[ask] menu_key = ctrl-g keeps its key");
        assert!(taken.is(b"\x1bi"));
        assert_eq!(taken.label(), "\u{2325}i");
    }

    fn inputs(bytes: &[u8]) -> Vec<Input> {
        Reader::default().feed(bytes)
    }

    #[test]
    fn sgr_reports_are_parsed_into_presses_releases_and_the_wheel() {
        assert_eq!(
            inputs(b"\x1b[<0;10;3M"),
            [Input::Mouse(Mouse::Press { col: 9, row: 2 })]
        );
        assert_eq!(
            inputs(b"\x1b[<0;10;3m"),
            [Input::Mouse(Mouse::Release { col: 9, row: 2 })]
        );
        assert_eq!(
            inputs(b"\x1b[<64;100;1M"),
            [Input::Mouse(Mouse::WheelUp { col: 99, row: 0 })]
        );
        assert_eq!(
            inputs(b"\x1b[<65;5;7M"),
            [Input::Mouse(Mouse::WheelDown { col: 4, row: 6 })]
        );
        assert_eq!(
            inputs(b"\x1b[<2;5;7M"),
            Vec::<Input>::new(),
            "the right button is not the panel's"
        );
        assert_eq!(
            inputs(b"\x1b[<32;5;7M"),
            Vec::<Input>::new(),
            "motion is not a press"
        );
    }

    #[test]
    fn keys_and_reports_mix_in_one_read_and_parse_skips_the_reports() {
        let bytes = b"\x1b[B\x1b[<0;2;2M\r";
        assert_eq!(
            inputs(bytes),
            [
                Input::Key(Key::Down),
                Input::Mouse(Mouse::Press { col: 1, row: 1 }),
                Input::Key(Key::Enter)
            ]
        );
        assert_eq!(parse(bytes), [Key::Down, Key::Enter]);
        assert!(is_mouse_report(b"\x1b[<0;1;1M"));
        assert!(!is_mouse_report(b"\x1b[A"));
    }

    #[test]
    fn a_report_split_across_reads_is_held_until_it_is_whole() {
        let mut r = Reader::default();
        assert_eq!(r.feed(b"\x1b[<0;1"), Vec::<Input>::new());
        assert!(r.pending());
        assert_eq!(r.feed(b"2;4"), Vec::<Input>::new());
        assert_eq!(
            r.feed(b"M"),
            [Input::Mouse(Mouse::Press { col: 11, row: 3 })]
        );
        assert!(!r.pending());
        // Keys before the split part come out at once.
        assert_eq!(r.feed(b"a\x1b[<65;3"), [Input::Key(Key::Char('a'))]);
        assert_eq!(
            r.feed(b";3M"),
            [Input::Mouse(Mouse::WheelDown { col: 2, row: 2 })]
        );
    }

    #[test]
    fn the_mouse_setting_names_three_modes_and_defaults_to_the_menu() {
        assert_eq!(MouseMode::parse(None), MouseMode::Menu);
        assert_eq!(MouseMode::parse(Some("menu")), MouseMode::Menu);
        assert_eq!(MouseMode::parse(Some(" Always ")), MouseMode::Always);
        assert_eq!(MouseMode::parse(Some("off")), MouseMode::Off);
        assert_eq!(MouseMode::parse(Some("nonsense")), MouseMode::Menu);
    }

    #[test]
    fn a_lone_escape_stays_escape_and_runaway_input_is_not_held() {
        assert_eq!(inputs(b"\x1b"), [Input::Key(Key::Esc)]);
        let mut r = Reader::default();
        let mut junk = b"\x1b[".to_vec();
        junk.extend(std::iter::repeat_n(b'1', 100));
        assert_eq!(r.feed(&junk), Vec::<Input>::new());
        assert!(!r.pending(), "a sequence that never ends is dropped");
    }

    #[test]
    fn arrows_with_parameters_are_arrows() {
        // Shift, Option and Ctrl forms, and the kitty keyboard protocol's press and repeat.
        assert_eq!(parse(b"\x1b[1;2A\x1b[1;2B"), [Key::Up, Key::Down]);
        assert_eq!(parse(b"\x1b[1;3C\x1b[1;5D"), [Key::Right, Key::Left]);
        assert_eq!(parse(b"\x1b[1;1:1B\x1b[1;1:2A"), [Key::Down, Key::Up]);
        assert_eq!(parse(b"\x1b[1B\x1bO1;2B"), [Key::Down, Key::Down]);
        // A release is not a second press, and an unknown final byte is still skipped.
        assert_eq!(parse(b"\x1b[1;1:3B"), Vec::<Key>::new());
        assert_eq!(parse(b"\x1b[1;2Ex"), [Key::Char('x')]);
        // The parameters may arrive in two reads.
        let mut r = Reader::default();
        assert_eq!(r.feed(b"\x1b[1;"), Vec::<Input>::new());
        assert_eq!(r.feed(b"2B"), [Input::Key(Key::Down)]);
    }

    #[test]
    fn keys_are_parsed_from_one_read() {
        assert_eq!(parse(b"\x1b"), [Key::Esc]);
        assert_eq!(parse(b"\x1b[A\x1b[B"), [Key::Up, Key::Down]);
        assert_eq!(parse(b"\x1bOA\x1bOB"), [Key::Up, Key::Down]);
        assert_eq!(parse(b"\x1b[C\x1b[D"), [Key::Right, Key::Left]);
        assert_eq!(parse(b"\x1bOC\x1bOD"), [Key::Right, Key::Left]);
        assert_eq!(parse(b"\r"), [Key::Enter]);
        assert_eq!(
            parse(b"p?x"),
            [Key::Char('p'), Key::Char('?'), Key::Char('x')]
        );
        assert_eq!(parse(b"\x1bi"), [Key::Menu]);
        assert_eq!(parse("\u{2c6}".as_bytes()), [Key::Menu]);
        assert_eq!(parse(b"\x1b[105;3u"), [Key::Menu]);
        assert_eq!(
            parse(b"\x1b[1;5A"),
            [Key::Up],
            "a modified arrow is still the arrow (it used to be dropped)"
        );
        assert_eq!(
            parse(b"\x1bx"),
            Vec::<Key>::new(),
            "Alt-x is not the panel's"
        );
        assert_eq!(parse(b"\x03a"), [Key::Char('a')], "controls are dropped");
        assert_eq!(
            parse(b"a\x7f\x08"),
            [Key::Char('a'), Key::Backspace, Key::Backspace]
        );
    }
}
