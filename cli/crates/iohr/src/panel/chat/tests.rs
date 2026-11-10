use super::*;

fn now() -> Instant {
    Instant::now()
}

fn idle(chat: &mut Chat) {
    chat.message(
        Msg::Idle {
            mode: "auto".into(),
            ring: vec!["manual".into(), "plan".into(), "auto".into()],
        },
        now(),
    );
}

fn typed(chat: &mut Chat, text: &str) -> Vec<Do> {
    chat.feed(text.as_bytes(), now())
}

fn row_text(laid: &Laid, row: usize) -> String {
    laid.floating.cells[row].iter().map(|c| c.ch).collect()
}

fn osc(body: &str) -> Vec<u8> {
    format!("\x1b]7771;chat;{body}\x07").into_bytes()
}

#[test]
fn reports_are_taken_out_of_the_output() {
    let mut held = Vec::new();
    let mut chunk = b"before".to_vec();
    chunk.extend(osc("idle;auto;manual,plan,auto"));
    chunk.extend(b"after");
    let (out, msgs) = extract(&mut held, &chunk);
    assert_eq!(out, b"beforeafter");
    assert_eq!(
        msgs,
        vec![Msg::Idle {
            mode: "auto".into(),
            ring: vec!["manual".into(), "plan".into(), "auto".into()]
        }]
    );
    assert_eq!(held, Vec::<u8>::new());
}

#[test]
fn a_report_split_across_reads_is_held_until_it_ends() {
    let whole = osc("step;3;-;reading 3 files");
    for cut in 1..whole.len() {
        let mut held = Vec::new();
        let (a, m1) = extract(&mut held, &whole[..cut]);
        let (b, m2) = extract(&mut held, &whole[cut..]);
        assert!(a.is_empty() && b.is_empty(), "cut at {cut}: {a:?} {b:?}");
        let all: Vec<Msg> = m1.into_iter().chain(m2).collect();
        assert_eq!(
            all,
            vec![Msg::Step {
                n: 3,
                total: None,
                label: "reading 3 files".into()
            }],
            "cut at {cut}"
        );
    }
}

#[test]
fn other_escapes_and_a_lone_prefix_start_pass_through() {
    let mut held = Vec::new();
    let (out, msgs) = extract(&mut held, b"\x1b[31mred\x1b]0;title\x07\x1b");
    // The lone ESC could start a report, so it waits; everything before it goes.
    assert_eq!(out, b"\x1b[31mred\x1b]0;title\x07");
    assert_eq!(msgs, Vec::<Msg>::new());
    let (out, _) = extract(&mut held, b"[0m");
    assert_eq!(out, b"\x1b[0m");
}

#[test]
fn unknown_verbs_are_swallowed_and_a_runaway_one_is_not_held_forever() {
    let mut held = Vec::new();
    let (out, msgs) = extract(&mut held, &osc("nonsense;1"));
    assert!(out.is_empty() && msgs.is_empty() && held.is_empty());

    let mut runaway = PREFIX.to_vec();
    runaway.extend(vec![b'x'; MAX_BODY + 10]);
    let (out, _) = extract(&mut held, &runaway);
    assert_eq!(held, Vec::<u8>::new());
    assert_eq!(out.len(), runaway.len());
}

#[test]
fn a_step_label_loses_its_control_characters() {
    let mut held = Vec::new();
    let (_, msgs) = extract(&mut held, &osc("step;1;4;a\x1b[31mb"));
    // The ESC ends nothing; BEL is the end, so the label is "a" + "[31mb" without the ESC.
    assert_eq!(
        msgs,
        vec![Msg::Step {
            n: 1,
            total: Some(4),
            label: "a[31mb".into()
        }]
    );
}

