//! The panel as a grid of cells: the rounded box, its shadow, and the bytes that paint them.
//!
//! Pure: the width, the screen size and the look are arguments, so the geometry is tested
//! without a terminal. The layout is the approved design's (`band2.py`): a rounded box in
//! the top right corner, a title in the top border, one-cell inset rows, a soft shadow one
//! cell right and below that keeps the character under it.

use std::fmt::Write as _;

use console::measure_text_width;

use crate::card::{Look, Rgb, palette};

/// The panel's colours (the design's tokens). The accents are the shared [`palette`].
pub(super) const BORDER: Rgb = (0x6d, 0x5b, 0xd0);
pub(super) const PBG: Rgb = (0x1c, 0x1e, 0x2e);
pub(super) const SEL: Rgb = (0x2b, 0x2d, 0x4a);
pub(super) const TXT: Rgb = (0xd7, 0xda, 0xe0);
pub(super) const MUTE: Rgb = palette::DIM;
pub(super) const FAINT: Rgb = (0x56, 0x5a, 0x70);
pub(super) const KEY: Rgb = palette::CYAN;
pub(super) const GREEN: Rgb = palette::GREEN;
pub(super) const AMBER: Rgb = palette::AMBER;
pub(super) const RED: Rgb = (0xf8, 0x71, 0x71);
pub(super) const ACC: Rgb = palette::VIOLET;
const SHADOW_FG: Rgb = (0x3a, 0x3d, 0x52);
const SHADOW_BG: Rgb = (0x10, 0x11, 0x19);

/// Below this many columns the panel is not drawn at all.
pub(super) const MIN_COLS: usize = 24;

/// One screen cell of the panel: one column wide, always.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Cell {
    pub(super) ch: char,
    pub(super) fg: Rgb,
    pub(super) bg: Rgb,
    pub(super) bold: bool,
}

impl Cell {
    pub(super) fn new(ch: char, fg: Rgb, bg: Rgb, bold: bool) -> Self {
        Self { ch, fg, bg, bold }
    }
}

/// A run of text in one colour.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Span {
    pub(super) text: String,
    pub(super) fg: Rgb,
    pub(super) bold: bool,
}

/// Text in a colour.
pub(super) fn sp(text: &str, fg: Rgb) -> Span {
    Span {
        text: one_wide(text),
        fg,
        bold: false,
    }
}

/// Bold text in a colour.
pub(super) fn bold(text: &str, fg: Rgb) -> Span {
    Span {
        bold: true,
        ..sp(text, fg)
    }
}

/// `text` with every character that is not exactly one column wide (controls, wide and
/// zero-width ones) replaced by a middle dot, so the grid stays one character per cell.
/// Ask titles come from outside; this is also what keeps an escape out of the panel.
pub(super) fn one_wide(text: &str) -> String {
    text.chars()
        .map(|c| {
            if !c.is_control() && measure_text_width(c.encode_utf8(&mut [0; 4])) == 1 {
                c
            } else {
                '·'
            }
        })
        .collect()
}

/// A body row: its spans and whether it is the selected one.
#[derive(Debug, Clone)]
pub(super) struct Line {
    pub(super) spans: Vec<Span>,
    pub(super) selected: bool,
}

/// What a box shows.
#[derive(Debug, Clone)]
pub(super) struct BoxSpec {
    pub(super) width: usize,
    pub(super) title: Vec<Span>,
    pub(super) right: Vec<Span>,
    pub(super) body: Vec<Line>,
    pub(super) footer: Option<Vec<Span>>,
}

/// A screen rectangle, zero-based.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Rect {
    pub(super) top: usize,
    pub(super) left: usize,
    pub(super) height: usize,
    pub(super) width: usize,
}

impl Rect {
    /// The rectangle grown by the shadow (one cell right and below), clipped to the screen.
    pub(super) fn with_shadow(self, rows: usize, cols: usize) -> Rect {
        Rect {
            height: (self.height + 1).min(rows.saturating_sub(self.top)),
            width: (self.width + 1).min(cols.saturating_sub(self.left)),
            ..self
        }
    }
}

/// Where a mouse report landed, relative to a box.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Hit {
    /// Not on the box.
    Outside,
    /// On its border (or its title row).
    Frame,
    /// On this line of its body (zero is the first row under the title); the footer line, when
    /// there is one, counts after the body.
    Line(usize),
}

impl Rect {
    /// Where the cell (`col`, `row`) is on this box: its first row is the title border, its
    /// last the bottom border, the columns at both ends are the side borders.
    pub(super) fn hit(self, col: usize, row: usize) -> Hit {
        let inside = (self.left..self.left + self.width).contains(&col)
            && (self.top..self.top + self.height).contains(&row);
        if !inside {
            return Hit::Outside;
        }
        let (down, across) = (row - self.top, col - self.left);
        if down == 0 || down + 1 == self.height || across == 0 || across + 1 == self.width {
            Hit::Frame
        } else {
            Hit::Line(down - 1)
        }
    }
}

