//! What the `?` card asks back, and what it knows about the project before it answers.
//!
//! An answer used to stop at "not known". Now iohr gathers the facts it can (the git branch,
//! the last commits and the change counts, with its own `git`, never a shell for the model)
//! and what the project already has written down (its `inorbit.toml` description and its
//! `.inorbit/memory/` notes), and the model may end its answer with at most two questions only
//! the person can answer. A question is a small, validated JSON block the model cannot make
//! iohr act on beyond one thing: when the person answers, iohr (never the model) writes the
//! answer to the project's memory (and the purpose to `[project] description` in
//! `inorbit.toml`), so the next `?` knows it.
//!
//! Nothing here logs the question, the answer or the notes; an error says what failed, never
//! what was in it.

use std::fmt::Write as _;
use std::path::Path;

use serde::Serialize;
use serde_json::Value;

use crate::error::Error;
use crate::memory::drift;
use crate::memory::store::{Kind, Remembered, Store, remember};
use crate::memory::text::has_secret;
use crate::ui::{Choice, Ui};

/// Questions the model may ask in one answer.
pub(crate) const MAX_QUESTIONS: usize = 2;
/// Choices a question offers besides "Other".
pub(crate) const MIN_CHOICES: usize = 2;
pub(crate) const MAX_CHOICES: usize = 4;
/// Longest question and choice, in characters.
pub(crate) const ASK_MAX: usize = 160;
pub(crate) const CHOICE_MAX: usize = 60;
/// Longest typed answer, in characters.
pub(crate) const ANSWER_MAX: usize = 300;
/// Commits the facts name, and how long a subject may be.
pub(crate) const COMMITS: usize = 5;
const SUBJECT_MAX: usize = 120;
/// The bytes the project notes may take in the prompt.
pub(crate) const NOTES_MAX: usize = 3 * 1024;
const NOTE_MAX: usize = 240;
const NOTE_COUNT: usize = 12;

/// What a question is about, which decides where its answer is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum About {
    /// What the project is for: kept as a memory and as `[project] description`.
    Purpose,
    /// Anything else the person can tell: kept as a memory.
    Note,
}

/// One question for the person.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct Question {
    pub(crate) about: About,
    pub(crate) ask: String,
    /// Two to four short answers; "Other" is always offered besides.
    pub(crate) choices: Vec<String>,
}

fn clean(s: &str, max: usize) -> Option<String> {
    let s = s.trim();
    if s.is_empty() || s.chars().count() > max || s.chars().any(char::is_control) {
        return None;
    }
    Some(s.to_owned())
}

/// Validates the JSON the model wrote: `{"questions": [...]}` or the bare list. A question
/// that does not pass (no text, a control character, fewer than two or more than four
/// choices once "other" is removed, an unknown `about`) is dropped; at most
/// [`MAX_QUESTIONS`] are kept. A block that is not JSON gives none.
fn validate(json: &str) -> Vec<Question> {
    let Ok(v) = serde_json::from_str::<Value>(json.trim()) else {
        return Vec::new();
    };
    let list = match &v {
        Value::Array(a) => a.as_slice(),
        Value::Object(o) => o
            .get("questions")
            .and_then(Value::as_array)
            .map_or(&[][..], Vec::as_slice),
        _ => &[],
    };
    list.iter()
        .filter_map(|q| {
            let ask = clean(q.get("ask")?.as_str()?, ASK_MAX)?;
            let about = match q.get("about").and_then(Value::as_str).map(str::trim) {
                None | Some("note") => About::Note,
                Some("purpose") => About::Purpose,
                Some(_) => return None,
            };
            let mut choices: Vec<String> = Vec::new();
            for c in q.get("choices")?.as_array()? {
                let c = clean(c.as_str()?, CHOICE_MAX)?;
                // "Other" is the card's own last entry; the person's wording is typed there.
                if matches!(c.to_lowercase().as_str(), "other" | "other..." | "other…") {
                    continue;
                }
                if !choices.iter().any(|x| x.eq_ignore_ascii_case(&c)) {
                    choices.push(c);
                }
            }
            (MIN_CHOICES..=MAX_CHOICES)
                .contains(&choices.len())
                .then_some(Question {
                    about,
                    ask,
                    choices,
                })
        })
        .take(MAX_QUESTIONS)
        .collect()
}