#[test]
fn one_enter_sends_a_question_and_two_question_marks_send_it_deeper() {
    let mut chat = Chat::new(true, false);
    idle(&mut chat);
    chat.message(Msg::Focus, now());
    assert!(chat.focus);
    let did = typed(&mut chat, "why is the roll slow\r");
    assert_eq!(
        did.last(),
        Some(&Do::Send {
            line: "? why is the roll slow".into(),
            mode: "auto".into()
        })
    );
    assert!(!chat.focus && !chat.at_prompt);

    idle(&mut chat);
    chat.message(Msg::Focus, now());
    chat.message(Msg::Focus, now());
    let did = typed(&mut chat, "and the retries\r");
    assert_eq!(
        did.last(),
        Some(&Do::Send {
            line: "?? and the retries".into(),
            mode: "auto".into()
        })
    );
}

#[test]
fn a_question_mark_typed_first_in_the_box_means_deeper() {
    let mut chat = Chat::new(true, false);
    idle(&mut chat);
    chat.message(Msg::Focus, now());
    typed(&mut chat, "?");
    assert!(chat.deeper && chat.text.is_empty());
    typed(&mut chat, "why?");
    assert_eq!(chat.text.iter().collect::<String>(), "why?");
}

#[test]
fn nothing_is_sent_while_the_shell_is_busy_or_the_line_is_empty() {
    let mut chat = Chat::new(true, false);
    idle(&mut chat);
    chat.message(Msg::Focus, now());
    assert_eq!(typed(&mut chat, "\r"), vec![Do::None]);
    assert!(!chat.focus, "Enter on an empty line gives the keys back");

    chat.message(Msg::Focus, now());
    typed(&mut chat, "hello");
    chat.at_prompt = false;
    assert_eq!(typed(&mut chat, "\r"), vec![Do::None]);
}

#[test]
fn esc_leaves_the_box_with_the_draft_and_ctrl_c_throws_it_away() {
    let mut chat = Chat::new(true, false);
    idle(&mut chat);
    chat.message(Msg::Focus, now());
    typed(&mut chat, "abc");
    typed(&mut chat, "\x1b");
    assert!(!chat.focus);
    assert_eq!(chat.text.iter().collect::<String>(), "abc");
    // The shell's `?` (or a click) brings the keys back to the same draft.
    chat.message(Msg::Focus, now());
    assert!(chat.focus);
    typed(&mut chat, "d");
    assert_eq!(chat.text.iter().collect::<String>(), "abcd");
    typed(&mut chat, "\x03");
    assert!(chat.focus && chat.text.is_empty());
    typed(&mut chat, "\x03");
    assert!(!chat.focus);
}

#[test]
fn the_switch_key_moves_the_keys_both_ways_and_keeps_the_draft() {
    let mut chat = Chat::new(true, false);
    idle(&mut chat);
    assert!(chat.switch() && chat.focus);
    typed(&mut chat, "half a question");
    assert!(chat.switch() && !chat.focus);
    let laid = chat.layout(80, 20, now());
    let input = row_text(&laid, 1);
    assert!(
        input.contains("half a question") && input.contains("draft"),
        "{input}"
    );
    assert!(
        row_text(&laid, 2).contains("chat"),
        "{}",
        row_text(&laid, 2)
    );
    assert!(chat.switch() && chat.focus);
    assert_eq!(chat.text.iter().collect::<String>(), "half a question");
    assert!(row_text(&chat.layout(80, 20, now()), 0).contains("typing here"));
    // A sent line starts again on the next focus.
    typed(&mut chat, "\r");
    idle(&mut chat);
    assert!(chat.switch());
    assert_eq!(chat.text, Vec::<char>::new());
}

#[test]
fn the_switch_key_waits_while_an_answer_runs_or_the_shell_is_busy() {
    let mut chat = Chat::new(true, false);
    assert!(!chat.switch(), "no hook yet");
    idle(&mut chat);
    chat.message(Msg::Begin, now());
    assert!(!chat.switch() && !chat.focus);
    chat.message(Msg::End, now());
    chat.message(Msg::Busy, now());
    assert!(!chat.switch());
}

#[test]
fn up_recalls_the_last_question_and_down_hands_over_to_the_card_menu() {
    let mut chat = Chat::new(true, false);
    idle(&mut chat);
    chat.message(Msg::Focus, now());
    typed(&mut chat, "first one\r");
    idle(&mut chat);
    chat.message(Msg::Focus, now());
    typed(&mut chat, "\x1b[A");
    assert_eq!(chat.text.iter().collect::<String>(), "first one");
    let did = typed(&mut chat, "\x1b[B");
    assert_eq!(did, vec![Do::Forward(b"\x1b[B".to_vec())]);
    assert!(!chat.focus);
}

