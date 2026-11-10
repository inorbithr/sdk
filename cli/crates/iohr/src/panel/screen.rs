//! What the proxy knows of the child's screen: a vt100 model of it (to restore what the panel
//! covered and to tell the alternate screen), and a scanner that says whether a chunk of the
//! child's output ended between sequences (the only time the proxy may paint).

use super::cells::Rect;

/// The child's screen as the terminal shows it.
pub(super) struct Screen {
    parser: vt100::Parser,
}

impl Screen {
    pub(super) fn new(rows: usize, cols: usize) -> Self {
        Self {
            parser: vt100::Parser::new(clamp(rows), clamp(cols), 0),
        }
    }

    pub(super) fn feed(&mut self, bytes: &[u8]) {
        self.parser.process(bytes);
    }

    /// The screen as bytes that draw it again on an empty terminal of the same size (what the
    /// phone starts from when a share begins).
    pub(super) fn snapshot(&self) -> Vec<u8> {
        self.parser.screen().state_formatted()
    }

    pub(super) fn resize(&mut self, rows: usize, cols: usize) {
        self.parser.screen_mut().set_size(clamp(rows), clamp(cols));
    }

    /// The child is on the alternate screen (a full screen program): the panel stays away.
    pub(super) fn alt(&self) -> bool {
        self.parser.screen().alternate_screen()
    }

    /// Whether the program asked the terminal for mouse reports itself (a full screen program
    /// does; a shell does not).
    pub(super) fn uses_mouse(&self) -> bool {
        self.parser.screen().mouse_protocol_mode() != vt100::MouseProtocolMode::None
    }

    /// The sequences that put the program's own mouse mode back on the terminal, after the
    /// panel turned the reports off; empty when the program asked for none.
    pub(super) fn own_mouse(&self) -> Vec<u8> {
        let screen = self.parser.screen();
        let mode: &[u8] = match screen.mouse_protocol_mode() {
            vt100::MouseProtocolMode::None => return Vec::new(),
            vt100::MouseProtocolMode::Press => b"\x1b[?9h",
            vt100::MouseProtocolMode::PressRelease => b"\x1b[?1000h",
            vt100::MouseProtocolMode::ButtonMotion => b"\x1b[?1002h",
            vt100::MouseProtocolMode::AnyMotion => b"\x1b[?1003h",
        };
        let mut out = mode.to_vec();
        match screen.mouse_protocol_encoding() {
            vt100::MouseProtocolEncoding::Default => {}
            vt100::MouseProtocolEncoding::Utf8 => out.extend_from_slice(b"\x1b[?1005h"),
            vt100::MouseProtocolEncoding::Sgr => out.extend_from_slice(b"\x1b[?1006h"),
        }
        out
    }

    /// The character at a cell; a space for an empty cell, a wide character's second half, or
    /// a wide character (the shadow cannot dim half of one).
    pub(super) fn char_at(&self, row: usize, col: usize) -> char {
        let (Ok(r), Ok(c)) = (u16::try_from(row), u16::try_from(col)) else {
            return ' ';
        };
        match self.parser.screen().cell(r, c) {
            Some(cell) if !cell.is_wide() && !cell.is_wide_continuation() => {
                let mut chars = cell.contents().chars();
                match (chars.next(), chars.next()) {
                    (Some(ch), None) if !ch.is_control() => ch,
                    _ => ' ',
                }
            }
            _ => ' ',
        }
    }

    /// The bytes that put back what the child had in `rect`: for each row a cursor move, then
    /// every cell of it written out (blanks too, so what the panel drew is overwritten) with
    /// its colours and style, ending with a reset.
    pub(super) fn restore(&self, rect: Rect) -> Vec<u8> {
        let screen = self.parser.screen();
        let (rows, cols) = screen.size();
        let at = |n: usize| u16::try_from(n).unwrap_or(u16::MAX).min(cols);
        let left = at(rect.left);
        let right = at(rect.left + rect.width);
        let mut out = Vec::new();
        for r in rect.top..(rect.top + rect.height).min(usize::from(rows)) {
            let Ok(r) = u16::try_from(r) else { break };
            out.extend_from_slice(format!("\x1b[{};{}H\x1b[0m", r + 1, left + 1).as_bytes());
            let mut col = left;
            while col < right {
                let cell = screen.cell(r, col);
                let wide_fits = cell.is_some_and(vt100::Cell::is_wide) && col + 1 < right;
                match cell {
                    Some(c) if !c.is_wide_continuation() && (!c.is_wide() || wide_fits) => {
                        write_sgr(&mut out, c);
                        let text = c.contents();
                        if text.is_empty() {
                            out.push(b' ');
                        } else {
                            out.extend_from_slice(text.as_bytes());
                        }
                    }
                    _ => {
                        out.extend_from_slice(b"\x1b[0m ");
                    }
                }
                col += if wide_fits { 2 } else { 1 };
            }
            out.extend_from_slice(b"\x1b[0m");
        }
        out
    }
}

/// The style of one cell as a complete SGR (it starts from a reset).
fn write_sgr(out: &mut Vec<u8>, cell: &vt100::Cell) {
    let mut sgr = String::from("\x1b[0");
    for (on, code) in [
        (cell.bold(), "1"),
        (cell.dim(), "2"),
        (cell.italic(), "3"),
        (cell.underline(), "4"),
        (cell.inverse(), "7"),
    ] {
        if on {
            sgr.push(';');
            sgr.push_str(code);
        }
    }
    for (colour, base) in [(cell.fgcolor(), 30u8), (cell.bgcolor(), 40)] {
        match colour {
            vt100::Color::Default => {}
            vt100::Color::Idx(i) => {
                let _ = std::fmt::Write::write_fmt(&mut sgr, format_args!(";{};5;{i}", base + 8));
            }
            vt100::Color::Rgb(r, g, b) => {
                let _ = std::fmt::Write::write_fmt(
                    &mut sgr,
                    format_args!(";{};2;{r};{g};{b}", base + 8),
                );
            }
        }
    }
    sgr.push('m');
    out.extend_from_slice(sgr.as_bytes());
}

