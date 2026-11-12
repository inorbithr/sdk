//! The command line: text and JSON output, exit codes that mean something, and a version that
//! says which commit it was built from. Standard library only.

use std::io::Write;

pub const VERSION: &str = "0.1.0";
/// The commit this was built from (`build.rs`).
pub const COMMIT: &str = env!("BUILD_COMMIT");

pub const OK: u8 = 0;
pub const FAILURE: u8 = 1;
pub const USAGE: u8 = 2;

pub const HELP: &str = "Usage: @@NAME@@ [--json] <command>

Commands:
  greet [NAME]   Say hello to NAME (default: world)
  version        Print the version and the commit it was built from

Options:
      --json     Machine-readable output
  -h, --help     Show this help

Exit codes: 0 success, 1 failure, 2 usage error.
";

/// A JSON string: quoted, with the characters JSON requires escaped.
fn json_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// One flat JSON object with its keys in the order given (alphabetical, so output is stable).
fn json_object(fields: &[(&str, &str)]) -> String {
    let body: Vec<String> = fields
        .iter()
        .map(|(k, v)| format!("{}:{}", json_string(k), json_string(v)))
        .collect();
    format!("{{{}}}", body.join(","))
}

/// An argument in an error line is cut and escaped: a hostile one cannot shape the output.
fn quote(s: &str) -> String {
    let cut: String = if s.chars().count() > 40 {
        s.chars().take(40).chain("...".chars()).collect()
    } else {
        s.to_owned()
    };
    format!("{cut:?}")
}

fn usage_error(err: &mut dyn Write, msg: &str) -> u8 {
    let _ = writeln!(err, "@@NAME@@: {msg}\nrun `@@NAME@@ --help` for the commands");
    USAGE
}

/// The whole program, with its streams as arguments so a test can read them. `commit` is
/// what `version` prints.
pub fn run(args: &[String], out: &mut dyn Write, err: &mut dyn Write, commit: &str) -> u8 {
    let mut as_json = false;
    let mut rest: Vec<&str> = Vec::new();
    for a in args {
        match a.as_str() {
            "--json" => as_json = true,
            "-h" | "--help" => {
                let _ = out.write_all(HELP.as_bytes());
                return OK;
            }
            other => rest.push(other),
        }
    }
    let Some((cmd, tail)) = rest.split_first() else {
        let _ = err.write_all(HELP.as_bytes());
        return USAGE;
    };
    match *cmd {
        "greet" => {
            if tail.len() > 1 {
                return usage_error(err, "greet takes at most one name");
            }
            let name = tail.first().map_or("world", |n| n.trim());
            if name.is_empty() || name.chars().count() > 64 {
                let _ = writeln!(err, "@@NAME@@: the name is 1 to 64 characters of text");
                return FAILURE;
            }
            let msg = format!("Hello, {name}");
            let _ = if as_json {
                writeln!(out, "{}", json_object(&[("message", &msg)]))
            } else {
                writeln!(out, "{msg}")
            };
            OK
        }
        "version" | "--version" => {
            if !tail.is_empty() {
                return usage_error(err, "version takes no arguments");
            }
            let _ = if as_json {
                writeln!(
                    out,
                    "{}",
                    json_object(&[("commit", commit), ("name", "@@NAME@@"), ("version", VERSION)])
                )
            } else {
                writeln!(out, "@@NAME@@ {VERSION} (commit {commit})")
            };
            OK
        }
        other => usage_error(err, &format!("unknown command {}", quote(other))),
    }
}