#[test]
fn shift_tab_goes_round_the_modes_the_shell_reported() {
    let mut chat = Chat::new(true, false);
    idle(&mut chat);
    chat.message(Msg::Focus, now());
    typed(&mut chat, "\x1b[Z");
    assert_eq!(chat.mode, "manual");
    typed(&mut chat, "\x1b[Z");
    assert_eq!(chat.mode, "plan");
    typed(&mut chat, "\x1b[Z");
    assert_eq!(chat.mode, "auto");
}

#[test]
fn esc_and_ctrl_c_stop_an_answer_and_only_while_one_runs() {
    let mut chat = Chat::new(true, false);
    idle(&mut chat);
    assert_eq!(chat.press(&ChatKey::Esc, now()), Do::None);
    chat.message(Msg::Begin, now());
    assert_eq!(chat.press(&ChatKey::Esc, now()), Do::Stop);
    assert_eq!(chat.press(&ChatKey::CtrlC, now()), Do::Stop);
    chat.message(Msg::Back(true), now());
    assert_eq!(
        chat.press(&ChatKey::Esc, now()),
        Do::None,
        "while Io asks the person, keys belong to the terminal"
    );
    chat.message(Msg::End, now());
    assert!(!chat.answering());
}

#[test]
fn a_question_to_the_person_before_the_run_began_shows_in_the_status_row() {
    let mut chat = Chat::new(true, false);
    idle(&mut chat);
    chat.message(Msg::Focus, now());
    typed(&mut chat, "hi\r");
    assert!(chat.delivered.is_some());
    // The ask asks about a folder first (no `begin` yet): the box steps aside at once.
    chat.message(Msg::Back(true), now());
    assert!(chat.waiting_for_person() && chat.delivered.is_none());
    let laid = chat.layout(80, 20, now());
    assert!(
        row_text(&laid, 2).contains("Io asks you something"),
        "{}",
        row_text(&laid, 2)
    );
    chat.message(Msg::Back(false), now());
    let laid = chat.layout(80, 20, now());
    assert!(!row_text(&laid, 2).contains("Io asks you something"));
}

#[test]
fn editing_keys_work_in_the_line() {
    let mut chat = Chat::new(true, false);
    idle(&mut chat);
    chat.message(Msg::Focus, now());
    typed(&mut chat, "hello big world");
    typed(&mut chat, "\x17");
    assert_eq!(chat.text.iter().collect::<String>(), "hello big ");
    typed(&mut chat, "\x1b[D\x1b[D\x7f");
    assert_eq!(chat.text.iter().collect::<String>(), "hello bg ");
    typed(&mut chat, "\x15");
    assert_eq!(chat.text.iter().collect::<String>(), "g ");
    typed(&mut chat, "é");
    assert_eq!(
        chat.text.iter().collect::<String>(),
        "ég ",
        "typed at the cursor, which Ctrl-U left at the start"
    );
}

#[test]
fn a_paste_is_one_line() {
    let mut chat = Chat::new(true, false);
    idle(&mut chat);
    chat.message(Msg::Focus, now());
    typed(&mut chat, "\x1b[200~line one\nline two\x1b[201~");
    assert_eq!(chat.text.iter().collect::<String>(), "line one line two");
    assert!(chat.focus);
}

#[test]
fn the_box_shows_only_once_the_shell_hook_has_reported() {
    let mut chat = Chat::new(true, false);
    assert!(!chat.open() && !chat.folded());
    idle(&mut chat);
    assert!(chat.open());
    chat.toggle();
    assert!(chat.folded() && !chat.open());
    chat.toggle();
    assert!(chat.open());

    let off = Chat::new(false, false);
    assert!(!off.available());
}

#[test]
fn a_folded_box_does_not_take_the_question_mark() {
    let mut chat = Chat::new(true, true);
    idle(&mut chat);
    assert!(chat.folded());
    chat.message(Msg::Focus, now());
    assert!(chat.folded() && !chat.focus);
}