/// A laid out box at a place on the screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Floating {
    pub(super) rect: Rect,
    pub(super) cells: Vec<Vec<Cell>>,
}

fn to_cells(spans: &[Span], bg: Rgb) -> Vec<Cell> {
    spans
        .iter()
        .flat_map(|s| s.text.chars().map(move |c| Cell::new(c, s.fg, bg, s.bold)))
        .collect()
}

fn fit(mut cells: Vec<Cell>, len: usize, bg: Rgb) -> Vec<Cell> {
    cells.truncate(len);
    cells.resize(len, Cell::new(' ', TXT, bg, false));
    cells
}

/// The spans cut to `max` characters, the last one an ellipsis when something was cut.
fn cut(spans: &[Span], max: usize) -> Vec<Span> {
    let total: usize = spans.iter().map(|s| s.text.chars().count()).sum();
    if total <= max {
        return spans.to_vec();
    }
    let keep = max.saturating_sub(1);
    let mut out = Vec::new();
    let mut left = keep;
    let mut last_fg = TXT;
    for s in spans {
        if left == 0 {
            break;
        }
        let text: String = s.text.chars().take(left).collect();
        left -= text.chars().count();
        last_fg = s.fg;
        out.push(Span { text, ..s.clone() });
    }
    out.push(sp("…", last_fg));
    out
}

/// Lays the box out at `top`, `left`. Width is `spec.width`; every row is exactly that many
/// cells. A title too long for the border loses the right header first, then is cut.
pub(super) fn lay_out(spec: &BoxSpec, top: usize, left: usize) -> Floating {
    let width = spec.width.max(8);
    let inner = width - 2;
    let border = |c: char| Cell::new(c, BORDER, PBG, false);
    let len = |s: &[Span]| s.iter().map(|x| x.text.chars().count()).sum::<usize>();

    // The top border: "─ title " then fill then " right ─" (or a lone "─").
    let mut title = spec.title.clone();
    let mut right = spec.right.clone();
    let framed = |right: &[Span]| if right.is_empty() { 1 } else { len(right) + 4 };
    if 3 + len(&title) + framed(&right) > inner {
        right.clear();
    }
    if 3 + len(&title) + framed(&right) > inner {
        title = cut(&title, inner.saturating_sub(4));
    }
    let mut top_row = vec![border('╭')];
    top_row.extend(to_cells(&[sp("─ ", BORDER)], PBG));
    top_row.extend(to_cells(&title, PBG));
    top_row.extend(to_cells(&[sp(" ", BORDER)], PBG));
    let tail: Vec<Cell> = if right.is_empty() {
        to_cells(&[sp("─", BORDER)], PBG)
    } else {
        let mut t = to_cells(&[sp(" ", BORDER)], PBG);
        t.extend(to_cells(&right, PBG));
        t.extend(to_cells(&[sp(" ─", BORDER)], PBG));
        t
    };
    let fill = inner.saturating_sub(top_row.len() - 1 + tail.len());
    top_row.extend(std::iter::repeat_n(border('─'), fill));
    top_row.extend(tail);
    top_row.truncate(inner + 1);
    top_row.push(border('╮'));

    let mut rows = vec![top_row];
    for line in &spec.body {
        let bg = if line.selected { SEL } else { PBG };
        let lead = if line.selected {
            sp("▎", ACC)
        } else {
            sp(" ", TXT)
        };
        let mut spans = vec![lead];
        spans.extend(line.spans.iter().cloned());
        let mut cells = to_cells(&spans, bg);
        if line.selected {
            for c in &mut cells {
                c.bold |= c.fg == TXT;
            }
        }
        let mut row = vec![border('│'), Cell::new(' ', TXT, PBG, false)];
        row.extend(fit(cells, inner - 1, bg));
        row.push(border('│'));
        rows.push(row);
    }
    if let Some(footer) = &spec.footer {
        let mut spans = vec![sp("  ", MUTE)];
        spans.extend(footer.iter().cloned());
        let mut row = vec![border('│')];
        row.extend(fit(to_cells(&spans, PBG), inner, PBG));
        row.push(border('│'));
        rows.push(row);
    }
    let mut bottom = vec![border('╰')];
    bottom.extend(std::iter::repeat_n(border('─'), inner));
    bottom.push(border('╯'));
    rows.push(bottom);

    Floating {
        rect: Rect {
            top,
            left,
            height: rows.len(),
            width,
        },
        cells: rows,
    }
}

