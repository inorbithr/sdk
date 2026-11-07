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
//! command and the error text filled in. Nothing is run and nothing is sent when an error is
//! printed; the model is asked only if the person runs that line.

use std::ffi::OsString;
use std::io::Write as _;
use std::process::ExitCode;

use clap::error::{ContextKind, ContextValue, ErrorKind};
use serde_json::json;

use crate::examples::{self, Example, Style, quote};
use crate::ui::Paint;

/// The code of the envelope a usage error is printed as under `--json`.
const CODE: &str = "usage";

/// How much of the error goes into the AI line: a question, not a log.
const ASK_ERROR_CHARS: usize = 300;

/// What a usage error says, before it is drawn as text or JSON.
#[derive(Debug)]
struct Report {
    /// What was wrong, in one line.
    headline: String,
    /// More lines about it: the missing arguments and what each is, or clap's tips.
    notes: Vec<String>,
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
    if matches!(
        e.kind(),
        ErrorKind::DisplayHelp
            | ErrorKind::DisplayVersion
            | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
    ) {
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
    let (headline, notes) = split_lines(message);
    emit(&build(&root, args, headline, notes), json)
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
        ask: ask_line(&command, &said),
        headline,
        notes,
        command,
        usage: usage.to_owned(),
        examples,
    }
}

/// The offer: `iohr ask '<question>'`, the command that failed and what iohr said filled in.
/// The command is its name only (`iohr api`), never the arguments: a value typed on the line
/// may be a secret, and the line is meant to be pasted, shared and kept in shell history.
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

/// What clap said, in our words where we can be plainer (a missing argument), else its own
/// text without the "error:" label, the usage and the "for more information" line.
fn clap_words(e: &clap::Error, cmd: &clap::Command) -> (String, Vec<String>) {
    if e.kind() == ErrorKind::MissingRequiredArgument
        && let Some(names) = strings(e.get(ContextKind::InvalidArg))
        && !names.is_empty()
    {
        let width = names.iter().map(String::len).max().unwrap_or(0);
        let notes = names
            .iter()
            .map(|n| match describe(cmd, n) {
                Some(help) => format!("{n:width$}  {help}"),
                None => n.clone(),
            })
            .collect();
        let name = cmd.get_bin_name().unwrap_or("iohr");
        return (format!("`{name}` is missing {}", join_and(&names)), notes);
    }
    let full = e.to_string();
    let body = full.split_once("Usage:").map_or(full.as_str(), |(b, _)| b);
    let (headline, notes) = split_lines(body);
    let headline = headline
        .strip_prefix("error:")
        .map_or(headline.as_str(), str::trim)
        .to_owned();
    (headline, notes)
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
    let mut out = format!("{} {}\n", paint.bad("error:"), r.headline);
    for note in &r.notes {
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
    fn another_error_keeps_claps_words_without_its_labels() {
        let r = report_for(&["api", "--nope", "GET", "/v1/me"]);
        assert!(r.headline.contains("--nope"), "{r:?}");
        assert!(!r.headline.starts_with("error"), "{r:?}");
        assert!(!r.headline.contains("Usage"), "{r:?}");
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
    fn nothing_typed_on_the_line_comes_back_in_the_report() {
        // `--config api_key=...` is refused at run time with a usage message; the value must
        // not reach the line meant to be pasted and kept in shell history, nor the envelope.
        let a = args(&[
            "connections",
            "add",
            "incident-io",
            "--config",
            "api_key=SECRETMARKER",
        ]);
        let root = tree(false);
        let (headline, notes) = split_lines("api_key is a secret and is never an argument");
        let r = build(&root, &a, headline, notes);
        assert!(r.ask.contains("I ran `iohr connections add`"), "{}", r.ask);
        assert!(
            !format!("{r:?}{}{}", envelope(&r), text(&r, Paint::plain())).contains("SECRETMARKER")
        );
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