#[test]
fn a_running_command_takes_the_keys_back() {
    let mut chat = Chat::new(true, false);
    idle(&mut chat);
    chat.message(Msg::Focus, now());
    chat.message(Msg::Busy, now());
    assert!(!chat.focus && !chat.at_prompt);
}

#[test]
fn a_delivered_question_that_never_starts_can_be_given_up() {
    let mut chat = Chat::new(true, false);
    idle(&mut chat);
    chat.message(Msg::Focus, now());
    typed(&mut chat, "hi\r");
    assert!(chat.delivered.is_some() && chat.animating());
    idle(&mut chat);
    assert!(chat.delivered.is_none() && chat.at_prompt);
}

#[test]
fn a_question_nobody_started_is_given_up_after_the_limit() {
    let mut chat = Chat::new(true, false);
    idle(&mut chat);
    chat.message(Msg::Focus, now());
    let t = now();
    typed(&mut chat, "hi\r");
    assert!(!chat.expire(t), "not yet");
    assert!(!chat.at_prompt);
    assert!(chat.expire(t + DELIVER_LIMIT + Duration::from_millis(1)));
    assert!(chat.at_prompt && chat.delivered.is_none());
    assert!(!chat.expire(t + DELIVER_LIMIT * 2), "only once");
}

#[test]
fn the_rest_box_is_four_rows_with_the_hint_row() {
    let mut chat = Chat::new(true, false);
    idle(&mut chat);
    chat.message(Msg::Focus, now());
    typed(&mut chat, "why is the roll slow");
    let laid = chat.layout(80, 20, now());
    assert_eq!(laid.floating.cells.len(), ROWS);
    assert!(laid.floating.cells.iter().all(|r| r.len() == 80));
    let top = row_text(&laid, 0);
    assert!(
        top.contains("◆ iohr")
            && top.contains("ask · auto")
            && top.contains(&format!("⌄ hide {HIDE_LABEL}"))
    );
    assert!(row_text(&laid, 1).contains("❯ ? why is the roll slow"));
    let status = row_text(&laid, 2);
    assert!(
        status.contains("↵ send")
            && status.contains("esc terminal")
            && status.contains("^C clear")
            && status.contains("mode: auto")
    );
    assert!(status.contains("↑ last question"));
}

#[test]
fn a_sent_question_says_so_on_the_input_row() {
    let mut chat = Chat::new(true, false);
    idle(&mut chat);
    chat.message(Msg::Focus, now());
    typed(&mut chat, "why\r");
    let laid = chat.layout(80, 20, now());
    let input = row_text(&laid, 1);
    assert!(
        input.contains("❯ ? why") && input.contains("sent ✓"),
        "{input}"
    );
}

#[test]
fn while_answering_the_status_row_shows_progress_and_a_stop_button() {
    let mut chat = Chat::new(true, false);
    idle(&mut chat);
    let t0 = now();
    chat.message(Msg::Begin, t0);
    chat.message(
        Msg::Step {
            n: 2,
            total: Some(3),
            label: "reading 3 files".into(),
        },
        t0,
    );
    let laid = chat.layout(80, 20, t0 + Duration::from_millis(6100));
    let status = row_text(&laid, 2);
    assert!(status.contains("reading 3 files"), "{status}");
    assert!(
        status.contains("2/3") && status.contains("6.1s"),
        "{status}"
    );
    assert!(status.contains("■ Stop esc"), "{status}");
    assert!(status.contains('━') && status.contains('─'));
}

#[test]
fn with_no_total_the_bar_travels_and_the_count_is_steps_not_a_percentage() {
    let mut chat = Chat::new(true, false);
    idle(&mut chat);
    let t0 = now();
    chat.message(Msg::Begin, t0);
    chat.message(
        Msg::Step {
            n: 5,
            total: None,
            label: "reading 5 files".into(),
        },
        t0,
    );
    let heads: Vec<usize> = (0..6)
        .map(|k| {
            let laid = chat.layout(80, 20, t0 + Duration::from_millis(k * 240));
            let s = row_text(&laid, 2);
            assert!(s.contains("5 steps") && !s.contains('%'), "{s}");
            s.chars().position(|c| c == '━').expect("a bar")
        })
        .collect();
    assert!(
        heads.windows(2).any(|w| w[0] != w[1]),
        "it moves: {heads:?}"
    );
}