/// The box in the top right corner of a `cols` wide screen, its shadow still on screen;
/// `None` when the screen is too narrow for the panel.
pub(super) fn place(spec: &BoxSpec, cols: usize) -> Option<Floating> {
    if cols < MIN_COLS {
        return None;
    }
    let spec = BoxSpec {
        width: spec.width.min(cols - 2),
        ..spec.clone()
    };
    Some(lay_out(&spec, 0, cols - spec.width - 1))
}

/// A shadow cell: the character that was there, dimmed.
fn shadow_cell(ch: char) -> Cell {
    Cell::new(ch, SHADOW_FG, SHADOW_BG, false)
}

/// The shadow's cells as (row, column, cell), clipped to a `rows` by `cols` screen:
/// the column right of the box below its top edge, then the row under it.
/// `under(row, col)` is the character the screen holds there.
pub(super) fn shadow(
    f: &Floating,
    rows: usize,
    cols: usize,
    under: &dyn Fn(usize, usize) -> char,
) -> Vec<(usize, usize, Cell)> {
    let Rect {
        top,
        left,
        height,
        width,
    } = f.rect;
    let mut out = Vec::new();
    let mut push = |r: usize, c: usize| {
        if r < rows && c < cols {
            out.push((r, c, shadow_cell(under(r, c))));
        }
    };
    for r in top + 1..top + height {
        push(r, left + width);
    }
    for c in left + 1..=left + width {
        push(top + height, c);
    }
    out
}

/// The bytes that draw `segments` (a row, a column, its cells): a cursor move, then each
/// run of one style once. They end with a reset; the caller saves and restores the cursor.
pub(super) fn paint(look: Look, segments: &[(usize, usize, Vec<Cell>)]) -> Vec<u8> {
    let mut out = String::new();
    for (row, col, cells) in segments {
        let _ = write!(out, "\x1b[{};{}H", row + 1, col + 1);
        let mut i = 0;
        while i < cells.len() {
            let first = cells[i];
            let mut text = String::new();
            while i < cells.len()
                && (cells[i].fg, cells[i].bg, cells[i].bold) == (first.fg, first.bg, first.bold)
            {
                text.push(cells[i].ch);
                i += 1;
            }
            let mut style = look.rgb(first.fg, Some(first.bg));
            if first.bold {
                style = style.bold();
            }
            let _ = write!(out, "{}", style.apply_to(text));
        }
    }
    out.push_str("\x1b[0m");
    out.into_bytes()
}