/// Splits the model's answer into what is shown and the questions it asked: the last fenced
/// ```` ```questions ```` block is removed from the text (even when it is broken or never
/// closed: raw JSON is never shown) and validated. The text keeps everything else, trimmed.
pub(crate) fn split(text: &str) -> (String, Vec<Question>) {
    let lines: Vec<&str> = text.lines().collect();
    let opens = |l: &&str| l.trim().to_ascii_lowercase().starts_with("```questions");
    let Some(open) = lines.iter().rposition(opens) else {
        return (text.trim().to_owned(), Vec::new());
    };
    let close = lines[open + 1..]
        .iter()
        .position(|l| l.trim() == "```")
        .map(|i| open + 1 + i);
    let json = lines[open + 1..close.unwrap_or(lines.len())].join("\n");
    let mut kept: Vec<&str> = lines[..open].to_vec();
    if let Some(c) = close {
        kept.extend(&lines[c + 1..]);
    }
    (kept.join("\n").trim().to_owned(), validate(&json))
}

// ---- git facts ----

fn one_line(s: &str, max: usize) -> String {
    let s: String = s.chars().filter(|c| !c.is_control()).collect();
    let s = s.trim();
    if s.chars().count() <= max {
        s.to_owned()
    } else {
        let cut: String = s.chars().take(max.saturating_sub(3)).collect();
        format!("{cut}...")
    }
}

/// The counts of `git status --porcelain=v1`: staged, changed in the work tree, untracked and
/// conflicted paths. A path both staged and changed counts in both.
fn counts(porcelain: &str) -> (usize, usize, usize, usize) {
    let (mut staged, mut changed, mut untracked, mut conflicted) = (0, 0, 0, 0);
    for l in porcelain.lines().filter(|l| !l.starts_with("##")) {
        let mut c = l.chars();
        let (x, y) = (c.next().unwrap_or(' '), c.next().unwrap_or(' '));
        match (x, y) {
            ('?', '?') => untracked += 1,
            ('!', '!') => {}
            ('U', _) | (_, 'U') | ('A', 'A') | ('D', 'D') => conflicted += 1,
            _ => {
                if x != ' ' {
                    staged += 1;
                }
                if y != ' ' {
                    changed += 1;
                }
            }
        }
    }
    (staged, changed, untracked, conflicted)
}

/// The text of `<git>`: the branch, the last [`COMMITS`] commits (one line each) and the
/// change counts, from iohr's own `git` in `dir`. A folder outside a repository says so,
/// so the model never has to guess. Never fails: the facts are context, not a gate.
#[must_use]
pub(crate) fn git_facts(dir: &Path) -> String {
    let Some(status) = drift::git(dir, &["status", "--porcelain=v1", "-b"]) else {
        return "(not a git repository, or git could not be run here)".to_owned();
    };
    let branch = status
        .lines()
        .find_map(|l| l.strip_prefix("## "))
        .map_or_else(|| "unknown".to_owned(), |b| one_line(b, 120));
    let mut out = format!("branch: {branch}\n");
    let n = COMMITS.to_string();
    match drift::git(dir, &["log", "-n", &n, "--format=%h %s"]) {
        Some(log) if !log.trim().is_empty() => {
            let _ = writeln!(out, "last commits, newest first:");
            for l in log.lines().take(COMMITS) {
                let _ = writeln!(out, "{}", one_line(l, SUBJECT_MAX));
            }
        }
        _ => out.push_str("last commits: none yet\n"),
    }
    let (staged, changed, untracked, conflicted) = counts(&status);
    if staged + changed + untracked + conflicted == 0 {
        out.push_str("working tree: clean");
    } else {
        let _ = write!(
            out,
            "working tree: {staged} staged, {changed} changed, {untracked} untracked"
        );
        if conflicted > 0 {
            let _ = write!(out, ", {conflicted} conflicted");
        }
    }
    out
}

// ---- what the project already says ----

