//! Usage errors that help: what was missing or wrong in plain words, the usage line, a few
//! real examples of the exact command that failed (the table `--help` ends with, in
//! [`crate::examples`]), where the help is, and one line that offers AI help.
//!
//! Two things reach here: clap's own errors (a missing argument, an unknown flag, a value it
//! refuses) and the usage errors commands raise themselves ([`crate::Error::Usage`]). Both
//! exit 2, as before. Under `--json` the report is one JSON line in the platform's error
//! envelope (`code`, `error`, `details`); with no terminal, `NO_COLOR` or `TERM=dumb` it is
//! the same text without styling. Everything goes to stderr.
//!
//! The AI line is an offer and nothing more: it prints an `iohr ask` command with the failed
//! command and what was wrong filled in. Nothing is run and nothing is sent when an error is
//! printed; the model is asked only if the person runs that line.
//!
//! No value typed on the line is ever repeated: not in the AI line, not in the JSON, not in
//! the text on the terminal. A clap error is rebuilt from its kind, the command path and the
//! NAME of the argument or flag (clap's `InvalidArg`, which is the argument's own name, never
//! the value), so the words say "invalid value for --days" and not which value. A usage error
//! a command raises itself is free text that may repeat a value; it is shown on the terminal
//! as raised, and the AI line and the JSON carry a fixed value-free line in its place (the
//! one message known to be free of values, [`NOT_A_COMMAND`], is passed on as it is).

use std::ffi::OsString;
use std::io::Write as _;
use std::process::ExitCode;

use clap::error::{ContextKind, ContextValue, ErrorKind};
use serde_json::json;

use crate::examples::{self, Example, Style, quote};
use crate::ui::Paint;

/// The code of the envelope a usage error is printed as under `--json`.
const CODE: &str = "usage";

/// The message for a word that is no command and no installed extension. It names no word:
/// what was typed is not repeated, so it is the one runtime usage message passed on as raised.
pub(crate) const NOT_A_COMMAND: &str = "that is not an iohr command or an installed extension\n\
     `iohr --help` lists the commands; `iohr ext install PUBLISHER/NAME` installs an extension";

/// What replaces a runtime usage message in the AI line and the JSON: those messages are free
/// text, and some repeat a value that was typed.
const WITHHELD: &str = "the arguments were refused (a usage error)";
const WITHHELD_NOTE: &str = "The reason is left out of this line because it can repeat a value you typed; run the command without --json to read it.";

/// How much of the error goes into the AI line: a question, not a log.
const ASK_ERROR_CHARS: usize = 300;

/// What a usage error says, before it is drawn as text or JSON.
#[derive(Debug)]
struct Report {
    /// What was wrong, in one line. Free of typed values: it goes into the AI line and the JSON.
    headline: String,
    /// More lines about it: the missing arguments and what each is, or tips. Free of typed
    /// values as well.
    notes: Vec<String>,
    /// For a usage error a command raised: the message as raised, which the terminal shows in
    /// place of the headline and notes. It may repeat a typed value, so it goes nowhere else.
    shown: Option<(String, Vec<String>)>,
    /// The command that failed, `iohr api`.
    command: String,
    /// Its usage, `iohr api [OPTIONS] <METHOD> <PATH>`.
    usage: String,
    /// Real lines for it.
    examples: &'static [Example],
    /// The line that gets AI help from this error, to be run by the person.
    ask: String,
}

/// Prints clap's error as a usage report and returns the exit code (2). Help and version
/// are not errors: clap prints them itself, as before.
#[must_use]
pub fn report_clap(args: &[OsString], json: bool, e: &clap::Error) -> ExitCode {
    // Help and version are what was asked for. A bare `iohr token` is not: it is a command
    // that needs more, so it gets the report (the subcommands listed, the examples, exit 2).
    if matches!(e.kind(), ErrorKind::DisplayHelp | ErrorKind::DisplayVersion) {
        e.exit()
    }
    let root = tree(json);
    let (_, cmd) = resolve(&root, args);
    let (headline, notes) = clap_words(e, cmd);
    emit(&build(&root, args, headline, notes), json)
}