#[test]
fn the_stop_button_is_where_it_is_drawn() {
    let mut chat = Chat::new(true, false);
    idle(&mut chat);
    chat.message(Msg::Begin, now());
    for width in [60, 80, 120] {
        let laid = chat.layout(width, 20, now());
        let status = row_text(&laid, 2);
        let at = status.chars().position(|c| c == '■').expect("stop");
        assert_eq!(Chat::hit(&laid, at, 22), Hit::Stop, "width {width}");
        assert_eq!(Chat::hit(&laid, at + 9, 22), Hit::Stop, "width {width}");
        assert_eq!(Chat::hit(&laid, at - 2, 22), Hit::Outside, "width {width}");
    }
}

#[test]
fn hide_chip_and_input_clicks_land_on_what_is_drawn() {
    let mut chat = Chat::new(true, false);
    idle(&mut chat);
    let laid = chat.layout(80, 20, now());
    let top = row_text(&laid, 0);
    let hide = top.chars().position(|c| c == '⌄').expect("hide");
    assert_eq!(Chat::hit(&laid, hide, 20), Hit::Hide);
    assert_eq!(Chat::hit(&laid, hide + 7, 20), Hit::Hide);
    let chip = top.chars().position(|c| c == 'a').expect("chip");
    assert_eq!(Chat::hit(&laid, chip, 20), Hit::Chip);
    assert_eq!(Chat::hit(&laid, 10, 21), Hit::Input);
    assert_eq!(Chat::hit(&laid, 10, 19), Hit::Outside);
    assert_eq!(Chat::hit(&laid, 10, 23), Hit::Outside);
}

#[test]
fn a_narrow_box_still_has_four_equal_rows() {
    let mut chat = Chat::new(true, false);
    idle(&mut chat);
    chat.message(Msg::Focus, now());
    typed(&mut chat, &"x".repeat(200));
    for width in [24, 30, 40] {
        let laid = chat.layout(width, 10, now());
        assert_eq!(laid.floating.cells.len(), ROWS);
        assert!(
            laid.floating.cells.iter().all(|r| r.len() == width),
            "{width}"
        );
    }
}

#[test]
fn the_cursor_is_one_inverted_cell_while_the_box_has_the_keys() {
    let mut chat = Chat::new(true, false);
    idle(&mut chat);
    chat.message(Msg::Focus, now());
    typed(&mut chat, "ab");
    let laid = chat.layout(80, 20, now());
    let inverted: Vec<usize> = laid.floating.cells[1]
        .iter()
        .enumerate()
        .filter(|(_, c)| c.bg == TXT && c.fg == PBG)
        .map(|(i, _)| i)
        .collect();
    assert_eq!(inverted, vec![3 + 2 + 2 + 2]);
    chat.message(Msg::Busy, now());
    let laid = chat.layout(80, 20, now());
    assert!(laid.floating.cells[1].iter().all(|c| c.bg != TXT));
}

#[test]
fn the_folded_chip_names_the_key_and_shows_progress() {
    let mut chat = Chat::new(true, true);
    idle(&mut chat);
    let text: String = chat.chip_cells(now()).iter().map(|c| c.ch).collect();
    assert!(text.contains(HIDE_LABEL) && text.contains("iohr"), "{text}");
    chat.message(Msg::Begin, now());
    chat.message(
        Msg::Step {
            n: 4,
            total: None,
            label: "x".into(),
        },
        now(),
    );
    let text: String = chat.chip_cells(now()).iter().map(|c| c.ch).collect();
    assert!(
        text.contains("4 steps") && text.contains(HIDE_LABEL),
        "{text}"
    );
}