fn clamp(n: usize) -> u16 {
    u16::try_from(n).unwrap_or(u16::MAX).max(1)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Ground,
    Esc,
    Csi,
    /// A string (OSC, DCS, APC, PM, SOS) up to BEL or ST.
    Str,
    StrEsc,
    /// Continuation bytes still owed by a UTF-8 character.
    Utf8(u8),
}

/// Tracks where in an escape sequence the child's output stands, byte by byte.
#[derive(Debug)]
pub(super) struct Boundary {
    state: State,
}

impl Boundary {
    pub(super) fn new() -> Self {
        Self {
            state: State::Ground,
        }
    }

    /// Between sequences and characters: a cursor save and a drawing here cannot split one.
    pub(super) fn clean(&self) -> bool {
        self.state == State::Ground
    }

    pub(super) fn feed(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.step(b);
        }
    }

    fn step(&mut self, b: u8) {
        use State::{Csi, Esc, Ground, Str, StrEsc, Utf8};
        // CAN and SUB abort a sequence.
        if matches!(b, 0x18 | 0x1a) && self.state != Ground {
            self.state = Ground;
            return;
        }
        self.state = match (self.state, b) {
            // ESC starts a sequence anywhere but inside a string, where it may end it.
            (Ground | Utf8(_) | Esc | Csi, 0x1b) | (Esc, 0x20..=0x2f) => Esc,
            (Utf8(n), 0x80..=0xbf) => {
                if n <= 1 {
                    Ground
                } else {
                    Utf8(n - 1)
                }
            }
            (Utf8(_) | Ground, _) => match b {
                0xc0..=0xdf => Utf8(1),
                0xe0..=0xef => Utf8(2),
                0xf0..=0xf7 => Utf8(3),
                _ => Ground,
            },
            (Str | StrEsc, 0x1b) => StrEsc,
            (Str, 0x07) | (StrEsc, b'\\') | (Csi, 0x40..=0x7e) => Ground,
            (Esc | StrEsc, b'[') | (Csi, _) => Csi,
            (Esc, b']' | b'P' | b'X' | b'^' | b'_') | (Str, _) => Str,
            (Esc | StrEsc, _) => Ground,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clean_after(chunks: &[&[u8]]) -> Vec<bool> {
        let mut b = Boundary::new();
        chunks
            .iter()
            .map(|c| {
                b.feed(c);
                b.clean()
            })
            .collect()
    }

    #[test]
    fn a_chunk_that_ends_inside_a_sequence_is_not_a_clean_boundary() {
        assert_eq!(clean_after(&[b"hello\r\n"]), [true]);
        assert_eq!(clean_after(&[b"a\x1b", b"[31mb"]), [false, true]);
        assert_eq!(clean_after(&[b"a\x1b[1;3", b"1m"]), [false, true]);
        assert_eq!(clean_after(&[b"\x1b]0;title", b"\x07"]), [false, true]);
        assert_eq!(clean_after(&[b"\x1b]0;title\x1b", b"\\"]), [false, true]);
        assert_eq!(clean_after(&[b"\x1b(", b"B"]), [false, true]);
        assert_eq!(
            clean_after(&[b"\xe2\x94", b"\x80"]),
            [false, true],
            "split UTF-8"
        );
        assert_eq!(clean_after(&[b"\x1b[3\x18ok"]), [true], "CAN aborts");
        assert_eq!(clean_after(&[b"\x1bPq#0", b"\x1b\\"]), [false, true]);
    }

    #[test]
    fn the_model_reports_the_alternate_screen_and_restores_what_was_under_a_rect() {
        let mut s = Screen::new(5, 20);
        s.feed(b"\x1b[1;1Hhello world\x1b[2;1H\x1b[31mred\x1b[0m");
        assert!(!s.alt());
        assert_eq!(s.char_at(0, 4), 'o');
        assert_eq!(s.char_at(4, 0), ' ');
        assert_eq!(s.char_at(99, 99), ' ');
        let bytes = s.restore(Rect {
            top: 0,
            left: 6,
            height: 2,
            width: 5,
        });
        let mut check = vt100::Parser::new(5, 20, 0);
        check.process(b"\x1b[1;1H~~~~~~~~~~~~~~~\x1b[2;1H~~~~~~~~~~~~~~~");
        check.process(&bytes);
        assert_eq!(check.screen().contents_between(0, 6, 0, 11), "world");
        assert_eq!(check.screen().contents_between(0, 0, 0, 6), "~~~~~~");
        // Blank cells of the model overwrite what the panel drew, and colours come back.
        assert_eq!(check.screen().contents_between(1, 6, 1, 11).trim(), "");
        let mut red = vt100::Parser::new(5, 20, 0);
        red.process(b"\x1b[2;1H~~~~~");
        red.process(&s.restore(Rect {
            top: 1,
            left: 0,
            height: 1,
            width: 5,
        }));
        let cell = red.screen().cell(1, 0).unwrap();
        assert_eq!(cell.contents(), "r");
        assert_eq!(cell.fgcolor(), vt100::Color::Idx(1));
        assert_eq!(red.screen().cell(1, 3).unwrap().contents().trim(), "");
        s.feed(b"\x1b[?1049h");
        assert!(s.alt());
        s.feed(b"\x1b[?1049l");
        assert!(!s.alt());
    }
}