/// Prints a usage error a command raised (`Error::Usage`) the same way, and returns 2.
#[must_use]
pub fn report_message(args: &[OsString], json: bool, message: &str) -> ExitCode {
    let root = tree(json);
    let mut report = if message == NOT_A_COMMAND {
        let (headline, notes) = split_lines(message);
        build(&root, args, headline, notes)
    } else {
        let mut r = build(
            &root,
            args,
            WITHHELD.to_owned(),
            vec![WITHHELD_NOTE.to_owned()],
        );
        r.shown = Some(split_lines(message));
        r
    };
    if report.shown.as_ref().is_some_and(|(h, _)| h.is_empty()) {
        report.shown = None;
    }
    emit(&report, json)
}

fn emit(report: &Report, json: bool) -> ExitCode {
    let text = if json {
        envelope(report)
    } else {
        text(report, Paint::stderr(json))
    };
    let _ = writeln!(std::io::stderr().lock(), "{text}");
    ExitCode::from(2)
}

/// The command tree, built so every subcommand knows its full name and the global flags.
fn tree(json: bool) -> clap::Command {
    let mut root = crate::cli::command(json);
    root.build();
    root
}

/// The command the arguments reach: its path (`["token", "create"]`) and the command itself.
/// Flags are skipped, and the value of one that takes a value with them, so
/// `iohr --profile work api` is `api`.
fn resolve<'a>(root: &'a clap::Command, args: &[OsString]) -> (Vec<String>, &'a clap::Command) {
    let mut cur = root;
    let mut path = Vec::new();
    let mut words = args.iter().skip(1).filter_map(|a| a.to_str());
    while let Some(arg) = words.next() {
        if arg == "--" {
            break;
        }
        if let Some(long) = arg.strip_prefix("--") {
            if !long.contains('=') && takes_value(cur, |a| a.get_long() == Some(long)) {
                words.next();
            }
        } else if let Some(short) = arg.strip_prefix('-') {
            let mut chars = short.chars();
            if let (Some(c), None) = (chars.next(), chars.next())
                && takes_value(cur, |a| a.get_short() == Some(c))
            {
                words.next();
            }
        } else if let Some(sub) = cur.find_subcommand(arg) {
            path.push(sub.get_name().to_owned());
            cur = sub;
        }
    }
    (path, cur)
}

fn takes_value(cmd: &clap::Command, which: impl Fn(&clap::Arg) -> bool) -> bool {
    cmd.get_arguments()
        .any(|a| which(a) && a.get_action().takes_values())
}