/// The `[project] description` of the project's `inorbit.toml`, when it has one.
#[must_use]
pub(crate) fn description(root: &Path) -> Option<String> {
    std::fs::read_to_string(root.join("inorbit.toml"))
        .ok()?
        .parse::<toml::Table>()
        .ok()?
        .get("project")?
        .get("description")?
        .as_str()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

/// The text of `<project>`: the description and the memory notes, newest first, within
/// [`NOTES_MAX`]. A project with neither says so plainly, which is what tells the model it
/// may ask.
#[must_use]
pub(crate) fn project_notes(root: &Path) -> String {
    let mut out = String::new();
    if let Some(d) = description(root) {
        let _ = writeln!(out, "purpose (inorbit.toml): {}", one_line(&d, NOTE_MAX));
    }
    let mut notes = Store::project(root).load().unwrap_or_default();
    notes.sort_by(|a, b| b.updated.cmp(&a.updated));
    for (count, e) in notes.iter().take(NOTE_COUNT).enumerate() {
        let line = format!("- {}\n", one_line(&e.text, NOTE_MAX));
        if out.len() + line.len() > NOTES_MAX {
            break;
        }
        if count == 0 {
            out.push_str("notes (.inorbit/memory):\n");
        }
        out.push_str(&line);
    }
    if out.is_empty() {
        "nothing is written down yet: no description in inorbit.toml and no notes".to_owned()
    } else {
        out.trim_end().to_owned()
    }
}

// ---- keeping an answer ----

/// Sets `[project] description` in `inorbit.toml`, keeping the rest of the file as it is.
/// Nothing is written when the project has no `inorbit.toml` (a memory still is).
fn set_description(root: &Path, text: &str) -> Result<bool, String> {
    let path = root.join("inorbit.toml");
    let Ok(old) = std::fs::read_to_string(&path) else {
        return Ok(false);
    };
    let mut doc: toml_edit::DocumentMut = old
        .parse()
        .map_err(|_| "inorbit.toml is not valid TOML, so the purpose was not added to it")?;
    if !doc.contains_key("project") {
        doc["project"] = toml_edit::table();
    }
    let Some(project) = doc["project"].as_table_like_mut() else {
        return Err("`project` in inorbit.toml is not a table".to_owned());
    };
    project.insert("description", toml_edit::value(text));
    std::fs::write(&path, doc.to_string())
        .map_err(|e| format!("cannot write inorbit.toml: {e}"))?;
    Ok(true)
}

/// Keeps what the person answered, in the project at `root`: one memory per answer
/// (`.inorbit/memory/`, through the same writer as `iohr memory remember`, which refuses
/// anything that looks like a secret) and, for the purpose, `[project] description` in
/// `inorbit.toml`. Returns one line per thing kept or refused, for the person to read.
pub(crate) fn save(root: &Path, answers: &[(Question, String)]) -> Vec<String> {
    let store = Store::project(root);
    let mut said = Vec::new();
    for (q, answer) in answers {
        let text = match q.about {
            About::Purpose => format!("This project is for: {answer}"),
            About::Note => format!("{} {answer}", q.ask),
        };
        match remember(
            &store,
            &text,
            Kind::Fact,
            "answered on the ? card",
            &[],
            false,
        ) {
            Ok(Remembered::Created { entry, .. } | Remembered::Confirmed(entry)) => {
                said.push(format!("saved to .inorbit/memory/{}.md", entry.id));
            }
            Err(e) => {
                said.push(format!("not saved: {}", plain(&e)));
                continue;
            }
        }
        if q.about == About::Purpose && !has_secret(answer) {
            match set_description(root, answer) {
                Ok(true) => said.push("purpose added to inorbit.toml".to_owned()),
                Ok(false) => {}
                Err(why) => said.push(why),
            }
        }
    }
    said
}

/// An error's own message, which for a refused memory is a fixed sentence.
fn plain(e: &Error) -> String {
    e.to_string()
}

/// Asks the person each question on the terminal, keyboard-driven like the action row: a digit
/// takes that choice, `o` types their own, `s` skips that question, and Esc or `q` dismisses
/// all of them (nothing is kept). None when nothing was answered.
pub(crate) fn ask_person(questions: &[Question]) -> Option<Vec<(Question, String)>> {
    let ui = Ui::detect(false);
    let pick = |value: String, label: &str, hint: &str, key: char| Choice {
        value,
        label: label.to_owned(),
        hint: hint.to_owned(),
        available: true,
        key: Some(key),
    };
    let mut answers = Vec::new();
    for q in questions {
        let mut choices: Vec<Choice> = q
            .choices
            .iter()
            .zip('1'..)
            .map(|(c, key)| pick(c.clone(), c, "", key))
            .collect();
        choices.push(pick("\u{0}other".to_owned(), "Other", "type your own", 'o'));
        choices.push(pick("\u{0}skip".to_owned(), "Skip", "keep nothing", 's'));
        let first = q.choices[0].clone();
        match Ui::select_keyed(&q.ask, &choices, &first) {
            Ok(Some(v)) if v == "\u{0}skip" => {}
            Ok(Some(v)) if v == "\u{0}other" => {
                let typed = ui.input("Your answer", "", |v| {
                    if v.is_empty() || v.chars().count() > ANSWER_MAX {
                        Err(format!("one to {ANSWER_MAX} characters"))
                    } else if v.chars().any(char::is_control) {
                        Err("no control characters".to_owned())
                    } else {
                        Ok(())
                    }
                });
                match typed {
                    Ok(t) => answers.push((q.clone(), t)),
                    Err(_) => return None,
                }
            }
            Ok(Some(v)) => answers.push((q.clone(), v)),
            // Esc, q, Ctrl-C or no terminal: dismissed.
            Ok(None) | Err(_) => return None,
        }
    }
    (!answers.is_empty()).then_some(answers)
}

/// The questions as plain lines for a terminal that cannot ask (a pipe, a script): the
/// question and its numbered choices, and how to keep an answer.
#[must_use]
pub(crate) fn plain_lines(questions: &[Question]) -> Vec<String> {
    let mut out = Vec::new();
    for q in questions {
        let choices: Vec<String> = q
            .choices
            .iter()
            .enumerate()
            .map(|(i, c)| format!("[{}] {c}", i + 1))
            .collect();
        out.push(format!(
            "Question for you: {}  {}  [other]",
            q.ask,
            choices.join("  ")
        ));
    }
    if !out.is_empty() {
        out.push(
            "To keep an answer: iohr memory remember \"<what you answered>\"  (then ask again)"
                .to_owned(),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use super::*;

    const BLOCK: &str = "I do not know what it is for.\n\n```questions\n{\"questions\":[{\"about\":\"purpose\",\"ask\":\"What is this project for?\",\"choices\":[\"A CLI tool\",\"A web service\",\"A library\"]}]}\n```\n";

    #[test]
    fn a_questions_block_is_taken_out_of_the_answer_and_validated() {
        let (text, qs) = split(BLOCK);
        assert_eq!(text, "I do not know what it is for.");
        assert_eq!(qs.len(), 1);
        assert_eq!(qs[0].about, About::Purpose);
        assert_eq!(qs[0].choices, ["A CLI tool", "A web service", "A library"]);
    }

    #[test]
    fn the_command_block_survives_and_the_questions_may_come_before_or_after_it() {
        let after = "Try this.\n```sh\ngit log\n```\n```questions\n[{\"ask\":\"Why?\",\"choices\":[\"a\",\"b\"]}]\n```";
        let (text, qs) = split(after);
        assert_eq!(text, "Try this.\n```sh\ngit log\n```");
        assert_eq!(qs.len(), 1);
        assert_eq!(
            qs[0].about,
            About::Note,
            "a bare list and no `about` still parse"
        );
    }

    #[test]
    fn at_most_two_questions_and_two_to_four_choices() {
        let q = |n: usize| {
            let c: Vec<String> = (0..n).map(|i| format!("\"c{i}\"")).collect();
            format!("{{\"ask\":\"Q{n}?\",\"choices\":[{}]}}", c.join(","))
        };
        let json = format!(
            "{{\"questions\":[{},{},{},{},{}]}}",
            q(1),
            q(2),
            q(5),
            q(4),
            q(3)
        );
        let qs = validate(&json);
        let asks: Vec<&str> = qs.iter().map(|q| q.ask.as_str()).collect();
        assert_eq!(
            asks,
            ["Q2?", "Q4?"],
            "one choice and five are dropped, then the cap of two"
        );
    }

    #[test]
    fn other_is_never_a_choice_of_the_model_and_bad_text_is_dropped() {
        let qs = validate(
            r#"{"questions":[{"ask":"Which?","choices":["a","Other","b","other"]},
                {"ask":"x\u0007y","choices":["a","b"]},
                {"ask":"Fine?","about":"secrets","choices":["a","b"]},
                {"ask":"","choices":["a","b"]}]}"#,
        );
        assert_eq!(qs.len(), 1);
        assert_eq!(qs[0].choices, ["a", "b"]);
    }

    #[test]
    fn a_broken_or_unclosed_block_shows_no_raw_json_and_gives_no_questions() {
        for text in [
            "Answer.\n```questions\n{not json\n```\nAfter.",
            "Answer.\n```questions\n{\"questions\":[",
        ] {
            let (shown, qs) = split(text);
            assert_eq!(qs, Vec::<Question>::new());
            assert!(!shown.contains("questions"), "{shown}");
            assert!(shown.starts_with("Answer."));
        }
        assert_eq!(split("No block.").1, Vec::new());
    }

    #[test]
    fn counts_read_porcelain() {
        let p = "## main...origin/main [ahead 1]\n M a\nM  b\nMM c\n?? d\n?? e\nUU f\n!! g\n";
        assert_eq!(counts(p), (2, 2, 2, 1));
    }

    fn git(dir: &Path, args: &[&str]) {
        let ok = std::process::Command::new("git")
            .args(["-c", "user.name=t", "-c", "user.email=t@example.com", "-C"])
            .arg(dir)
            .args(args)
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "git {args:?}");
    }

    #[test]
    fn git_facts_name_the_branch_the_commits_and_the_counts() {
        let t = tempfile::tempdir().unwrap();
        assert!(git_facts(t.path()).starts_with("(not a git repository"));
        git(t.path(), &["init", "-q", "-b", "trunk"]);
        assert!(git_facts(t.path()).contains("branch: No commits yet on trunk"));
        assert!(git_facts(t.path()).contains("last commits: none yet"));
        for i in 0..7 {
            std::fs::write(t.path().join(format!("f{i}")), "x").unwrap();
            git(t.path(), &["add", "."]);
            git(t.path(), &["commit", "-q", "-m", &format!("change {i}")]);
        }
        std::fs::write(t.path().join("f0"), "changed").unwrap();
        std::fs::write(t.path().join("new"), "n").unwrap();
        let facts = git_facts(t.path());
        assert!(facts.contains("branch: trunk"), "{facts}");
        assert_eq!(facts.matches(" change ").count(), COMMITS, "{facts}");
        assert!(facts.contains(" change 6\n"), "{facts}");
        assert!(!facts.contains(" change 1\n"), "{facts}");
        assert!(
            facts.contains("working tree: 0 staged, 1 changed, 1 untracked"),
            "{facts}"
        );
    }

    #[test]
    fn an_answer_is_kept_as_a_memory_and_the_purpose_in_inorbit_toml() {
        let t = tempfile::tempdir().unwrap();
        std::fs::write(
            t.path().join("inorbit.toml"),
            "name = \"demo\"\ntemplate = \"blank\"\n",
        )
        .unwrap();
        assert!(project_notes(t.path()).starts_with("nothing is written down"));
        let q = Question {
            about: About::Purpose,
            ask: "What is this project for?".to_owned(),
            choices: vec!["a".to_owned(), "b".to_owned()],
        };
        let said = save(t.path(), &[(q, "A CLI that audits Rust crates".to_owned())]);
        assert!(
            said.iter()
                .any(|s| s.starts_with("saved to .inorbit/memory/")),
            "{said:?}"
        );
        assert!(said.iter().any(|s| s.contains("inorbit.toml")), "{said:?}");
        let toml = std::fs::read_to_string(t.path().join("inorbit.toml")).unwrap();
        assert!(
            toml.starts_with("name = \"demo\"\ntemplate = \"blank\"\n"),
            "{toml}"
        );
        assert!(
            toml.contains("[project]\ndescription = \"A CLI that audits Rust crates\""),
            "{toml}"
        );
        assert_eq!(
            description(t.path()).as_deref(),
            Some("A CLI that audits Rust crates")
        );
        let notes = project_notes(t.path());
        assert!(
            notes.contains("purpose (inorbit.toml): A CLI that audits Rust crates"),
            "{notes}"
        );
        assert!(
            notes.contains("This project is for: A CLI that audits Rust crates"),
            "{notes}"
        );
    }

    #[test]
    fn a_secret_is_refused_and_nothing_is_written() {
        let t = tempfile::tempdir().unwrap();
        std::fs::write(t.path().join("inorbit.toml"), "name = \"demo\"\n").unwrap();
        let q = Question {
            about: About::Purpose,
            ask: "What?".to_owned(),
            choices: vec!["a".to_owned(), "b".to_owned()],
        };
        let said = save(
            t.path(),
            &[(
                q,
                "key sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789".to_owned(),
            )],
        );
        assert!(said[0].starts_with("not saved:"), "{said:?}");
        assert_eq!(
            std::fs::read_to_string(t.path().join("inorbit.toml")).unwrap(),
            "name = \"demo\"\n"
        );
        assert_eq!(Store::project(t.path()).load().unwrap().len(), 0);
    }

    #[test]
    fn a_project_without_inorbit_toml_keeps_the_memory_only() {
        let t = tempfile::tempdir().unwrap();
        let q = Question {
            about: About::Note,
            ask: "Who deploys it?".to_owned(),
            choices: vec!["me".to_owned(), "CI".to_owned()],
        };
        let said = save(t.path(), &[(q, "CI".to_owned())]);
        assert_eq!(said.len(), 1, "{said:?}");
        assert!(!t.path().join("inorbit.toml").exists());
        assert!(project_notes(t.path()).contains("Who deploys it? CI"));
    }

    #[test]
    fn the_plain_lines_name_the_choices() {
        let (_, qs) = split(BLOCK);
        let lines = plain_lines(&qs);
        assert!(lines[0].contains("[1] A CLI tool  [2] A web service  [3] A library  [other]"));
        assert!(lines[1].contains("iohr memory remember"));
        assert_eq!(plain_lines(&[]), Vec::<String>::new());
    }
}
