//! The keys the panel reads from the terminal, parsed from the bytes the person types.

use super::model::Key;

/// The panel's key, Option+i, as terminals send it: Esc then `i` (Option as Meta), the
/// macOS dead-key `ˆ` (U+02C6), or the CSI-u form (`ESC [ 105 ; 3 u`).
const MENU_KEYS: [&[u8]; 3] = [b"\x1bi", "\u{2c6}".as_bytes(), b"\x1b[105;3u"];

/// Whether this input is exactly the panel's key (the only thing the panel reads at rest:
/// anything else belongs to the shell and is forwarded whole).
pub(super) fn is_menu_key(bytes: &[u8]) -> bool {
    MENU_KEYS.contains(&bytes)
}

/// The keys in one read while the panel is open. A lone Esc is Esc; arrows are
/// `ESC [ A` or `ESC O A`; Enter is a carriage return or a newline; every other
/// sequence is skipped; text is one `Char` per character.
pub(super) fn parse(bytes: &[u8]) -> Vec<Key> {
    let mut keys = Vec::new();
    let text = String::from_utf8_lossy(bytes);
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\x1b' => match chars.peek().copied() {
                None => keys.push(Key::Esc),
                Some('i') => {
                    chars.next();
                    keys.push(Key::Menu);
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
                    match (intro, last, params.as_str()) {
                        (_, Some('A'), "") | ('O', Some('A'), _) => keys.push(Key::Up),
                        (_, Some('B'), "") | ('O', Some('B'), _) => keys.push(Key::Down),
                        ('[', Some('u'), "105;3") => keys.push(Key::Menu),
                        _ => {}
                    }
                }
                // Alt plus another key: not the panel's.
                Some(_) => {
                    chars.next();
                }
            },
            '\u{2c6}' => keys.push(Key::Menu),
            '\r' | '\n' => keys.push(Key::Enter),
            c if c.is_control() => {}
            c => keys.push(Key::Char(c)),
        }
    }
    keys
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_menu_key_is_recognised_in_its_three_forms_and_only_alone() {
        assert!(is_menu_key(b"\x1bi"));
        assert!(is_menu_key("\u{2c6}".as_bytes()));
        assert!(is_menu_key(b"\x1b[105;3u"));
        assert!(!is_menu_key(b"i"));
        assert!(!is_menu_key(b"ls\x1bi"));
        assert!(!is_menu_key(b"\x1b"));
    }

    #[test]
    fn keys_are_parsed_from_one_read() {
        assert_eq!(parse(b"\x1b"), [Key::Esc]);
        assert_eq!(parse(b"\x1b[A\x1b[B"), [Key::Up, Key::Down]);
        assert_eq!(parse(b"\x1bOA\x1bOB"), [Key::Up, Key::Down]);
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
            Vec::<Key>::new(),
            "modified arrows are skipped"
        );
        assert_eq!(
            parse(b"\x1bx"),
            Vec::<Key>::new(),
            "Alt-x is not the panel's"
        );
        assert_eq!(parse(b"\x03a"), [Key::Char('a')], "controls are dropped");
    }
}