fn build(root: &clap::Command, args: &[OsString], headline: String, notes: Vec<String>) -> Report {
    let (path, cmd) = resolve(root, args);
    let command: String = std::iter::once("iohr")
        .chain(path.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join(" ");
    let usage = cmd.clone().render_usage().to_string();
    let usage = usage.trim().strip_prefix("Usage:").unwrap_or(&usage).trim();
    let (_, examples) = examples::for_path(&path);
    let said = headline.replace('\'', "\"");
    Report {
        shown: None,
        ask: ask_line(&command, &said),
        headline,
        notes,
        command,
        usage: usage.to_owned(),
        examples,
    }
}

/// The offer: `iohr ask '<question>'`, the command that failed and what was wrong filled in.
/// The command is its name only (`iohr api`), never the arguments, and `said` is the headline,
/// which holds no typed value: a value on the line may be a secret, and this line is meant to
/// be pasted, shared and kept in shell history.
fn ask_line(command: &str, said: &str) -> String {
    let said: String = said
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(ASK_ERROR_CHARS)
        .collect();
    let question =
        format!("I ran `{command}` and iohr said: {said} What command should I run instead?");
    format!("iohr ask {}", quote(&question))
}

/// The first line, and the rest of the lines trimmed and without the empty ones.
fn split_lines(message: &str) -> (String, Vec<String>) {
    let mut lines = message.lines();
    let headline = lines.next().unwrap_or_default().trim().to_owned();
    let notes = lines
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect();
    (headline, notes)
}

/// What clap said, rebuilt in our words from the error's kind, the command and the NAME of the
/// argument. Clap's own text is never used: it quotes the value that was typed (an unexpected
/// argument, an invalid value, an unknown subcommand). `InvalidArg` is the argument's name for
/// every kind handled here except an unknown argument, where it is the typed word, so that one
/// is only told apart (a flag or not) and never repeated.
fn clap_words(e: &clap::Error, cmd: &clap::Command) -> (String, Vec<String>) {
    let name = cmd.get_bin_name().unwrap_or("iohr").to_owned();
    let arg = strings(e.get(ContextKind::InvalidArg)).and_then(|v| v.into_iter().next());
    let tips = |kind, lead: &str| -> Vec<String> {
        strings(e.get(kind))
            .filter(|v| !v.is_empty())
            .map(|v| format!("{lead} {}", join_and(&v)))
            .into_iter()
            .collect()
    };
    match e.kind() {
        ErrorKind::MissingRequiredArgument => {
            let names = strings(e.get(ContextKind::InvalidArg)).unwrap_or_default();
            if names.is_empty() {
                return (format!("`{name}` is missing an argument"), Vec::new());
            }
            (
                format!("`{name}` is missing {}", join_and(&names)),
                described(cmd, &names),
            )
        }
        ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand | ErrorKind::MissingSubcommand => {
            needs_words(cmd, &name)
        }
        ErrorKind::UnknownArgument => {
            let flag = arg.as_deref().is_some_and(|a| a.starts_with('-'));
            let headline = if flag {
                format!("`{name}` got a flag it does not have")
            } else {
                format!("`{name}` got an unexpected argument")
            };
            (headline, tips(ContextKind::SuggestedArg, "did you mean"))
        }
        ErrorKind::InvalidSubcommand => (
            format!("`{name}` has no such subcommand"),
            tips(ContextKind::SuggestedSubcommand, "did you mean"),
        ),
        ErrorKind::InvalidValue | ErrorKind::ValueValidation => (
            format!(
                "invalid value for {}",
                arg.unwrap_or_else(|| "an argument".to_owned())
            ),
            tips(ContextKind::ValidValue, "possible values:"),
        ),
        ErrorKind::ArgumentConflict => {
            let prior = strings(e.get(ContextKind::PriorArg)).unwrap_or_default();
            let a = arg.unwrap_or_else(|| "an argument".to_owned());
            let headline = if prior.is_empty() {
                format!("{a} cannot be used here")
            } else {
                format!("{a} cannot be used with {}", join_and(&prior))
            };
            (headline, Vec::new())
        }
        kind => {
            let what = match kind {
                ErrorKind::TooManyValues => "too many values for",
                ErrorKind::TooFewValues => "too few values for",
                ErrorKind::WrongNumberOfValues => "the wrong number of values for",
                ErrorKind::NoEquals => "needs `=` in",
                _ => "did not accept its arguments:",
            };
            let which = match kind {
                ErrorKind::TooManyValues
                | ErrorKind::TooFewValues
                | ErrorKind::WrongNumberOfValues
                | ErrorKind::NoEquals => arg.unwrap_or_else(|| "an argument".to_owned()),
                _ => "see the usage below".to_owned(),
            };
            (format!("`{name}` {what} {which}"), Vec::new())
        }
    }
}

/// What a command needs when it was run with nothing: its subcommands, each with its first help
/// line (what clap's help listed), or else the arguments it requires.
fn needs_words(cmd: &clap::Command, name: &str) -> (String, Vec<String>) {
    let mut visible: Vec<&clap::Command> = cmd
        .get_subcommands()
        .filter(|s| !s.is_hide_set() && s.get_name() != "help")
        .collect();
    visible.sort_by_key(|s| s.get_display_order());
    let subs: Vec<(String, String)> = visible
        .into_iter()
        .map(|s| {
            let about = s.get_about().map(ToString::to_string).unwrap_or_default();
            (
                s.get_name().to_owned(),
                about.lines().next().unwrap_or_default().to_owned(),
            )
        })
        .collect();
    if !subs.is_empty() {
        let width = subs.iter().map(|(n, _)| n.len()).max().unwrap_or(0);
        let notes = subs
            .iter()
            .map(|(n, a)| format!("{n:width$}  {a}").trim_end().to_owned())
            .collect();
        return (format!("`{name}` needs a subcommand"), notes);
    }
    let names: Vec<String> = cmd
        .get_arguments()
        .filter(|a| a.is_required_set())
        .map(ToString::to_string)
        .collect();
    let notes = described(cmd, &names);
    (format!("`{name}` needs arguments"), notes)
}

/// One note per argument name: the name, then the first line of its help when it has one.
fn described(cmd: &clap::Command, names: &[String]) -> Vec<String> {
    let width = names.iter().map(String::len).max().unwrap_or(0);
    names
        .iter()
        .map(|n| match describe(cmd, n) {
            Some(help) => format!("{n:width$}  {help}"),
            None => n.clone(),
        })
        .collect()
}

fn strings(value: Option<&ContextValue>) -> Option<Vec<String>> {
    match value? {
        ContextValue::Strings(v) => Some(v.clone()),
        ContextValue::String(s) => Some(vec![s.clone()]),
        _ => None,
    }
}

/// `a`, `a and b`, `a, b and c`.
fn join_and(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    }
}