#[test]
fn keys_parse_the_way_a_terminal_sends_them() {
    let mut paste = false;
    assert_eq!(parse(b"\x1b", &mut paste), vec![ChatKey::Esc]);
    assert_eq!(
        parse(b"\x1b[A\x1bOB", &mut paste),
        vec![ChatKey::Up, ChatKey::Down]
    );
    assert_eq!(parse(b"\x1b[3~", &mut paste), vec![ChatKey::Delete]);
    assert_eq!(parse(b"\x1b[1;5C", &mut paste), vec![ChatKey::Right]);
    assert_eq!(parse(b"\x1bb", &mut paste), vec![]);
    assert_eq!(parse(b"\x03", &mut paste), vec![ChatKey::CtrlC]);
    assert_eq!(
        parse("é€".as_bytes(), &mut paste),
        vec![ChatKey::Char('é'), ChatKey::Char('€')]
    );
}

#[test]
fn a_key_cut_between_two_reads_is_held_not_eaten_or_typed() {
    let mut chat = Chat::new(true, false);
    idle(&mut chat);
    chat.focus_input();
    let before = chat.mode.clone();
    typed(&mut chat, "\x1b[");
    assert_eq!(chat.text, Vec::<char>::new());
    typed(&mut chat, "Z");
    assert_eq!(
        chat.text,
        Vec::<char>::new(),
        "the end of the key was typed"
    );
    assert_ne!(chat.mode, before, "Shift+Tab did not change the mode");
    // A UTF-8 character cut in two.
    chat.feed(&"é".as_bytes()[..1], now());
    chat.feed(&"é".as_bytes()[1..], now());
    assert_eq!(chat.text, vec!['é']);
    // A lone Esc is the Esc key, not a key waiting for more: it hands the keys back, keeping the draft.
    chat.feed(b"\x1b", now());
    assert!(!chat.focus);
    assert_eq!(chat.text, vec!['é']);
}

#[test]
fn a_question_written_at_a_clean_prompt_opens_the_box_with_its_text() {
    let mut chat = Chat::new(true, false);
    idle(&mut chat);
    assert!(chat.open_with(b"? hello", now()).is_some());
    assert!(chat.focus);
    assert_eq!(chat.text.iter().collect::<String>(), "hello");
    assert!(!chat.deeper);

    let mut chat = Chat::new(true, false);
    idle(&mut chat);
    assert!(
        chat.open_with(b"\x1b[200~?? why not\nmore\x1b[201~", now())
            .is_some()
    );
    assert!(chat.deeper);
    assert_eq!(chat.text.iter().collect::<String>(), "why not more");
    assert!(!chat.paste, "the paste was closed");
}

#[test]
fn only_a_question_mark_and_a_space_at_a_clean_prompt_open_the_box() {
    let mut chat = Chat::new(true, false);
    idle(&mut chat);
    for not in [&b"?"[..], b"?x", b"ls -la", b"- ", b" ? a", b"\x1b[Z"] {
        assert!(chat.open_with(not, now()).is_none(), "{not:?}");
    }
    // Typed on the line already: the line's.
    chat.forwarded();
    assert!(chat.open_with(b"? hello", now()).is_none());
    // The next prompt is clean again; type-ahead during a command is not.
    chat.message(Msg::Busy, now());
    chat.forwarded();
    idle(&mut chat);
    assert!(chat.open_with(b"? hello", now()).is_none());
    idle(&mut chat);
    assert!(chat.open_with(b"? hello", now()).is_some());
    // A folded box, an answer running, a shell not at its prompt: not the box's.
    let mut chat = Chat::new(true, true);
    idle(&mut chat);
    assert!(chat.open_with(b"? hi", now()).is_none());
    let mut chat = Chat::new(true, false);
    idle(&mut chat);
    chat.message(Msg::Busy, now());
    assert!(chat.open_with(b"? hi", now()).is_none());
}

#[test]
fn ls_la_is_never_a_key_of_the_box() {
    let mut chat = Chat::new(true, false);
    idle(&mut chat);
    assert!(chat.open_with(b"ls -la", now()).is_none());
    assert!(!chat.focus);
    chat.focus_input();
    typed(&mut chat, "ls -la");
    assert_eq!(chat.text.iter().collect::<String>(), "ls -la");
}