/// The box and its shadow as paint segments for a `rows` by `cols` screen: the box rows
/// that are on screen, then the shadow.
pub(super) fn segments(
    f: &Floating,
    rows: usize,
    cols: usize,
    under: &dyn Fn(usize, usize) -> char,
) -> Vec<(usize, usize, Vec<Cell>)> {
    let mut out = Vec::new();
    for (i, row) in f.cells.iter().enumerate() {
        let r = f.rect.top + i;
        if r < rows && f.rect.left < cols {
            let room = cols - f.rect.left;
            out.push((r, f.rect.left, row.iter().copied().take(room).collect()));
        }
    }
    for (r, c, cell) in shadow(f, rows, cols, under) {
        out.push((r, c, vec![cell]));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(row: &[Cell]) -> String {
        row.iter().map(|c| c.ch).collect()
    }

    fn pill(width: usize) -> BoxSpec {
        BoxSpec {
            width,
            title: vec![sp("◆ ", ACC), bold("iohr", ACC)],
            right: vec![],
            body: vec![Line {
                spans: vec![sp("auto ", MUTE), sp("●", GREEN), sp(" on", GREEN)],
                selected: false,
            }],
            footer: None,
        }
    }

    #[test]
    fn the_box_is_in_the_top_right_corner_at_80_and_100_columns() {
        for cols in [80, 100] {
            let f = place(&pill(44), cols).expect("fits");
            assert_eq!((f.rect.top, f.rect.left), (0, cols - 44 - 1));
            assert_eq!(f.rect.height, 3);
            assert!(
                f.cells.iter().all(|r| r.len() == 44),
                "every row is 44 cells"
            );
            assert!(text(&f.cells[0]).starts_with("╭─ ◆ iohr "));
            assert!(text(&f.cells[0]).ends_with("─╮"));
            assert!(text(&f.cells[1]).starts_with("│ "));
            assert!(text(&f.cells[1]).contains("auto ● on"));
            assert_eq!(text(&f.cells[2]), format!("╰{}╯", "─".repeat(42)));
        }
    }

    #[test]
    fn a_narrow_screen_clamps_the_width_and_a_tiny_one_hides_the_panel() {
        let f = place(&pill(44), 30).expect("clamped");
        assert_eq!(f.rect.width, 28);
        assert_eq!(f.rect.left, 1);
        assert!(f.cells.iter().all(|r| r.len() == 28));
        assert!(place(&pill(44), 23).is_none());
    }

    #[test]
    fn a_long_title_loses_the_right_header_then_is_cut_with_an_ellipsis() {
        let mut spec = pill(24);
        spec.title = vec![bold("a very long title indeed", ACC)];
        spec.right = vec![sp("3 waiting", AMBER)];
        let f = lay_out(&spec, 0, 0);
        let top = text(&f.cells[0]);
        assert_eq!(top.chars().count(), 24);
        assert!(!top.contains("waiting"), "right header dropped: {top}");
        assert!(top.contains('…'), "title cut: {top}");
        assert!(top.ends_with("─╮"));
        // With room, the right header stays, framed by a space and a dash.
        let mut roomy = pill(44);
        roomy.right = vec![sp("3 waiting", AMBER)];
        let top = text(&lay_out(&roomy, 0, 0).cells[0]);
        assert!(top.ends_with(" 3 waiting ─╮"), "{top}");
    }

    #[test]
    fn the_selected_row_has_the_bar_the_tint_and_bold_text() {
        let mut spec = pill(44);
        spec.body[0].selected = true;
        spec.body[0].spans = vec![sp("Auto mode", TXT)];
        let f = lay_out(&spec, 0, 0);
        let row = &f.cells[1];
        assert_eq!((row[2].ch, row[2].fg, row[2].bg), ('▎', ACC, SEL));
        assert!(row[3].bold && row[3].bg == SEL, "text is bold on the tint");
        assert_eq!(row[1].bg, PBG, "the inset space keeps the panel colour");
        assert_eq!(row[0].ch, '│');
        assert_eq!(row[43].ch, '│');
    }

    #[test]
    fn the_shadow_is_one_cell_right_and_below_and_keeps_the_character_beneath() {
        let f = lay_out(&pill(10), 0, 20);
        let sh = shadow(&f, 24, 80, &|r, c| {
            char::from(b'a' + u8::try_from((r + c) % 26).unwrap())
        });
        // Height 3: right column rows 1 and 2, then the bottom row, columns 21..=30.
        assert_eq!(sh.len(), 2 + 10);
        assert!(sh.iter().any(|&(r, c, _)| (r, c) == (1, 30)));
        assert!(sh.iter().any(|&(r, c, _)| (r, c) == (3, 21)));
        assert!(
            !sh.iter().any(|&(r, c, _)| (r, c) == (0, 30)),
            "not beside the top edge"
        );
        let (_, _, cell) = sh[0];
        assert_eq!((cell.fg, cell.bg), (SHADOW_FG, SHADOW_BG));
        assert_eq!(
            cell.ch, 'f',
            "row 1 + column 30 = 31, 31 % 26 = 5, the sixth letter"
        );
    }

    #[test]
    fn the_shadow_is_clipped_at_the_screen_edge() {
        // A box whose right edge is the last column: no shadow column; on a screen exactly as
        // tall as the box: no shadow row.
        let f = lay_out(&pill(10), 0, 70);
        let sh = shadow(&f, 3, 80, &|_, _| ' ');
        assert!(sh.iter().all(|&(r, c, _)| r < 3 && c < 80), "{sh:?}");
        assert!(
            sh.is_empty(),
            "right column off screen and bottom row off screen: {sh:?}"
        );
        let f = lay_out(&pill(10), 0, 69);
        let sh = shadow(&f, 3, 80, &|_, _| ' ');
        assert_eq!(sh.len(), 2, "only the right column fits");
        assert_eq!(f.rect.with_shadow(3, 80).height, 3);
    }

    #[test]
    fn painted_bytes_move_the_cursor_and_group_runs_of_one_style() {
        let f = lay_out(&pill(10), 0, 20);
        let look = Look {
            fancy: true,
            truecolor: true,
        };
        let bytes = String::from_utf8(paint(look, &[(0, 20, f.cells[0].clone())])).expect("utf8");
        assert!(bytes.starts_with("\x1b[1;21H"));
        assert!(
            bytes.contains("38;2;109;91;208"),
            "border colour in 24-bit: {bytes:?}"
        );
        assert!(bytes.ends_with("\x1b[0m"));
        let look = Look {
            fancy: true,
            truecolor: false,
        };
        let bytes = String::from_utf8(paint(look, &[(0, 20, f.cells[0].clone())])).expect("utf8");
        assert!(bytes.contains("38;5;"), "256-colour fallback: {bytes:?}");
        assert!(!bytes.contains("38;2;"));
    }

    #[test]
    fn outside_text_is_one_column_per_character() {
        assert_eq!(one_wide("ok\x1b[31m"), "ok·[31m");
        assert_eq!(one_wide("日本"), "··");
        assert_eq!(sp("a\tb", TXT).text, "a·b");
    }
}