/// What the command's own help says about the argument clap named (`<PATH>`,
/// `--name <NAME>`): its first line.
fn describe(cmd: &clap::Command, name: &str) -> Option<String> {
    let arg = if let Some(long) = name.strip_prefix("--") {
        let long = long.split([' ', '=']).next()?;
        cmd.get_arguments().find(|a| a.get_long() == Some(long))?
    } else {
        let bare = name
            .trim_matches(['<', '>', '[', ']'])
            .trim_end_matches("...");
        cmd.get_arguments().find(|a| {
            a.is_positional()
                && (a.get_id().as_str().eq_ignore_ascii_case(bare)
                    || a.get_value_names()
                        .is_some_and(|v| v.iter().any(|n| n.as_str().eq_ignore_ascii_case(bare))))
        })?
    };
    let help = arg.get_help()?.to_string();
    help.lines()
        .next()
        .map(str::to_owned)
        .filter(|l| !l.is_empty())
}

/// The report for a person: a plain line, the notes, the usage, the examples, where the
/// help is, and the AI offer. `paint` is plain with no terminal, `NO_COLOR` or `--json`.
fn text(r: &Report, paint: Paint) -> String {
    use std::fmt::Write as _;
    let (headline, notes) = match &r.shown {
        Some((h, n)) => (h, n),
        None => (&r.headline, &r.notes),
    };
    let mut out = format!("{} {headline}\n", paint.bad("error:"));
    for note in notes {
        let _ = writeln!(out, "  {note}");
    }
    let _ = write!(out, "\n{} {}\n", paint.head("Usage:"), r.usage);
    if !r.examples.is_empty() {
        let block = examples::block(
            r.examples,
            &Style {
                head: &|t| paint.head(t),
                command: &|t| paint.name(t),
                note: &|t| paint.dim(t),
            },
        );
        let _ = write!(out, "\n{block}\n");
    }
    let _ = write!(
        out,
        "\nMore: {}\n\n{}\n  {}",
        paint.name(&format!("{} --help", r.command)),
        paint.dim("Stuck? Ask AI (your own agent). Nothing is sent unless you run this line:"),
        paint.name(&r.ask),
    );
    out
}

/// The report for a program: one JSON line in the platform's envelope, `code`, `error` and
/// typed `details` (a `usage` detail with everything the text has).
fn envelope(r: &Report) -> String {
    let examples: Vec<_> = r
        .examples
        .iter()
        .map(|e| json!({ "command": e.line(), "what": e.what }))
        .collect();
    json!({
        "code": CODE,
        "error": r.headline,
        "details": [{
            "type": "usage",
            "command": r.command,
            "usage": r.usage,
            "notes": r.notes,
            "examples": examples,
            "help": format!("{} --help", r.command),
            "ask_ai": r.ask,
        }],
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(words: &[&str]) -> Vec<OsString> {
        std::iter::once("iohr")
            .chain(words.iter().copied())
            .map(OsString::from)
            .collect()
    }

    fn report_for(words: &[&str]) -> Report {
        let a = args(words);
        let root = tree(false);
        let e = crate::cli::command(false)
            .try_get_matches_from(&a)
            .expect_err("the arguments are wrong");
        let (_, cmd) = resolve(&root, &a);
        let (headline, notes) = clap_words(&e, cmd);
        build(&root, &a, headline, notes)
    }

    #[test]
    fn the_command_is_found_past_flags_and_their_values() {
        let root = tree(false);
        let found = |words: &[&str]| resolve(&root, &args(words)).0;
        assert_eq!(found(&["api"]), ["api"]);
        assert_eq!(found(&["--profile", "work", "api", "GET"]), ["api"]);
        assert_eq!(
            found(&["--json", "token", "create", "-v"]),
            ["token", "create"]
        );
        assert_eq!(found(&["api", "--", "token"]), ["api"]);
        assert_eq!(found(&["--bogus"]).len(), 0);
    }

    #[test]
    fn a_missing_argument_is_named_and_explained() {
        let r = report_for(&["api", "--json"]);
        assert_eq!(r.headline, "`iohr api` is missing <METHOD> and <PATH>");
        assert!(r.notes[0].starts_with("<METHOD>"), "{:?}", r.notes);
        assert!(r.notes[1].contains("A path on the API"), "{:?}", r.notes);
        assert_eq!(r.usage, "iohr api [OPTIONS] <METHOD> <PATH>");
        assert!(r.examples.iter().any(|e| e.line() == "iohr api GET /v1/me"));
    }

    #[test]
    fn a_missing_flag_is_explained_too() {
        let r = report_for(&["token", "create"]);
        assert!(
            r.headline
                .starts_with("`iohr token create` is missing --name"),
            "{r:?}"
        );
        // The nested command has examples of its own.
        assert!(
            r.examples
                .iter()
                .all(|e| e.args[..2] == ["token", "create"])
        );
    }

    #[test]
    fn another_error_is_told_by_kind_and_name_without_claps_text() {
        let r = report_for(&["api", "--nope", "GET", "/v1/me"]);
        assert_eq!(r.headline, "`iohr api` got a flag it does not have");
        assert!(!r.headline.starts_with("error"), "{r:?}");
        let r = report_for(&["whoami", "VALUE"]);
        assert_eq!(r.headline, "`iohr whoami` got an unexpected argument");
        let r = report_for(&[
            "token", "create", "--name", "x", "--scope", "s", "--days", "VALUE",
        ]);
        assert_eq!(r.headline, "invalid value for --days <DAYS>");
        let r = report_for(&["token", "creat"]);
        assert_eq!(r.headline, "`iohr token` has no such subcommand");
        assert_eq!(r.notes, ["did you mean create"]);
        for r in [r, report_for(&["api", "--nope", "VALUE"])] {
            assert!(!format!("{r:?}").contains("VALUE"), "{r:?}");
        }
    }

    #[test]
    fn a_bare_command_lists_what_it_needs() {
        let r = report_for(&["token"]);
        assert_eq!(r.headline, "`iohr token` needs a subcommand");
        assert!(
            r.notes[0].starts_with("create  Create an API token"),
            "{:?}",
            r.notes
        );
        assert_eq!(r.notes.len(), 3, "{:?}", r.notes);
    }

    #[test]
    fn the_ai_offer_is_one_quoted_command_that_names_the_failure() {
        let r = report_for(&["api", "--json"]);
        assert!(r.ask.starts_with("iohr ask '"), "{}", r.ask);
        assert!(r.ask.contains("I ran `iohr api`"), "{}", r.ask);
        assert!(
            r.ask.contains("is missing <METHOD> and <PATH>"),
            "{}",
            r.ask
        );
        // One line, and a quote in what iohr said cannot end the quoting early.
        let tricky = ask_line("iohr memory", "it's\nbad");
        assert!(!tricky.contains('\n'), "{tricky}");
    }

    #[test]
    fn a_runtime_message_is_shown_but_never_reaches_the_ask_or_the_envelope() {
        let a = args(&["profile", "use", "SECRETMARKER"]);
        let root = tree(false);
        let message = "there is no profile SECRETMARKER";
        let mut r = build(
            &root,
            &a,
            WITHHELD.to_owned(),
            vec![WITHHELD_NOTE.to_owned()],
        );
        r.shown = Some(split_lines(message));
        assert!(r.ask.contains("I ran `iohr profile use`"), "{}", r.ask);
        assert!(!r.ask.contains("SECRETMARKER"), "{}", r.ask);
        assert!(!envelope(&r).contains("SECRETMARKER"), "{}", envelope(&r));
        // The person at the terminal sees the reason as raised.
        let t = text(&r, Paint::plain());
        assert!(t.contains("error: there is no profile SECRETMARKER"), "{t}");
        assert!(!t.contains(WITHHELD), "{t}");
    }

    #[test]
    fn the_one_message_without_values_is_passed_on_as_it_is() {
        let (h, n) = split_lines(NOT_A_COMMAND);
        let r = build(&tree(false), &args(&["SECRETMARKER"]), h, n);
        assert_eq!(
            r.headline,
            "that is not an iohr command or an installed extension"
        );
        assert!(!format!("{r:?}{}", envelope(&r)).contains("SECRETMARKER"));
    }

    #[test]
    fn the_ai_offer_is_bounded() {
        let long = "x".repeat(5000);
        let line = ask_line("iohr api", &long);
        assert!(line.len() < ASK_ERROR_CHARS + 200, "{}", line.len());
    }

    #[test]
    fn plain_text_has_every_part_and_no_styling() {
        let r = report_for(&["api"]);
        let t = text(&r, Paint::plain());
        for part in [
            "error: `iohr api` is missing <METHOD> and <PATH>",
            "Usage: iohr api [OPTIONS] <METHOD> <PATH>",
            "Examples:",
            "iohr api GET /v1/me",
            "More: iohr api --help",
            "Ask AI (your own agent)",
            "iohr ask '",
        ] {
            assert!(t.contains(part), "{part:?} missing from:\n{t}");
        }
        assert!(!t.contains('\u{1b}'), "{t:?}");
    }

    #[test]
    fn the_envelope_is_one_json_line_with_the_platforms_fields() {
        let r = report_for(&["api"]);
        let line = envelope(&r);
        assert!(!line.contains('\n'));
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["code"], "usage");
        assert_eq!(v["error"], "`iohr api` is missing <METHOD> and <PATH>");
        let d = &v["details"][0];
        assert_eq!(d["type"], "usage");
        assert_eq!(d["help"], "iohr api --help");
        assert_eq!(d["examples"][0]["command"], "iohr api GET /v1/me");
        assert!(d["ask_ai"].as_str().unwrap().starts_with("iohr ask "));
    }

    #[test]
    fn a_runtime_usage_message_is_reported_with_its_command() {
        let a = args(&["permissions", "allow", "Bash(*)"]);
        let root = tree(false);
        let (headline, notes) = split_lines("that rule is too broad\nuse a narrower one");
        let r = build(&root, &a, headline, notes);
        assert_eq!(r.command, "iohr permissions allow");
        assert_eq!(r.notes, ["use a narrower one"]);
        assert!(
            r.examples
                .iter()
                .any(|e| e.line().contains("permissions allow"))
        );
    }
}
