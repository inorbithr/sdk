//! The headless `iohr ask` and `iohr shell hook zsh` (PRD 0187, PRD 0088.1, RFC 0203): the
//! binary with a private HOME and a PATH holding a fake `claude`, so no real agent starts.

#![allow(
    clippy::unwrap_used,
    reason = "test helpers fail the test by panicking"
)]
#![cfg(unix)]

use std::fmt::Write as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

const SECRET: &str = "the-secret-question";

/// A fake `claude`: records its arguments (one per line), its stdin and whether a provider
/// key or token variable reached it, then prints `$FAKE_JSON`.
const FAKE_CLAUDE: &str = r#"#!/bin/sh
# The run's environment is scrubbed, so what the fake needs is in files under $HOME.
O="$HOME/out"
printf '%s\n' "$@" > "$O/claude.args"
cat > "$O/claude.stdin"
env | grep -E '^(ANTHROPIC_|IOHR_TOKEN|OPENAI_)' > "$O/claude.leaked"
cat "$O/json"
exit "$(cat "$O/exit")"
"#;

const OK_JSON: &str = r#"{"type":"result","subtype":"success","is_error":false,"result":"It is Rayleigh scattering.","duration_ms":321,"num_turns":1}"#;

struct Rig {
    home: tempfile::TempDir,
}

impl Rig {
    fn new() -> Self {
        let r = Self {
            home: tempfile::tempdir().unwrap(),
        };
        std::fs::create_dir_all(r.out()).unwrap();
        std::fs::create_dir_all(r.bin()).unwrap();
        std::fs::create_dir_all(r.work()).unwrap();
        r
    }
    fn bin(&self) -> PathBuf {
        self.home.path().join("bin")
    }
    fn out(&self) -> PathBuf {
        self.home.path().join("out")
    }
    fn work(&self) -> PathBuf {
        self.home.path().join("work")
    }
    fn fake(&self, name: &str, script: &str) {
        let p = self.bin().join(name);
        std::fs::write(&p, script).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    fn run(&self, args: &[&str], json: &str, exit: &str) -> Output {
        self.run_env(args, json, exit, &[])
    }
    fn run_env(&self, args: &[&str], json: &str, exit: &str, env: &[(&str, &str)]) -> Output {
        std::fs::write(self.out().join("json"), json).unwrap();
        std::fs::write(self.out().join("exit"), exit).unwrap();
        Command::new(env!("CARGO_BIN_EXE_iohr"))
            .args(args)
            .current_dir(self.work())
            .env_clear()
            .envs(env.iter().copied())
            .env("PATH", format!("{}:/usr/bin:/bin", self.bin().display()))
            .env("HOME", self.home.path())
            .env("IOHR_CONFIG_DIR", self.home.path().join("config"))
            // Reaches the fake agent only if the scrub is broken.
            .env("ANTHROPIC_API_KEY", "sk-test-must-not-leak")
            .env("IOHR_TOKEN", "tok-test-must-not-leak")
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }
    fn saw(&self, file: &str) -> String {
        std::fs::read_to_string(self.out().join(file)).unwrap_or_default()
    }
}

fn text(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

// ---------------------------------------------------------------- iohr ask, headless

#[test]
fn json_is_the_documented_shape_and_never_the_question() {
    let r = Rig::new();
    r.fake("claude", FAKE_CLAUDE);
    let out = r.run(&["--json", "ask", "--headless", SECRET], OK_JSON, "0");
    assert!(out.status.success(), "{}", text(&out));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let mut keys: Vec<_> = v.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    assert_eq!(
        keys,
        [
            "access",
            "agent",
            "answer",
            "command",
            "context",
            "duration_ms",
            "exit_code",
            "label",
            "model",
            "next",
            "permissions",
            "questions",
            "via"
        ]
    );
    assert_eq!(v["questions"], serde_json::json!([]));
    assert_eq!(v["access"], serde_json::Value::Null);
    assert_eq!(v["next"], serde_json::json!([]));
    assert_eq!(v["answer"], "It is Rayleigh scattering.");
    assert_eq!(v["agent"], "claude");
    assert_eq!(v["via"], "local");
    assert_eq!(v["model"], "haiku");
    assert_eq!(v["exit_code"], 0);
    assert_eq!(v["duration_ms"], 321);
    // The AI label (AI Act Art. 50) travels with the answer.
    let label = v["label"].as_str().unwrap();
    assert!(
        label.contains("Io -- Io running on Claude Code (local)"),
        "{label}"
    );
    assert!(
        label.contains("Read: your question, the folder's path"),
        "{label}"
    );
    // The question went to the agent on stdin, and nowhere else.
    assert!(r.saw("claude.stdin").contains(SECRET));
    assert!(!text(&out).contains(SECRET));
    assert!(!r.saw("claude.args").contains(SECRET), "never an argument");
}

#[test]
fn text_mode_prints_the_label_then_the_answer() {
    let r = Rig::new();
    r.fake("claude", FAKE_CLAUDE);
    // Not a terminal, no flag: the answer is printed, the agent is not opened.
    let out = r.run(
        &["ask", "--headless", "why", "is", "the", "sky", "blue"],
        OK_JSON,
        "0",
    );
    assert!(out.status.success(), "{}", text(&out));
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "It is Rayleigh scattering.\n"
    );
    let err = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_eq!(err.lines().count(), 1, "{err}");
    assert!(err.starts_with("Io -- Io running on Claude Code (local)"));
    assert!(
        r.saw("claude.stdin")
            .contains("Question: why is the sky blue")
    );
}

#[test]
fn card_mode_off_a_terminal_is_the_plain_label_and_answer_with_the_answer_on_stdout() {
    let r = Rig::new();
    r.fake("claude", FAKE_CLAUDE);
    let out = r.run(
        &["ask", "--headless", "--card", "why", "blue"],
        OK_JSON,
        "0",
    );
    assert!(out.status.success(), "{}", text(&out));
    // The answer once, on stdout; stderr carries only the label (the double-answer bug).
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "It is Rayleigh scattering.\n"
    );
    let err = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(!err.contains('\u{1b}'), "{err:?}");
    let lines: Vec<&str> = err.lines().collect();
    assert_eq!(lines.len(), 1, "{err}");
    assert!(lines[0].starts_with("Io -- Io running on Claude Code (local)"));
    // Without a terminal the rules decide, and the label says commands may run under them.
    assert!(
        lines[0].ends_with("a command runs only when you or your rules allow it."),
        "{err}"
    );
    assert_eq!(text(&out).matches("Rayleigh").count(), 1, "{}", text(&out));
    // A failure is a card too: exit 1, the reason, never the question or the agent's text.
    let login =
        format!(r#"{{"type":"result","is_error":true,"result":"Not logged in /login {SECRET}"}}"#);
    let out = r.run(&["ask", "--headless", "--card", SECRET], &login, "1");
    assert_eq!(out.status.code(), Some(1), "{}", text(&out));
    assert_eq!(out.stdout, b"");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("not signed in"), "{err}");
    assert!(!err.contains(SECRET), "{err}");
}

#[test]
fn the_run_is_bounded_scrubbed_and_never_has_a_forbidden_flag() {
    let r = Rig::new();
    r.fake("claude", FAKE_CLAUDE);
    let out = r.run(
        &["ask", "--headless", "--permissions", "deny", "q"],
        OK_JSON,
        "0",
    );
    assert!(out.status.success(), "{}", text(&out));
    let args: Vec<String> = r.saw("claude.args").lines().map(str::to_owned).collect();
    for bad in [
        "--bare",
        "--api-key",
        "--auth-token",
        "--base-url",
        "--settings",
        "--resume",
        "--continue",
        "--dangerously-skip-permissions",
        "--mcp-config",
        "--allowedTools",
    ] {
        assert!(!args.iter().any(|a| a == bad), "{bad} in {args:?}");
    }
    let at = |f: &str| args.iter().position(|a| a == f).unwrap();
    assert_eq!(args[at("--model") + 1], "haiku");
    // A project folder (not home, not the root) lets the model read its files: a few more
    // turns, read-only tools, never Bash.
    assert_eq!(args[at("--max-turns") + 1], "12");
    assert_eq!(args[at("--tools") + 1], "Read,Grep,Glob");
    assert!(args.contains(&"--no-session-persistence".to_owned()));
    assert!(args.contains(&"--strict-mcp-config".to_owned()));
    assert_eq!(
        r.saw("claude.leaked"),
        "",
        "no key or token reaches the agent"
    );
}

fn model_sent(r: &Rig, args: &[&str]) -> String {
    let out = r.run(args, OK_JSON, "0");
    assert!(out.status.success(), "{}", text(&out));
    let sent: Vec<String> = r.saw("claude.args").lines().map(str::to_owned).collect();
    let at = sent.iter().position(|a| a == "--model").unwrap();
    sent[at + 1].clone()
}

#[test]
fn the_model_is_the_flag_then_the_config_then_haiku_and_the_json_says_which() {
    let r = Rig::new();
    r.fake("claude", FAKE_CLAUDE);
    assert_eq!(model_sent(&r, &["ask", "--headless", "q"]), "haiku");
    std::fs::create_dir_all(r.home.path().join("config")).unwrap();
    std::fs::write(
        r.home.path().join("config").join("config.toml"),
        "[ask]\nmodel = \"sonnet\"\n",
    )
    .unwrap();
    assert_eq!(model_sent(&r, &["ask", "--headless", "q"]), "sonnet");
    assert_eq!(
        model_sent(&r, &["ask", "--headless", "--model", "opus", "q"]),
        "opus"
    );
    let out = r.run(
        &["--json", "ask", "--headless", "--model", "opus", "q"],
        OK_JSON,
        "0",
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["model"], "opus");
    let bad = r.run(
        &["ask", "--headless", "--model", "--help", "q"],
        OK_JSON,
        "0",
    );
    assert!(!bad.status.success());
}

#[test]
fn context_flags_reach_the_agent_and_the_output_is_cut_to_its_end() {
    let r = Rig::new();
    r.fake("claude", FAKE_CLAUDE);
    let long = (0..2000).fold(String::new(), |mut s, i| {
        writeln!(s, "line {i}").unwrap();
        s
    });
    let f = r.home.path().join("out.txt");
    std::fs::write(&f, long).unwrap();
    let out = r.run(
        &[
            "ask",
            "--cwd",
            "/work/api",
            "--last-command",
            "cargo test",
            "--exit-code",
            "101",
            "--output-file",
            f.to_str().unwrap(),
            "--prior-question",
            "earlier q",
            "--prior-answer",
            "earlier a",
            "--",
            "what failed?",
        ],
        OK_JSON,
        "0",
    );
    assert!(out.status.success(), "{}", text(&out));
    let m = r.saw("claude.stdin");
    assert!(m.contains("<cwd>/work/api</cwd>"), "{m}");
    assert!(
        m.contains("<command exit=\"101\">cargo test</command>"),
        "{m}"
    );
    assert!(
        m.contains("line 1999") && !m.contains("line 100\n"),
        "tail only"
    );
    assert!(m.contains("Earlier question: earlier q\nEarlier answer: earlier a"));
    assert!(m.ends_with("Question: what failed?\n"));
    assert!(m.len() <= 16 * 1024);
    // A missing file is a usage error that does not say what it held.
    let out = r.run(
        &["ask", "--output-file", "/nonexistent/x", "q"],
        OK_JSON,
        "0",
    );
    assert_eq!(out.status.code(), Some(2), "{}", text(&out));
}

#[test]
fn the_agent_gets_the_folders_top_level_names_and_never_their_contents() {
    let r = Rig::new();
    r.fake("claude", FAKE_CLAUDE);
    let dir = r.home.path().join("proj");
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("Cargo.toml"), "CONTENTS-NEVER-SENT").unwrap();
    std::fs::write(dir.join("debug.log"), "x").unwrap();
    std::fs::write(dir.join(".env"), "x").unwrap();
    std::fs::write(dir.join(".gitignore"), "*.log\n").unwrap();
    let out = r.run(
        &["ask", "--cwd", dir.to_str().unwrap(), "--", "what is here?"],
        OK_JSON,
        "0",
    );
    assert!(out.status.success(), "{}", text(&out));
    let m = r.saw("claude.stdin");
    assert!(
        m.contains("<files>\nsrc/\n.gitignore\nCargo.toml\n</files>"),
        "{m}"
    );
    assert!(!m.contains("CONTENTS-NEVER-SENT") && !m.contains("debug.log") && !m.contains(".env"));
    assert!(
        text(&out).contains("its top-level file names"),
        "{}",
        text(&out)
    );
}

/// A project with an `IO.md` at the root, one in a folder below it and the user's own in the
/// config folder; the walk goes by `inorbit.toml`.
fn io_md_project(r: &Rig) -> PathBuf {
    let root = r.home.path().join("proj");
    let deep = root.join("web/app");
    std::fs::create_dir_all(&deep).unwrap();
    std::fs::write(root.join("inorbit.toml"), "[project]\nname = \"proj\"\n").unwrap();
    std::fs::write(root.join("IO.md"), "ROOT-RULES: build with make.\n").unwrap();
    std::fs::write(deep.join("io.md"), "DEEP-RULES: use pnpm.\n").unwrap();
    std::fs::create_dir_all(r.home.path().join("config")).unwrap();
    std::fs::write(
        r.home.path().join("config/IO.md"),
        "USER-RULES: be brief.\n",
    )
    .unwrap();
    deep
}

#[test]
fn a_question_loads_the_io_md_chain_root_first_and_the_card_says_so() {
    let r = Rig::new();
    r.fake("claude", FAKE_CLAUDE);
    let deep = io_md_project(&r);
    let out = r.run(
        &[
            "ask",
            "--cwd",
            deep.to_str().unwrap(),
            "--",
            "how do I build?",
        ],
        OK_JSON,
        "0",
    );
    assert!(out.status.success(), "{}", text(&out));
    let m = r.saw("claude.stdin");
    let (u, root, deeper) = (
        m.find("USER-RULES").expect("user file sent"),
        m.find("ROOT-RULES").expect("root file sent"),
        m.find("DEEP-RULES").expect("nested file sent"),
    );
    assert!(
        u < root && root < deeper,
        "farthest first, nearest last: {m}"
    );
    assert!(
        m.contains("<io-md>") && m.contains("--- web/app/io.md"),
        "{m}"
    );
    assert!(text(&out).contains("context: IO.md x3"), "{}", text(&out));
    // Outside the project nothing is loaded and the card does not mention it.
    let plain = r.home.path().join("plain");
    std::fs::create_dir_all(&plain).unwrap();
    let out = r.run(
        &["ask", "--cwd", plain.to_str().unwrap(), "--", "hi"],
        OK_JSON,
        "0",
    );
    let m = r.saw("claude.stdin");
    assert!(m.contains("USER-RULES") && !m.contains("ROOT-RULES"), "{m}");
    assert!(text(&out).contains("context: IO.md x1"), "{}", text(&out));
}

#[test]
fn the_json_context_counts_the_io_md_files_and_never_holds_their_text() {
    let r = Rig::new();
    r.fake("claude", FAKE_CLAUDE);
    let deep = io_md_project(&r);
    let out = r.run(
        &[
            "--json",
            "ask",
            "--headless",
            "--cwd",
            deep.to_str().unwrap(),
            "build?",
        ],
        OK_JSON,
        "0",
    );
    assert!(out.status.success(), "{}", text(&out));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["context"]["io_md"]["files"], 3);
    assert_eq!(v["context"]["io_md"]["cut"], 0);
    assert!(!String::from_utf8_lossy(&out.stdout).contains("ROOT-RULES"));
}

#[test]
fn iohr_context_prints_the_chain_a_question_would_load() {
    let r = Rig::new();
    let deep = io_md_project(&r);
    let out = r.run(&["context", deep.to_str().unwrap()], "", "0");
    assert!(out.status.success(), "{}", text(&out));
    let t = text(&out);
    let (u, root, deeper) = (
        t.find("user IO.md").unwrap(),
        t.find("\nIO.md").unwrap(),
        t.find("web/app/io.md").unwrap(),
    );
    assert!(u < root && root < deeper, "{t}");
    assert!(
        t.contains("project root") && t.contains("Farthest first"),
        "{t}"
    );
    let out = r.run(
        &["--json", "context", "--full", deep.to_str().unwrap()],
        "",
        "0",
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["files"].as_array().unwrap().len(), 3);
    assert_eq!(v["files"][2]["kind"], "project");
    assert!(v["text"].as_str().unwrap().contains("DEEP-RULES"));
    // No file anywhere: it says so and points at init.
    let none = r.home.path().join("empty");
    std::fs::create_dir_all(&none).unwrap();
    std::fs::remove_file(r.home.path().join("config/IO.md")).unwrap();
    let out = r.run(&["context", none.to_str().unwrap()], "", "0");
    assert!(
        text(&out).contains("No IO.md or io.md found"),
        "{}",
        text(&out)
    );
}

#[test]
fn a_long_io_md_is_cut_and_the_card_and_context_say_so() {
    let r = Rig::new();
    r.fake("claude", FAKE_CLAUDE);
    let deep = io_md_project(&r);
    std::fs::write(
        r.home.path().join("proj/IO.md"),
        "a rule that goes on and on\n".repeat(400),
    )
    .unwrap();
    let out = r.run(
        &[
            "ask",
            "--cwd",
            deep.to_str().unwrap(),
            "--",
            "how do I build?",
        ],
        OK_JSON,
        "0",
    );
    let m = r.saw("claude.stdin");
    assert!(m.contains("[IO.md is cut here:"), "{m}");
    assert!(text(&out).contains("1 cut to 4 KiB each"), "{}", text(&out));
    let out = r.run(&["context", deep.to_str().unwrap()], "", "0");
    assert!(text(&out).contains("cut"), "{}", text(&out));
}

#[test]
fn a_failed_run_exits_1_and_never_quotes_the_question_or_the_agents_text() {
    let r = Rig::new();
    r.fake("claude", FAKE_CLAUDE);
    let login =
        format!(r#"{{"type":"result","is_error":true,"result":"Not logged in /login {SECRET}"}}"#);
    let out = r.run(&["ask", "--headless", SECRET], &login, "1");
    assert_eq!(out.status.code(), Some(1), "{}", text(&out));
    assert!(text(&out).contains("not signed in"), "{}", text(&out));
    assert!(!text(&out).contains(SECRET), "{}", text(&out));
    let out = r.run(&["ask", "--headless", SECRET], "", "1");
    assert_eq!(out.status.code(), Some(1));
    assert!(!text(&out).contains(SECRET), "{}", text(&out));
}

#[test]
fn io_and_codex_still_say_not_built_yet_and_start_nothing() {
    let r = Rig::new();
    r.fake("claude", FAKE_CLAUDE);
    for args in [
        vec!["--json", "ask", "--headless", "--via", "io", "q"],
        vec!["--json", "ask", "--headless", "--agent", "codex", "q"],
    ] {
        let out = r.run(&args, OK_JSON, "0");
        assert_eq!(out.status.code(), Some(1), "{}", text(&out));
        assert!(text(&out).contains("not built yet"), "{}", text(&out));
    }
    assert_eq!(r.saw("claude.args"), "");
}

/// An answer that suggests a command and names a file of the work folder.
fn answer_json(answer: &str) -> String {
    serde_json::json!({
        "type": "result", "subtype": "success", "is_error": false,
        "result": answer, "duration_ms": 5, "num_turns": 1
    })
    .to_string()
}

const WITH_COMMAND: &str = "Check the log in notes.md, then run:\n```bash\ngit status\n```\n";

#[test]
fn the_json_answer_carries_the_suggested_command_or_null() {
    let r = Rig::new();
    r.fake("claude", FAKE_CLAUDE);
    let out = r.run(
        &["--json", "ask", "--headless", "q"],
        &answer_json(WITH_COMMAND),
        "0",
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["command"], "git status");
    let out = r.run(&["--json", "ask", "--headless", "q"], OK_JSON, "0");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(v["command"].is_null(), "{v}");
    // Two lines, or a control character, are shown but never offered.
    let two = "```bash\ncd a\nls\n```";
    let out = r.run(
        &["--json", "ask", "--headless", "q"],
        &answer_json(two),
        "0",
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(v["command"].is_null(), "{v}");
}

// The bug: a model from IOHR_ASK_MODEL took the card path, which printed the answer on stderr
// and again on stdout, so a terminal showed it twice. One renderer now: once, wherever it goes.
#[test]
fn the_answer_is_printed_once_with_a_model_from_the_environment() {
    let r = Rig::new();
    r.fake("claude", FAKE_CLAUDE);
    for args in [
        &["ask", "--headless", "--card", "hi"][..],
        &["ask", "--headless", "hi"][..],
        &["ask", "hi"][..],
    ] {
        let out = r.run_env(args, OK_JSON, "0", &[("IOHR_ASK_MODEL", "opus")]);
        assert!(out.status.success(), "{}", text(&out));
        assert_eq!(
            text(&out).matches("Rayleigh scattering").count(),
            1,
            "{args:?}: {}",
            text(&out)
        );
        assert!(r.saw("claude.args").contains("opus"));
    }
}

// Off a terminal (a pipe, or NO_COLOR) the hook's card is plain: no escape, no row under it,
// and the record on stdout is the answer, the command, the link, the undo, the folder asked
// about and the next steps, in that order.
#[test]
fn the_hook_record_is_six_fields_and_the_plain_card_has_no_row() {
    let r = Rig::new();
    r.fake("claude", FAKE_CLAUDE);
    std::fs::write(r.work().join("notes.md"), "x").unwrap();
    for no_color in [false, true] {
        let env: &[(&str, &str)] = if no_color { &[("NO_COLOR", "1")] } else { &[] };
        let out = r.run_env(
            &[
                "ask",
                "--headless",
                "--hook",
                "--cwd",
                r.work().to_str().unwrap(),
                "q",
            ],
            &answer_json(WITH_COMMAND),
            "0",
            env,
        );
        assert!(out.status.success(), "{}", text(&out));
        let stdout = String::from_utf8(out.stdout.clone()).unwrap();
        let f: Vec<&str> = stdout.split('\u{1e}').collect();
        assert_eq!(f.len(), 6, "{stdout:?}");
        assert!(
            f[0].contains("run:") && f[0].contains("git status"),
            "{f:?}"
        );
        assert_eq!(f[1], "git status");
        assert_eq!(f[2], "notes.md");
        // Nobody was asked, so no rule was saved and Undo has nothing to take back.
        assert_eq!(f[3], "");
        // A readable folder asks for nothing and the answer named no next step.
        assert_eq!((f[4], f[5]), ("", ""));
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(
            !err.contains('\u{1b}') && !err.contains("to choose"),
            "{err:?}"
        );
        assert!(
            err.contains("git status"),
            "the card shows the answer: {err}"
        );
    }
}

#[test]
fn the_menu_needs_a_terminal_and_prints_nothing_without_one() {
    let r = Rig::new();
    let out = r.run(&["ask", "--menu", "--command=ls"], "", "0");
    assert!(!out.status.success());
    assert_eq!(out.stdout, b"", "nothing for a prompt line");
    // The menu asks no model: the fake agent was never started.
    assert_eq!(r.saw("claude.args"), "");
    // `--menu` is not a question and takes none.
    let out = r.run(&["ask", "--menu", "why"], "", "0");
    assert_eq!(out.status.code(), Some(2), "{}", text(&out));
}

// ---------------------------------------------------------------- iohr shell hook

fn hook(shell: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_iohr"))
        .args(["shell", "hook", shell])
        .env_clear()
        .env("HOME", std::env::temp_dir())
        .output()
        .unwrap()
}

#[test]
fn the_zsh_hook_has_its_key_lines() {
    let o = hook("zsh");
    assert!(o.status.success());
    let t = String::from_utf8(o.stdout).unwrap();
    // The widget: a `?` or `??` line, a space, a word; the shell is handed an empty line.
    assert!(t.contains(r"'^[[:space:]]*\?\??[[:space:]]+[^[:space:]]'"));
    assert!(t.contains("BUFFER=\"\""));
    // It chains to the accept-line that was there before, never replaces it.
    assert!(t.contains("zle -A accept-line _iohr_ask_prev_accept"));
    assert!(t.contains("zle _iohr_ask_prev_accept"));
    assert!(t.contains("zle -N accept-line _iohr_ask_accept"));
    // The answer comes from `iohr ask`, headless, drawn as a card, with the question after `--`.
    // The menu key is a widget that only ever fills the buffer: no `zle -U`, no `print -z`.
    assert!(t.contains("zle -N _iohr_card_action") && t.contains("ask --menu"));
    assert!(t.contains("'^[[B' '^[OB'"), "{t}");
    assert!(!t.contains("zle -U") && !t.contains("print -z"));
    assert!(t.contains("command iohr ask --headless --hook --cwd"));
    assert!(t.contains("-- \"$q\""));
    // Inside the InOrbit app it speaks the app's OSC 7727 ask protocol.
    assert!(t.contains(r"\e]7727;ask;"));
    assert!(t.contains("INORBIT_TERM"));
    assert!(t.contains("INORBIT_ASK"));
    // It never runs the answer.
    assert!(!t.contains("eval \"$answer\"") && !t.contains("eval $answer"));
    // Evaluated twice, or beside the app's own hook, it installs once.
    assert!(t.contains("${+widgets[_iohr_ask_prev_accept]}") && t.contains("_inorbit_accept"));
    // The mode key (PRD 0197): Shift+Tab, the ring in the order manual, plan, auto, and the
    // mode goes to every ask as `--mode`. Nothing is left to fill in.
    assert!(t.contains("_iohr_ask_ring=(manual plan auto)"), "{t}");
    assert!(
        t.contains("bindkey -M $_iohr_km -- '^[[Z' _iohr_ask_cycle"),
        "{t}"
    );
    assert!(t.contains("a+=(--mode \"$_iohr_ask_mode\")"), "{t}");
    // With nothing named it starts in manual, the mode a terminal gets anyway, so it is shown.
    assert!(t.contains("_iohr_ask_mode='manual'"), "{t}");
    assert!(!t.contains("__IOHR_"), "{t}");
}

// The hook starts in the mode `IOHR_ASK_MODE` or `[ask] mode` name; `deny` and `rules` are not
// of the switch, so the hook starts empty and the config keeps its say.
#[test]
fn the_hook_starts_in_the_mode_the_environment_or_config_names() {
    let dir = tempfile::tempdir().unwrap();
    let cfgdir = dir.path().join("cfg");
    std::fs::create_dir_all(&cfgdir).unwrap();
    let hook_with = |cfg: &str, env: &[(&str, &str)]| {
        std::fs::write(cfgdir.join("config.toml"), cfg).unwrap();
        Command::new(env!("CARGO_BIN_EXE_iohr"))
            .args(["shell", "hook", "zsh"])
            .env_clear()
            .envs(env.iter().copied())
            .env("HOME", dir.path())
            .env("IOHR_CONFIG_DIR", &cfgdir)
            .output()
            .unwrap()
    };
    let start = |o: &Output| {
        let t = String::from_utf8_lossy(&o.stdout).into_owned();
        t.lines()
            .find(|l| l.contains("typeset -g _iohr_ask_mode="))
            .unwrap_or_default()
            .trim()
            .to_owned()
    };
    let o = hook_with("[ask]\nmode = \"plan\"\n", &[]);
    assert_eq!(start(&o), "typeset -g _iohr_ask_mode='plan'");
    // The environment beats the file.
    let o = hook_with("[ask]\nmode = \"plan\"\n", &[("IOHR_ASK_MODE", "auto")]);
    assert_eq!(start(&o), "typeset -g _iohr_ask_mode='auto'");
    // Nothing named: manual, what a terminal gets anyway.
    let o = hook_with("", &[]);
    assert_eq!(start(&o), "typeset -g _iohr_ask_mode='manual'");
    // An older word that is not of the switch starts empty.
    let o = hook_with("[ask]\npermissions = \"deny\"\n", &[]);
    assert_eq!(start(&o), "typeset -g _iohr_ask_mode=''");
    // A bad word is a usage error here, as it is in `iohr ask`.
    let o = hook_with("", &[("IOHR_ASK_MODE", "wild")]);
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("manual, plan or auto"));
}

// Installed under another name (the preview channel's `iohr-preview`), the hook asks that
// binary, not whatever `iohr` is first on the PATH.
#[test]
fn the_hook_asks_the_binary_that_printed_it() {
    let dir = tempfile::tempdir().unwrap();
    let copy = dir.path().join("iohr-preview");
    std::fs::copy(env!("CARGO_BIN_EXE_iohr"), &copy).unwrap();
    let o = Command::new(&copy)
        .args(["shell", "hook", "zsh"])
        .env_clear()
        .env("HOME", dir.path())
        .output()
        .unwrap();
    assert!(o.status.success());
    let t = String::from_utf8(o.stdout).unwrap();
    assert!(
        t.contains("command iohr-preview ask --headless --hook --cwd"),
        "{t}"
    );
    assert!(!t.contains("__IOHR_BIN__"));
}

#[test]
fn the_zsh_hook_is_valid_zsh_and_runs_the_ask_with_its_context() {
    if Command::new("zsh").arg("--version").output().is_err() {
        return;
    }
    let z = Zsh::new();
    let (dir, file, bin) = (&z.dir, &z.file, &z.bin);
    let n = Command::new("zsh").arg("-n").arg(file).output().unwrap();
    assert!(n.status.success(), "{}", String::from_utf8_lossy(&n.stderr));
    // The precmd, with a fake `iohr` that prints its arguments.
    z.fake("#!/bin/sh\necho \"ARGS: $*\" >&2\nprintf 'the answer\\036ls -la\\036README.md'\n");
    let zsh = |script: &str| {
        Command::new("zsh")
            .args(["-f", "-i", "-c", script])
            .current_dir(dir.path())
            .env_clear()
            .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
            .env("HOME", dir.path())
            .output()
            .unwrap()
    };
    let o = zsh(&format!(
        "source {f}; _iohr_ask_q='?? and red'; _iohr_ask_pq=first _iohr_ask_pa=reply; \
         _iohr_ask_precmd; _iohr_ask_q='? plain'; _iohr_ask_precmd; print -r -- \"kept: $_iohr_ask_pa\"",
        f = file.display()
    ));
    let out = String::from_utf8_lossy(&o.stdout);
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(
        err.contains("--prior-question first --prior-answer reply -- and red"),
        "{err}"
    );
    assert!(
        err.contains("-- plain") && !err.contains("-- plain --prior"),
        "{err}"
    );
    // `iohr ask --hook` draws the card on stderr; the hook keeps the record on stdout: the
    // answer for `??`, the command and the link for the menu.
    assert!(err.contains("--headless --hook --cwd"), "{err}");
    assert_eq!(out.trim(), "kept: the answer", "{out}");
    let o = zsh(&format!(
        "source {f}; _iohr_ask_q='? plain'; _iohr_ask_precmd; \
         print -r -- \"$_iohr_ask_cmd|$_iohr_ask_link|$_iohr_ask_live\"",
        f = file.display()
    ));
    assert_eq!(
        String::from_utf8_lossy(&o.stdout).trim(),
        "ls -la|README.md|1"
    );
    // Inside the app: the OSC, not a call to iohr.
    let o = zsh(&format!(
        "source {f}; INORBIT_TERM=1; _iohr_ask_q='? hi'; _iohr_ask_precmd",
        f = file.display()
    ));
    let out = String::from_utf8_lossy(&o.stdout);
    assert!(out.starts_with("\u{1b}]7727;ask;6;"), "{out:?}");
    assert!(!String::from_utf8_lossy(&o.stderr).contains("ARGS:"));
}

/// Whether `program` runs here (a test that needs it returns when it does not).
fn have(program: &str) -> bool {
    Command::new(program).arg("--version").output().is_ok()
}

/// A folder with the hook file and a `bin` for a fake `iohr`, for the tests that source the
/// zsh hook.
struct Zsh {
    dir: tempfile::TempDir,
    file: PathBuf,
    bin: PathBuf,
}

impl Zsh {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("hook.zsh");
        std::fs::write(&file, hook("zsh").stdout).unwrap();
        let bin = dir.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        Self { dir, file, bin }
    }

    /// The `iohr` the hook finds on its PATH: `script` is the whole program.
    fn fake(&self, script: impl AsRef<[u8]>) {
        let fake = self.bin.join("iohr");
        std::fs::write(&fake, script).unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

/// Drives a real interactive zsh on a pty (python3's `pty`; the test is skipped where zsh
/// or python3 is missing). `lines` are typed one by one; the screen after each is returned.
/// `RAW:` lines are keys as they are, `WINCH:` resizes the terminal and `SIG:NAME` signals the
/// shell's process group.
fn drive_zsh(hook_file: &std::path::Path, bin: &std::path::Path, lines: &[String]) -> Vec<String> {
    const DRIVER: &str = r#"
import fcntl, os, pty, select, signal, struct, sys, termios, time
bindir, home = sys.argv[1], sys.argv[2]
lines = sys.argv[3:]
pid, fd = pty.fork()
if pid == 0:
    os.environ.clear()
    os.environ.update(PATH=bindir + ":/usr/bin:/bin", HOME=home, PS1="PROMPT> ", TERM="dumb")
    os.chdir(home)
    os.execvp("zsh", ["zsh", "-f", "-i"])
def rd(t):
    out = b""
    end = time.time() + t
    while time.time() < end:
        r, _, _ = select.select([fd], [], [], 0.1)
        if r:
            try:
                out += os.read(fd, 4096)
            except OSError:
                break
    return out
rd(1.0)
shown = []
for l in lines:
    # "RAW:" lines are keys, sent as they are; the others are typed with Enter.
    if l.startswith("RAW:"):
        os.write(fd, l[4:].encode())
    elif l.startswith("WINCH:"):
        # The terminal is resized: SIGWINCH to the foreground group, as a window drag does.
        fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 100, 0, 0))
    elif l.startswith("SIG:"):
        # What an async prompt's worker does to the shell's group (the shell and its children).
        os.killpg(pid, getattr(signal, "SIG" + l[4:]))
    else:
        os.write(fd, l.encode() + b"\n")
    shown.append(rd(1.2).decode(errors="replace"))
os.write(fd, b"exit\n")
rd(0.3)
sys.stdout.write("\x00".join(shown))
"#;
    let _ = hook_file;
    let o = Command::new("python3")
        .args(["-I", "-c", DRIVER])
        .arg(bin)
        .arg(bin.parent().unwrap())
        .args(lines)
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8_lossy(&o.stdout)
        .split('\0')
        .map(str::to_owned)
        .collect()
}

#[test]
fn evaluating_the_hook_twice_keeps_enter_working_and_wires_accept_line_once() {
    if !have("zsh") || !have("python3") {
        return;
    }
    let z = Zsh::new();
    let (file, bin) = (&z.file, &z.bin);
    z.fake("#!/bin/sh\necho \"ASKED: $*\" >&2\necho the answer >&2\necho the answer\n");
    let f = file.display();
    // A second `source ~/.zshrc`, then once more with a shell variable of the old guard
    // unset (state a plugin may reset): the wiring must not copy our own widget into the one
    // accept-line calls, or Enter loops until FUNCNEST.
    let out = drive_zsh(
        file,
        bin,
        &[
            format!("source {f}"),
            format!("source {f}"),
            format!("unset _iohr_ask_hooked; source {f}"),
            "echo AFTER-$((1+1))".to_owned(),
            "? still a question".to_owned(),
            "echo LAST-$((2+2))".to_owned(),
        ],
    );
    for o in &out {
        assert!(
            !o.contains("FUNCNEST") && !o.contains("nested function"),
            "{o:?}"
        );
    }
    assert!(out[3].contains("AFTER-2"), "{:?}", out[3]);
    assert_eq!(
        out[4].matches("ASKED:").count(),
        1,
        "a ? line still reaches the ask path: {:?}",
        out[4]
    );
    assert!(out[4].contains("the answer"), "{:?}", out[4]);
    assert!(out[5].contains("LAST-4"), "{:?}", out[5]);
}

/// Runs `iohr ask --menu ...` on a pty with the environment `env`, presses `key`, and gives
/// back what reached stdout (the one line for the prompt) and the screen. None where python3
/// is missing, so the test is skipped there.
fn press_in_menu(
    r: &Rig,
    args: &[&str],
    key: &str,
    env: &[(&str, &str)],
) -> Option<(String, String)> {
    const DRIVER: &str = r#"
import fcntl, json, os, pty, select, signal, struct, sys, termios, time
env, key, outfile, work = json.loads(sys.argv[1]), sys.argv[2], sys.argv[3], sys.argv[4]
argv = sys.argv[5:]
pid, fd = pty.fork()
if pid == 0:
    os.environ.clear()
    os.environ.update(env)
    os.environ["OUTFILE"] = outfile
    os.chdir(work)
    os.execv("/bin/sh", ["sh", "-c", 'exec "$@" >"$OUTFILE"', "sh", *argv])
def rd(t):
    out = b""
    end = time.time() + t
    while time.time() < end:
        x, _, _ = select.select([fd], [], [], 0.1)
        if x:
            try:
                out += os.read(fd, 4096)
            except OSError:
                break
    return out
screen = rd(0.5)
limit = time.time() + 10
while not screen and time.time() < limit:  # a slow start (a loaded machine) must not eat the key
    screen = rd(0.2)
# `key` is one key, or steps split by the unit separator (0x1f): keys as they are, `WINCH:` (resize) and `SIG:NAME`.
for step in key.split("\x1f"):
    try:
        if step.startswith("WINCH:"):
            fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 100, 0, 0))
        elif step.startswith("SIG:"):
            os.kill(pid, getattr(signal, "SIG" + step[4:]))
        else:
            os.write(fd, step.encode())
    except OSError:
        pass  # the menu is gone: its stdout says what it printed
    screen += rd(0.5)
sys.stdout.write(screen.decode(errors="replace"))
"#;
    if Command::new("python3").arg("--version").output().is_err() {
        return None;
    }
    let outfile = r.home.path().join("menu.stdout");
    let mut all: Vec<(&str, String)> = vec![
        ("PATH", format!("{}:/usr/bin:/bin", r.bin().display())),
        ("HOME", r.home.path().display().to_string()),
        (
            "IOHR_CONFIG_DIR",
            r.home.path().join("config").display().to_string(),
        ),
        ("TERM", "xterm".to_owned()),
    ];
    all.extend(env.iter().map(|(k, v)| (*k, (*v).to_owned())));
    let json = serde_json::to_string(&all.into_iter().collect::<std::collections::HashMap<_, _>>())
        .unwrap();
    let o = Command::new("python3")
        .args(["-I", "-c", DRIVER, &json, key])
        .arg(&outfile)
        .arg(r.work())
        .arg(env!("CARGO_BIN_EXE_iohr"))
        .args(args)
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let line = std::fs::read_to_string(&outfile).unwrap_or_default();
    Some((line, String::from_utf8_lossy(&o.stdout).into_owned()))
}

// The menu's letters: Run prints the command and the `run` marker, Explain the one line for the
// prompt, nothing else; Copy and Esc print nothing; Copy goes through tmux when inside it.
// `iohr` never runs the command: the shell hook does, and it only gets a line.
#[test]
fn the_menu_letters_print_one_prompt_line_or_nothing_and_copy_uses_tmux() {
    let r = Rig::new();
    let tmux_in = r.home.path().join("tmux.stdin");
    r.fake(
        "tmux",
        &format!(
            "#!/bin/sh\necho \"$*\" > {a}.args\ncat > {a}\n",
            a = tmux_in.display()
        ),
    );
    let cmd = ["ask", "--menu", "--command=git status", "--link=notes.md"];
    let Some((line, screen)) = press_in_menu(&r, &cmd, "r", &[]) else {
        return;
    };
    assert_eq!(line, "git status\u{1e}run", "{screen:?}");
    assert!(screen.contains("What next?"), "{screen:?}");
    let (line, _) = press_in_menu(&r, &cmd, "e", &[]).unwrap();
    assert_eq!(line, "?? explain this command: git status");
    let (line, _) = press_in_menu(&r, &cmd, "f", &[]).unwrap();
    assert_eq!(line, "?? ");
    let (line, _) = press_in_menu(&r, &cmd, "\u{1b}", &[]).unwrap();
    assert_eq!(line, "", "Esc leaves the line alone");
    // A command with a control character or a second line is never offered, so Run is not a key.
    let (line, screen) =
        press_in_menu(&r, &["ask", "--menu", "--command=ls\u{7}"], "r", &[]).unwrap();
    assert_eq!(line, "", "{screen:?}");
    // Copy: inside tmux the text goes to `tmux load-buffer -w -`, and the line stays empty.
    let (line, screen) = press_in_menu(&r, &cmd, "c", &[("TMUX", "/tmp/tmux-1,1,0")]).unwrap();
    assert_eq!(line, "");
    assert!(screen.contains("tmux"), "{screen:?}");
    assert_eq!(std::fs::read_to_string(&tmux_in).unwrap(), "git status");
    assert_eq!(
        std::fs::read_to_string(format!("{}.args", tmux_in.display()))
            .unwrap()
            .trim(),
        "load-buffer -w -"
    );
}

// Add folder refuses the home folder and the root where they are typed, with a reason, and
// keeps nothing in `trusted.toml`; a project folder is kept as a standing folder.
#[test]
fn add_folder_refuses_home_and_root_and_keeps_a_project_folder_as_standing() {
    let r = Rig::new();
    let cmd = ["ask", "--menu", "--command=ls"];
    let trusted = r.home.path().join("config").join("trusted.toml");
    for typed in ["~", "/"] {
        let keys = format!("d\u{1f}{typed}\u{1f}\r\u{1f}\u{1b}\u{1f}\u{1b}");
        let Some((line, screen)) = press_in_menu(&r, &cmd, &keys, &[]) else {
            return;
        };
        let plain_screen = strip_escapes(&screen);
        assert!(plain_screen.contains("too broad"), "{typed}: {screen:?}");
        assert_eq!(line, "", "{typed}: nothing is put on the prompt");
        assert!(!trusted.exists(), "{typed}: nothing was kept");
    }
    let project = r.home.path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    let keys = "d\u{1f}~/project\u{1f}\r";
    let (line, screen) = press_in_menu(&r, &cmd, keys, &[]).unwrap();
    let kept = std::fs::read_to_string(&trusted).unwrap_or_default();
    assert!(kept.contains("standing"), "{screen:?} {line:?} {kept:?}");
    assert!(kept.contains("project"), "{kept:?}");
}

// PRD 0187.1: the menu is a bar of numbered rows in a box under the card. A digit picks a row
// at once, so do its letter and Enter on the highlighted row; Esc leaves nothing on the screen
// and nothing on stdout; with NO_COLOR it is plain numbered lines.
/// `text` without its `ESC [ ... letter` sequences.
fn strip_escapes(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            for n in chars.by_ref() {
                if n.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[test]
fn the_action_bar_picks_by_number_letter_or_arrows_and_esc_leaves_nothing() {
    let r = Rig::new();
    let cmd = ["ask", "--menu", "--command=git status", "--link=notes.md"];
    let Some((line, screen)) = press_in_menu(&r, &cmd, "1", &[]) else {
        return;
    };
    assert_eq!(line, "git status\u{1e}run", "1 is Run: {screen:?}");
    let plain_screen = strip_escapes(&screen);
    for want in [
        "\u{256d}\u{2500} What next?",
        "[1] Run",
        "[2] Copy",
        "[3] Follow up",
        "[4] Explain",
        "[5] Open",
        "\u{2191}\u{2193} move \u{b7} 1-9 or letter pick \u{b7} enter \u{b7} esc",
    ] {
        assert!(plain_screen.contains(want), "{want}: {screen:?}");
    }
    let (line, _) = press_in_menu(&r, &cmd, "3", &[]).unwrap();
    assert_eq!(line, "?? ", "3 is Follow up");
    let (line, _) = press_in_menu(&r, &cmd, "4", &[]).unwrap();
    assert_eq!(line, "?? explain this command: git status");
    let (line, _) = press_in_menu(&r, &cmd, "9", &[]).unwrap();
    assert_eq!(
        line, "",
        "no ninth row: the bar holds, then the end of input leaves"
    );
    // Arrows (and j) move, Enter picks the highlighted row.
    let (line, _) = press_in_menu(&r, &cmd, "\u{1b}[B\u{1f}\u{1b}[B\u{1f}\r", &[]).unwrap();
    assert_eq!(line, "?? ", "Down twice from Run, then Enter");
    let (line, _) = press_in_menu(&r, &cmd, "j\u{1f}j\u{1f}j\u{1f}k\u{1f}\r", &[]).unwrap();
    assert_eq!(line, "?? ", "j j j k from Run is Follow up");
    // Esc and q leave: nothing on stdout, and every line the bar drew is erased again.
    for key in ["\u{1b}", "q"] {
        let (line, screen) = press_in_menu(&r, &cmd, key, &[]).unwrap();
        assert_eq!(line, "", "{key:?}");
        let drawn = screen.matches('\n').count();
        let erased = screen.matches("\u{1b}[2K").count();
        assert!(
            drawn >= 7 && erased >= 7,
            "{key:?}: drew {drawn}, erased {erased}: {screen:?}"
        );
    }
}

// Run goes through the permission rules before the hook executes it: a plain command goes at
// once, a risky one asks first (a yes sends it, a no or Esc sends nothing), a command a rule
// denies is never sent, and nothing is sent while the rule file cannot be read.
#[test]
fn run_asks_for_a_risky_command_and_never_sends_a_denied_one() {
    let r = Rig::new();
    let work = r.work().display().to_string();
    let plain = ["ask", "--menu", "--cwd", &work, "--command=git status"];
    let Some((line, _)) = press_in_menu(&r, &plain, "r", &[]) else {
        return;
    };
    assert_eq!(line, "git status\u{1e}run", "a plain command runs at once");

    let risky = ["ask", "--menu", "--cwd", &work, "--command=rm -rf build"];
    let (line, screen) = press_in_menu(&r, &risky, "r\u{1f}y", &[]).unwrap();
    assert_eq!(line, "rm -rf build\u{1e}run", "a yes sends it: {screen:?}");
    let plain_screen = strip_escapes(&screen);
    assert!(
        plain_screen.contains("Run it?") && plain_screen.contains("deletes files"),
        "it asked first and said why: {screen:?}"
    );
    for no in ["r\u{1f}n", "r\u{1f}\u{1b}"] {
        let (line, screen) = press_in_menu(&r, &risky, no, &[]).unwrap();
        assert_eq!(line, "", "{no:?} sends nothing: {screen:?}");
    }

    let rules = r.work().join(".inorbit/permissions.toml");
    std::fs::create_dir_all(rules.parent().unwrap()).unwrap();
    std::fs::write(&rules, "deny = [\"Bash(git:*)\"]\n").unwrap();
    let (line, screen) = press_in_menu(&r, &plain, "r", &[]).unwrap();
    assert_eq!(line, "", "a denied command is never sent: {screen:?}");
    assert!(
        strip_escapes(&screen).contains("does not allow it"),
        "{screen:?}"
    );
    std::fs::write(&rules, "deny = [\"Bash(\"]\n").unwrap();
    let (line, screen) = press_in_menu(&r, &plain, "r", &[]).unwrap();
    assert_eq!(line, "", "{screen:?}");
    assert!(
        strip_escapes(&screen).contains("cannot be read"),
        "{screen:?}"
    );
}

// A command that still holds a placeholder is Copy only: the bar has no Run and its key sends
// nothing; Copy is still there.
#[test]
fn a_command_with_a_placeholder_has_no_run() {
    let r = Rig::new();
    let work = r.work().display().to_string();
    let cmd = [
        "ask",
        "--menu",
        "--cwd",
        &work,
        "--command=cd path/to/your/repo && git log --oneline -5",
    ];
    let Some((line, screen)) = press_in_menu(&r, &cmd, "r", &[("NO_COLOR", "1")]) else {
        return;
    };
    assert_eq!(line, "", "no Run to choose: {screen:?}");
    assert!(
        screen.contains("[1] Copy") && !screen.contains("Run"),
        "{screen:?}"
    );
}

#[test]
fn with_no_color_the_action_bar_is_plain_numbered_lines() {
    let r = Rig::new();
    let cmd = ["ask", "--menu", "--command=git status"];
    let Some((line, screen)) = press_in_menu(&r, &cmd, "2", &[("NO_COLOR", "1")]) else {
        return;
    };
    assert_eq!(line, "", "2 is Copy, which prints nothing on stdout");
    for want in [
        "What next?",
        "[1] Run",
        "[2] Copy",
        "[3] Follow up",
        "[4] Explain",
    ] {
        assert!(screen.contains(want), "{want}: {screen:?}");
    }
    assert!(
        !screen.contains('\u{2502}') && !screen.contains('\u{256d}'),
        "{screen:?}"
    );
    // No colour or style sequences (`ESC [ ... m`); the cursor ones that erase the bar stay.
    let mut rest = screen.as_str();
    while let Some(i) = rest.find("\u{1b}[") {
        rest = &rest[i + 2..];
        let end = rest.find(|c: char| c.is_ascii_alphabetic()).unwrap();
        assert_ne!(&rest[end..=end], "m", "colour in NO_COLOR: {screen:?}");
    }
}

// The widget on a real zsh: the key opens the menu under a live answer on an empty line, the
// line it prints (no `run` marker) lands in the buffer and nothing runs until Enter; with text on
// the line, or with no answer yet, the key does what it did.
#[test]
fn the_menu_key_fills_the_prompt_line_and_nothing_runs_until_enter() {
    if !have("zsh") || !have("python3") {
        return;
    }
    let z = Zsh::new();
    let (dir, file, bin) = (&z.dir, &z.file, &z.bin);
    let log = dir.path().join("menu.log");
    z.fake(
        format!(
            "#!/bin/sh\ncase \"$*\" in\n*--menu*) echo \"$*\" >> {log}; printf '%s' 'echo RAN-$((1+1))';;\n*) printf 'the answer\\036echo RAN-$((1+1))\\036notes.md';;\nesac\n",
            log = log.display()
        ),
    );
    let down = "RAW:\u{1b}[B";
    let out = drive_zsh(
        file,
        bin,
        &[
            format!("source {}", file.display()),
            // No answer yet: Down is the shell's own, the menu is not asked.
            down.to_owned(),
            "? a question".to_owned(),
            // Text on the line: the key keeps its meaning, the menu is not asked.
            "RAW:abc".to_owned(),
            down.to_owned(),
            "RAW:\u{15}".to_owned(), // ctrl-u clears the line
            // An answer and an empty line: the menu, and its line lands in the buffer.
            down.to_owned(),
            // The line is only typed, not run, until Enter.
            "RAW:\r".to_owned(),
        ],
    );
    let asked = std::fs::read_to_string(&log).unwrap_or_default();
    assert_eq!(asked.lines().count(), 1, "one menu only: {asked:?}");
    assert!(
        asked.contains("--command=echo RAN-$((1+1))") && asked.contains("--link=notes.md"),
        "{asked}"
    );
    assert!(out[3].contains("abc"), "{out:?}");
    let on_line = &out[6];
    assert!(on_line.contains("echo RAN-$((1+1))"), "{out:?}");
    assert!(
        !on_line.contains("RAN-2"),
        "the line was not run: {on_line:?}"
    );
    assert!(out[7].contains("RAN-2"), "Enter runs it: {:?}", out[7]);
}

/// A fake `iohr` for the pty tests below: an ask answers with a command, `--menu` is logged.
fn fake_menu_iohr(dir: &std::path::Path) -> (std::path::PathBuf, std::path::PathBuf) {
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let log = dir.join("menu.log");
    let fake = bin.join("iohr");
    std::fs::write(
        &fake,
        format!(
            "#!/bin/sh\ncase \"$*\" in\n*--menu*) echo \"$*\" >> {log}; printf '%s' 'echo RAN-$((1+1))';;\n*) printf 'the answer\\036echo RAN-$((1+1))\\036notes.md';;\nesac\n",
            log = log.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    (bin, log)
}

// The card stays live across an empty Enter: an answer, then Enter on the empty prompt, then
// Down still opens the menu (the owner's "it works sometimes"). A command that is run, or a
// line of spaces, behaves as it should: spaces keep the card, a command ends it.
#[test]
fn the_menu_still_opens_after_an_empty_enter_and_ends_with_a_command() {
    let have = |c: &str| Command::new(c).arg("--version").output().is_ok();
    if !have("zsh") || !have("python3") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("hook.zsh");
    std::fs::write(&file, hook("zsh").stdout).unwrap();
    let (bin, log) = fake_menu_iohr(dir.path());
    let down = "RAW:\u{1b}[B";
    let out = drive_zsh(
        &file,
        &bin,
        &[
            format!("source {}", file.display()),
            "? a question".to_owned(),
            "RAW:\r".to_owned(), // Enter on the empty prompt
            down.to_owned(),
            "RAW:\u{15}".to_owned(), // ctrl-u clears the line the menu filled
            "RAW:   \r".to_owned(),  // Enter on spaces only
            down.to_owned(),
            "RAW:\u{15}".to_owned(),
            "echo PLAIN-$((1+1))".to_owned(), // a command ends the card
            down.to_owned(),
        ],
    );
    let asked = std::fs::read_to_string(&log).unwrap_or_default();
    assert_eq!(
        asked.lines().count(),
        2,
        "the menu opens after an empty Enter and after spaces, not after a command: {asked:?}\n{out:?}"
    );
    assert!(out[8].contains("PLAIN-2"), "{out:?}");
}

// Sourcing the hook again (a re-sourced ~/.zshrc after an upgrade) leaves accept-line wrapped
// once, rebinds the menu key when something took it, and keeps the key it replaced.
#[test]
fn evaluating_the_hook_again_rebinds_the_menu_key_without_wrapping_twice() {
    let have = |c: &str| Command::new(c).arg("--version").output().is_ok();
    if !have("zsh") || !have("python3") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("hook.zsh");
    std::fs::write(&file, hook("zsh").stdout).unwrap();
    let (bin, log) = fake_menu_iohr(dir.path());
    let down = "RAW:\u{1b}[B";
    let f = file.display();
    let out = drive_zsh(
        &file,
        &bin,
        &[
            format!("source {f}"),
            format!("source {f}"),
            // Something else takes Down (an older hook's shell, a history plugin).
            "bindkey '^[[B' down-line-or-history".to_owned(),
            format!("source {f}"),
            "print -r -- \"W[${widgets[accept-line]}][${widgets[_iohr_ask_prev_accept]}][${_iohr_card_prev[emacs]}]\"".to_owned(),
            "? a question".to_owned(),
            down.to_owned(),
        ],
    );
    assert!(
        out[4].contains("W[user:_iohr_ask_accept][builtin][down-line-or-history]"),
        "accept-line is wrapped once and the first previous key is kept: {:?}",
        out[4]
    );
    let asked = std::fs::read_to_string(&log).unwrap_or_default();
    assert_eq!(
        asked.lines().count(),
        1,
        "Down opens the menu: {asked:?}\n{out:?}"
    );
}

// A chosen question is asked at once, with no second Enter: the menu prints a whole `?` or `??`
// line (a step, Explain, Allow), the widget hands it to accept-line (our own widget, so the shell
// never sees it) and the ask runs. Follow up prints a bare `?? ` for the person to finish: it
// stays on the line, and nothing is asked until Enter.
#[test]
fn a_chosen_question_is_asked_at_once_and_a_bare_follow_up_waits_for_the_person() {
    if !have("zsh") || !have("python3") {
        return;
    }
    let z = Zsh::new();
    let (dir, file, bin) = (&z.dir, &z.file, &z.bin);
    let log = dir.path().join("asks.log");
    let mode = dir.path().join("menu.mode");
    z.fake(format!(
        "#!/bin/sh\ncase \"$*\" in\n*--menu*) cat {mode};;\n*) printf '%s\\n' \"$*\" >> {log}; printf 'the answer\\036\\036\\036\\036\\036step';;\nesac\n",
        mode = mode.display(),
        log = log.display()
    ));
    // The log also holds the background `asks count --cache` refresh: only the questions count.
    let questions = |f: &std::path::Path| {
        std::fs::read_to_string(f)
            .unwrap_or_default()
            .lines()
            .filter(|l| l.starts_with("ask --headless"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let down = "RAW:\u{1b}[B";
    // First menu: a step. Second menu: Follow up.
    std::fs::write(&mode, "?? explain what is in api/").unwrap();
    let out = drive_zsh(
        file,
        bin,
        &[
            format!("source {}", file.display()),
            "? a question".to_owned(),
            down.to_owned(),
        ],
    );
    let asked = questions(&log);
    assert_eq!(
        asked.lines().count(),
        2,
        "the chosen question ran once: {asked:?}\n{out:?}"
    );
    assert!(
        asked
            .lines()
            .last()
            .unwrap()
            .ends_with("-- explain what is in api/"),
        "{asked}"
    );
    assert!(!out[2].contains("FUNCNEST"), "{:?}", out[2]);

    // Follow up: a bare `?? ` stays on the line.
    std::fs::remove_file(&log).unwrap();
    std::fs::write(&mode, "?? ").unwrap();
    let out = drive_zsh(
        file,
        bin,
        &[
            format!("source {}", file.display()),
            "? a question".to_owned(),
            down.to_owned(),
            // The bare `?? ` is on the line, not asked; clear it so the driver's `exit` is not
            // taken for its question.
            "RAW:\u{15}".to_owned(),
        ],
    );
    assert!(out[2].contains("?? "), "{out:?}");
    let asked = questions(&log);
    assert_eq!(
        asked.lines().count(),
        1,
        "only the first question: {asked:?}\n{out:?}"
    );
}

// Choosing Run executes the command at once, with no second Enter: the menu prints the command
// and the `run` marker, and the widget hands the command to accept-line like a typed line. A
// bare command with no marker (an old `iohr`) still only lands on the line.
#[test]
fn a_chosen_run_is_executed_at_once_through_accept_line() {
    if !have("zsh") || !have("python3") {
        return;
    }
    let z = Zsh::new();
    let (dir, file, bin) = (&z.dir, &z.file, &z.bin);
    let log = dir.path().join("menu.log");
    z.fake(format!(
        "#!/bin/sh\ncase \"$*\" in\n*--menu*) echo \"$*\" >> {log}; printf 'echo RAN-$((1+1))\\036run';;\n*) printf 'the answer\\036echo RAN-$((1+1))\\036notes.md';;\nesac\n",
        log = log.display()
    ));
    let out = drive_zsh(
        file,
        bin,
        &[
            format!("source {}", file.display()),
            "? a question".to_owned(),
            // The answer is up and the line is empty: Down opens the menu, and Run is chosen.
            "RAW:\u{1b}[B".to_owned(),
        ],
    );
    let asked = std::fs::read_to_string(&log).unwrap_or_default();
    assert_eq!(asked.lines().count(), 1, "one menu only: {asked:?}");
    assert!(
        out[2].contains("RAN-2"),
        "Run executed with no Enter: {:?}",
        out[2]
    );
    assert!(
        !out[2].contains('\u{1e}'),
        "the marker is never typed: {:?}",
        out[2]
    );
}

// Shift+Tab on a `?` line goes on from manual (where every shell starts) to plan, auto and round
// again, says which, and every
// question after it carries `--mode`. On another line the key is the shell's own.
#[test]
fn shift_tab_on_a_question_line_cycles_the_mode_and_every_ask_carries_it() {
    if !have("zsh") || !have("python3") {
        return;
    }
    let z = Zsh::new();
    let (dir, file, bin) = (&z.dir, &z.file, &z.bin);
    let log = dir.path().join("asks.log");
    z.fake(format!(
        "#!/bin/sh\necho \"$*\" >> {log}\nprintf 'the answer\\036ls\\036notes.md'\n",
        log = log.display()
    ));
    let tab = "RAW:\u{1b}[Z";
    let out = drive_zsh(
        file,
        bin,
        &[
            format!("source {}", file.display()),
            // Not a question line: Shift+Tab leaves the mode alone.
            "RAW:abc".to_owned(),
            tab.to_owned(),
            "RAW:\u{15}".to_owned(),
            "RAW:? hi".to_owned(),
            tab.to_owned(),
            tab.to_owned(),
            tab.to_owned(),
            tab.to_owned(),
            "RAW:\u{15}".to_owned(),
            "RAW:? one".to_owned(),
            tab.to_owned(),
            "RAW:\r".to_owned(),
            // The mode lasts: the next question carries it too.
            "? two".to_owned(),
        ],
    );
    for (at, mode) in [(5, "plan"), (6, "auto"), (7, "manual"), (8, "plan")] {
        assert!(
            out[at].contains(&format!("ask mode: {mode}")),
            "{at}: {out:?}"
        );
    }
    assert!(!out[2].contains("ask mode"), "{out:?}");
    let asked = std::fs::read_to_string(&log).unwrap_or_default();
    let modes: Vec<&str> = asked.lines().filter(|l| l.contains("--headless")).collect();
    assert_eq!(modes.len(), 2, "{asked}");
    assert!(modes.iter().all(|l| l.contains("--mode auto")), "{asked}");
}

// The owner's bug (RFC 0203.1): the menu "does not hold, just exits". It must wait for a
// choice through a resize, the signals of an async prompt's worker, and the application
// cursor keys a shell's line editor turns on (`ESC O B` for Down; `ESC O F` and `ESC [ 1 ; 5 C`
// used to pick Follow up and Copy by their last letter). Only the letter at the end picks.
#[test]
fn the_menu_holds_through_resizes_signals_and_application_cursor_keys() {
    let r = Rig::new();
    let cmd = ["ask", "--menu", "--command=git status", "--link=notes.md"];
    let steps = [
        "WINCH:",
        "SIG:CHLD",
        "SIG:WINCH",
        "SIG:CONT",
        "\u{1b}OB",
        "\u{1b}OA",
        "\u{1b}OC",
        "\u{1b}OF",
        "\u{1b}[1;5C",
        "\u{1b}[B",
        "x",
        "e",
    ]
    .join("\u{1f}");
    let Some((line, screen)) = press_in_menu(&r, &cmd, &steps, &[]) else {
        return;
    };
    assert_eq!(line, "?? explain this command: git status", "{screen:?}");
}

// The same on a real zsh with the hook: the key opens the menu, an async worker's signals and
// a resize reach the shell's group while it is open, and the menu still waits for the letter.
#[test]
fn the_menu_under_zsh_holds_through_signals_and_a_resize() {
    if !have("zsh") || !have("python3") {
        return;
    }
    let z = Zsh::new();
    let (file, bin) = (&z.file, &z.bin);
    // The answer is fake; the menu is the real binary.
    z.fake(
        format!(
            "#!/bin/sh\ncase \"$*\" in\n*--menu*) exec {real} \"$@\";;\n*) printf 'the answer\\036git status\\036notes.md';;\nesac\n",
            real = env!("CARGO_BIN_EXE_iohr")
        ),
    );
    let out = drive_zsh(
        file,
        bin,
        &[
            format!("source {}", file.display()),
            "? a question".to_owned(),
            "RAW:\u{1b}[B".to_owned(), // Down on the empty line: the menu
            "WINCH:".to_owned(),
            "SIG:CHLD".to_owned(),
            "SIG:WINCH".to_owned(),
            "RAW:\u{1b}OB".to_owned(),
            "RAW:\u{1b}OF".to_owned(),
            "RAW:e".to_owned(),
        ],
    );
    assert!(out[2].contains("What next?"), "{out:?}");
    let want = "?? explain this command: git status";
    for (i, screen) in out.iter().enumerate().take(8).skip(3) {
        assert!(!screen.contains(want), "left the menu at step {i}: {out:?}");
    }
    assert!(out[8].contains(want), "the letter picked: {out:?}");
}

#[test]
fn the_menu_key_is_configurable_and_off_binds_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let hook_with = |cfg: &str| {
        let cfgdir = dir.path().join("cfg");
        std::fs::create_dir_all(&cfgdir).unwrap();
        std::fs::write(cfgdir.join("config.toml"), cfg).unwrap();
        Command::new(env!("CARGO_BIN_EXE_iohr"))
            .args(["shell", "hook", "zsh"])
            .env_clear()
            .env("HOME", dir.path())
            .env("IOHR_CONFIG_DIR", &cfgdir)
            .output()
            .unwrap()
    };
    let o = hook_with("[ask]\nmenu_key = \"ctrl-g\"\n");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let t = String::from_utf8(o.stdout).unwrap();
    assert!(t.contains("_iohr_card_keys=('^G')"), "{t}");
    let t = String::from_utf8(hook_with("[ask]\nmenu_key = \"off\"\n").stdout).unwrap();
    assert!(t.contains("_iohr_card_keys=()"), "{t}");
    let o = hook_with("[ask]\nmenu_key = \"f13\"\n");
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("menu_key"));
}

#[test]
fn bash_and_fish_say_not_built_yet_and_exit_1() {
    for shell in ["bash", "fish"] {
        let o = hook(shell);
        assert_eq!(o.status.code(), Some(1), "{shell}");
        assert_eq!(o.stdout, b"");
        let e = String::from_utf8_lossy(&o.stderr);
        assert!(e.contains("not built yet") && e.contains("zsh"), "{e}");
    }
    assert_eq!(hook("nushell").status.code(), Some(2));
}

// ---------------------------------------------------------------- the card asks back

const ASKS_BACK: &str = "The work is in src.\n```questions\n{\"questions\":[{\"about\":\"purpose\",\"ask\":\"What is this project for?\",\"choices\":[\"A CLI tool\",\"A web service\",\"other\"]}]}\n```\n";

fn git_in(dir: &std::path::Path, args: &[&str]) {
    let ok = Command::new("git")
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .output()
        .unwrap()
        .status
        .success();
    assert!(ok, "git {args:?}");
}

#[test]
fn a_questions_block_is_in_json_and_never_in_the_answer() {
    let r = Rig::new();
    r.fake("claude", FAKE_CLAUDE);
    std::fs::create_dir_all(r.work().join(".inorbit")).unwrap();
    let out = r.run(
        &["--json", "ask", "--headless", "where are we?"],
        &answer_json(ASKS_BACK),
        "0",
    );
    assert!(out.status.success(), "{}", text(&out));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["answer"], "The work is in src.");
    let q = &v["questions"][0];
    assert_eq!(q["about"], "purpose");
    assert_eq!(q["ask"], "What is this project for?");
    // "other" is the card's own key, never one of the model's choices.
    assert_eq!(
        q["choices"],
        serde_json::json!(["A CLI tool", "A web service"])
    );
    assert_eq!(v["questions"].as_array().unwrap().len(), 1);
    // Nothing was kept: only the person's answer is ever written, and iohr writes it.
    assert!(!r.work().join(".inorbit/memory").exists());
}

#[test]
fn without_a_terminal_the_questions_are_plain_lines_and_the_answer_stays_clean() {
    let r = Rig::new();
    r.fake("claude", FAKE_CLAUDE);
    std::fs::create_dir_all(r.work().join(".inorbit")).unwrap();
    let out = r.run(
        &["ask", "--headless", "where are we?"],
        &answer_json(ASKS_BACK),
        "0",
    );
    assert!(out.status.success(), "{}", text(&out));
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "The work is in src.\n"
    );
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("Question for you: What is this project for?")
            && err.contains("[1] A CLI tool"),
        "{err}"
    );
    assert!(!err.contains("```"), "{err}");
}

#[test]
fn the_model_is_told_to_ask_only_inside_a_project_and_a_folder_outside_one_gets_none() {
    let r = Rig::new();
    r.fake("claude", FAKE_CLAUDE);
    // A project: the notes block is there, so the model may ask.
    std::fs::create_dir_all(r.work().join(".inorbit")).unwrap();
    let out = r.run(&["--json", "ask", "--headless", "q"], OK_JSON, "0");
    assert!(out.status.success(), "{}", text(&out));
    let m = r.saw("claude.stdin");
    assert!(m.contains("<project>\nnothing is written down yet"), "{m}");
    // Not a project: no <project> block, so there is nowhere to keep an answer.
    let r = Rig::new();
    r.fake("claude", FAKE_CLAUDE);
    let out = r.run(&["--json", "ask", "--headless", "q"], OK_JSON, "0");
    assert!(out.status.success(), "{}", text(&out));
    assert!(!r.saw("claude.stdin").contains("<project>\nnothing"));
}

#[test]
fn git_facts_come_from_iohr_and_the_model_has_no_way_to_run_git() {
    let r = Rig::new();
    r.fake("claude", FAKE_CLAUDE);
    let w = r.work();
    git_in(&w, &["init", "-q", "-b", "trunk"]);
    std::fs::write(w.join("a.txt"), "x").unwrap();
    git_in(&w, &["add", "a.txt"]);
    git_in(&w, &["commit", "-q", "-m", "first change"]);
    std::fs::write(w.join("b.txt"), "y").unwrap();
    let out = r.run(
        &[
            "--json",
            "ask",
            "--headless",
            "--permissions",
            "deny",
            "where are we?",
        ],
        OK_JSON,
        "0",
    );
    assert!(out.status.success(), "{}", text(&out));
    let m = r.saw("claude.stdin");
    assert!(m.contains("<git>\nbranch: trunk"), "{m}");
    assert!(m.contains("first change"), "{m}");
    assert!(m.contains("1 untracked"), "{m}");
    // The model's tools are the read-only ones, never Bash: git was read by iohr, not by it.
    let args = r.saw("claude.args");
    let lines: Vec<&str> = args.lines().collect();
    let at = lines.iter().position(|l| *l == "--tools").unwrap();
    assert_eq!(lines[at + 1], "Read,Grep,Glob", "{args}");
    // The brief says what to do with the facts.
    assert!(
        args.contains("never say you cannot run git or see the history"),
        "{args}"
    );
    assert!(
        !args.contains("Bash") && !args.contains("--allowedTools"),
        "{args}"
    );
    // The label says it was read.
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(
        v["label"]
            .as_str()
            .unwrap()
            .contains("the git branch, last commits and change counts")
    );
}

#[test]
fn the_menu_offers_asks_only_when_some_wait_and_puts_the_list_on_the_prompt() {
    let r = Rig::new();
    let count = r.home.path().join("asks-count");
    let file = count.to_str().unwrap();
    let cmd = ["ask", "--menu", "--command=git status"];
    // None waiting: the key is not offered, so it prints nothing.
    std::fs::write(&count, "0\n").unwrap();
    let Some((line, _)) = press_in_menu(&r, &cmd, "a", &[("IOHR_ASKS_COUNT_FILE", file)]) else {
        return;
    };
    assert_eq!(line, "");
    // Two waiting: the entry is there, says how many, and fills the prompt with the command.
    std::fs::write(&count, "2\n").unwrap();
    let (line, screen) = press_in_menu(&r, &cmd, "a", &[("IOHR_ASKS_COUNT_FILE", file)]).unwrap();
    assert_eq!(line, "iohr asks list", "{screen:?}");
    assert!(screen.contains("2 waiting"), "{screen:?}");
}

// ------------------------------------------------ ask instead of "I can't" (RFC 0203)

/// The arguments the fake agent got, one per line.
fn sent_args(r: &Rig) -> Vec<String> {
    r.saw("claude.args").lines().map(str::to_owned).collect()
}

fn ask_json(r: &Rig, cwd: &std::path::Path, extra: &[&str], answer: &str) -> serde_json::Value {
    let mut a = vec![
        "--json",
        "ask",
        "--headless",
        "--cwd",
        cwd.to_str().unwrap(),
    ];
    a.extend_from_slice(extra);
    a.push("explain this directory");
    let out = r.run(&a, &answer_json(answer), "0");
    assert!(out.status.success(), "{}", text(&out));
    serde_json::from_slice(&out.stdout).unwrap()
}

// The bug: asked in the home folder, the run got no tools and the answer told the owner to run
// grep themselves. Now Io asks to read the folder: the request is data (no terminal here), the
// run is not given tools it was not allowed, and `--read` is the yes for one run.
#[test]
fn a_folder_too_broad_to_read_is_asked_for_and_a_yes_gives_the_read_tools() {
    let r = Rig::new();
    r.fake("claude", FAKE_CLAUDE);
    let home = r.home.path().canonicalize().unwrap();
    std::fs::write(home.join("README.md"), "hello").unwrap();

    let v = ask_json(&r, &home, &[], "It looks like a home folder.");
    assert_eq!(v["access"]["folder"], home.display().to_string());
    assert_eq!(v["access"]["why"], "home");
    assert_eq!(v["access"]["askable"], true);
    assert_eq!(
        v["access"]["allow_with"],
        format!("--read {}", home.display())
    );
    let args = sent_args(&r);
    let at = args.iter().position(|a| a == "--tools").unwrap();
    assert_eq!(args[at + 1], "", "no tools without the yes: {args:?}");

    let v = ask_json(
        &r,
        &home,
        &["--read", home.to_str().unwrap()],
        "It has a README.",
    );
    assert!(v["access"].is_null(), "{v}");
    let args = sent_args(&r);
    let at = args.iter().position(|a| a == "--tools").unwrap();
    // The read tools (the permission card adds Bash in the default mode).
    assert!(args[at + 1].starts_with("Read,Grep,Glob"), "{args:?}");
    // Secrets stay denied under a yes.
    assert!(
        args.iter()
            .any(|a| a.starts_with("--disallowedTools=Read(")),
        "{args:?}"
    );
}

#[test]
fn a_folder_the_question_names_outside_the_work_folder_is_asked_for_too() {
    let r = Rig::new();
    r.fake("claude", FAKE_CLAUDE);
    let home = r.home.path().canonicalize().unwrap();
    std::fs::create_dir_all(home.join("Downloads")).unwrap();
    let other = home.join("Downloads");
    let work = home.join("work");
    let q = format!("what is in {}", other.display());
    let out = r.run(
        &[
            "--json",
            "ask",
            "--headless",
            "--cwd",
            work.to_str().unwrap(),
            &q,
        ],
        &answer_json("Some files."),
        "0",
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["access"]["why"], "named", "{v}");
    assert_eq!(v["access"]["folder"], other.display().to_string());
    assert!(!sent_args(&r).iter().any(|a| a.starts_with("--add-dir")));
    // The yes adds that one folder, in the `=` form (the flag is variadic).
    let out = r.run(
        &[
            "--json",
            "ask",
            "--headless",
            "--cwd",
            work.to_str().unwrap(),
            "--read",
            other.to_str().unwrap(),
            &q,
        ],
        &answer_json("Some files."),
        "0",
    );
    assert!(out.status.success(), "{}", text(&out));
    assert!(sent_args(&r).contains(&format!("--add-dir={}", other.display())));
    // A folder that does not exist, or a secret place, is a usage error, never a yes.
    std::fs::create_dir_all(home.join(".ssh")).unwrap();
    for bad in ["nope", ".ssh"] {
        let out = r.run(
            &[
                "--json",
                "ask",
                "--headless",
                "--cwd",
                home.to_str().unwrap(),
                "--read",
                bad,
                "q",
            ],
            OK_JSON,
            "0",
        );
        assert!(!out.status.success(), "{bad}");
    }
}

// More folders to read, from every source (RFC 0203): `--read` and its alias `--add-dir` repeat,
// `IOHR_ASK_READ` is a path list and `[ask] read` a config list. Each folder is one `--add-dir=`
// to the agent, and a secret place from any source is a usage error.
#[test]
fn folders_to_read_come_from_flags_the_env_and_the_config_and_secrets_stay_refused() {
    let r = Rig::new();
    r.fake("claude", FAKE_CLAUDE);
    let home = r.home.path().canonicalize().unwrap();
    let work = home.join("work");
    let dirs: Vec<_> = ["one", "two", "three", "four"]
        .iter()
        .map(|n| {
            let d = home.join(n);
            std::fs::create_dir_all(&d).unwrap();
            d
        })
        .collect();
    let added = |r: &Rig| -> Vec<String> {
        sent_args(r)
            .into_iter()
            .filter(|a| a.starts_with("--add-dir="))
            .collect()
    };
    let want = |ds: &[&std::path::PathBuf]| -> Vec<String> {
        ds.iter()
            .map(|d| format!("--add-dir={}", d.display()))
            .collect()
    };
    let cwd = work.to_str().unwrap();
    let ask = |r: &Rig, extra: &[&str], env: &[(&str, &str)]| {
        let mut a = vec!["--json", "ask", "--headless", "--cwd", cwd];
        a.extend_from_slice(extra);
        a.push("what is in these");
        r.run_env(&a, &answer_json("Files."), "0", env)
    };

    // Flags: `--read` and `--add-dir` repeat and mix.
    let out = ask(
        &r,
        &[
            "--read",
            dirs[0].to_str().unwrap(),
            "--add-dir",
            dirs[1].to_str().unwrap(),
        ],
        &[],
    );
    assert!(out.status.success(), "{}", text(&out));
    assert_eq!(added(&r), want(&[&dirs[0], &dirs[1]]));

    // Env: a path list.
    let list = std::env::join_paths([&dirs[2], &dirs[3]]).unwrap();
    let out = ask(&r, &[], &[("IOHR_ASK_READ", list.to_str().unwrap())]);
    assert!(out.status.success(), "{}", text(&out));
    assert_eq!(added(&r), want(&[&dirs[2], &dirs[3]]));

    // Config: `[ask] read`, a `~/` is the home folder.
    std::fs::create_dir_all(home.join("config")).unwrap();
    std::fs::write(
        home.join("config").join("config.toml"),
        "[ask]\nread = [\"~/one\", \"~/two\"]\n",
    )
    .unwrap();
    let out = ask(&r, &[], &[]);
    assert!(out.status.success(), "{}", text(&out));
    assert_eq!(added(&r), want(&[&dirs[0], &dirs[1]]));

    // All three add up, none twice.
    let out = ask(
        &r,
        &["--read", dirs[0].to_str().unwrap()],
        &[("IOHR_ASK_READ", dirs[2].to_str().unwrap())],
    );
    assert!(out.status.success(), "{}", text(&out));
    assert_eq!(added(&r), want(&[&dirs[0], &dirs[2], &dirs[1]]));
    assert!(
        sent_args(&r)
            .iter()
            .any(|a| a.starts_with("--disallowedTools=Read(")),
        "secrets stay denied beside every added folder"
    );

    // A secret place from any source is refused and the agent is not started with it.
    std::fs::create_dir_all(home.join(".ssh")).unwrap();
    std::fs::write(r.out().join("claude.args"), "").unwrap();
    let secret = home.join(".ssh");
    let out = ask(&r, &["--add-dir", secret.to_str().unwrap()], &[]);
    assert!(!out.status.success(), "flag: {}", text(&out));
    let out = ask(&r, &[], &[("IOHR_ASK_READ", secret.to_str().unwrap())]);
    assert!(!out.status.success(), "env: {}", text(&out));
    std::fs::write(
        home.join("config").join("config.toml"),
        "[ask]\nread = [\"~/.ssh\"]\n",
    )
    .unwrap();
    let out = ask(&r, &[], &[]);
    assert!(!out.status.success(), "config: {}", text(&out));
    assert!(sent_args(&r).is_empty(), "the agent never started");
}

// The answer's next steps are actions in the bar, never a shell line to copy: in the JSON, and
// as the sixth field of the hook's record, and the block is not shown in the answer.
#[test]
fn next_steps_come_as_actions_and_the_block_is_not_shown() {
    let r = Rig::new();
    r.fake("claude", FAKE_CLAUDE);
    std::fs::create_dir_all(r.work().join("api")).unwrap();
    let answer = "A service.\n\n```next\nread api/\nsearch TODO\nread ../\n```\n";
    let v = ask_json(&r, &r.work(), &[], answer);
    assert_eq!(
        v["next"],
        serde_json::json!([
            {"kind": "read", "value": "api"},
            {"kind": "search", "value": "TODO"}
        ])
    );
    assert_eq!(v["answer"], "A service.");

    let out = r.run(
        &[
            "ask",
            "--headless",
            "--hook",
            "--cwd",
            r.work().to_str().unwrap(),
            "q",
        ],
        &answer_json(answer),
        "0",
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    let f: Vec<&str> = stdout.split('\u{1e}').collect();
    assert_eq!(f.len(), 6, "{stdout:?}");
    assert_eq!(f[5], "read:api\u{1f}search:TODO");
    assert!(!String::from_utf8_lossy(&out.stderr).contains("```next"));
}

// The system prompt: with no access Io says what it would read and offers it as an action; it
// never tells the person to run grep, ls, cat or find.
#[test]
fn the_prompt_offers_the_reading_and_never_tells_the_person_to_run_grep() {
    let r = Rig::new();
    r.fake("claude", FAKE_CLAUDE);
    let home = r.home.path().canonicalize().unwrap();
    ask_json(&r, &home, &[], "x");
    let sent = format!("{}{}", r.saw("claude.args"), r.saw("claude.stdin"));
    assert!(
        sent.contains("Io offers the reading as an action"),
        "{sent}"
    );
    assert!(sent.contains("```next"));
    assert!(sent.contains("never tell the person to run grep, ls, cat or find"));
    assert!(!sent.contains("run grep -rn"));
}

// ---------------------------------------------------------------- what the question can see
// RFC 0203, PRD 0187.1: the card says what was sent, and the output of the last command is
// sent only when the person attached it (a file, a re-run they said yes to, or a paste).

const FAILED: [&str; 7] = [
    "ask",
    "--cwd",
    "/work/api",
    "--last-command",
    "cargo test",
    "--exit-code",
    "101",
];

fn with<'a>(base: &[&'a str], more: &[&'a str]) -> Vec<&'a str> {
    base.iter().chain(more).copied().collect()
}

#[test]
fn the_card_says_the_output_was_not_sent_and_json_carries_the_same_facts() {
    let r = Rig::new();
    r.fake("claude", FAKE_CLAUDE);
    let json = |args: &[&str]| -> serde_json::Value {
        let mut a = vec!["--json"];
        a.extend_from_slice(args);
        let out = r.run(&a, OK_JSON, "0");
        assert!(out.status.success(), "{}", text(&out));
        serde_json::from_slice(&out.stdout).unwrap()
    };
    // A failed command and no output: the footer and the data both say it was not sent.
    let v = json(&with(&FAILED, &["--headless", "--", "why did that fail"]));
    let label = v["label"].as_str().unwrap();
    assert!(
        label.contains("its exit status (output not sent)"),
        "{label}"
    );
    assert_eq!(
        v["context"],
        serde_json::json!({
            "folder": "/work/api", "folders": [], "command": true, "exit_code": 101,
            "output": "not_sent", "output_bytes": 0, "can_attach": true,
            "io_md": {"files": 0, "bytes": 0, "cut": 0, "not_read": 0}
        })
    );
    // A question that does not ask why is not offered the attach.
    let v = json(&with(&FAILED, &["--headless", "--", "list the files"]));
    assert_eq!(v["context"]["can_attach"], false);
    // No command at all: nothing to say about output.
    let v = json(&["ask", "--headless", "--cwd", "/work/api", "--", "hello"]);
    assert_eq!(v["context"]["command"], false);
    assert_eq!(v["context"]["exit_code"], serde_json::Value::Null);
    assert!(!v["label"].as_str().unwrap().contains("output not sent"));
    // With output from a file, secrets are redacted and the footer says where it came from.
    let f = r.home.path().join("out.txt");
    std::fs::write(
        &f,
        "error: failed\npassword=hunter2\nAuthorization: Bearer abc.def.ghi\nAPI_KEY=sk-live-0123456789\n",
    )
    .unwrap();
    let file = f.to_str().unwrap();
    let v = json(&with(
        &FAILED,
        &[
            "--output-file",
            file,
            "--headless",
            "--",
            "why did that fail",
        ],
    ));
    assert_eq!(v["context"]["output"], "attached");
    assert_eq!(v["context"]["can_attach"], false);
    assert!(v["context"]["output_bytes"].as_u64().unwrap() > 0);
    let label = v["label"].as_str().unwrap();
    assert!(
        label.contains("from a file you gave, secrets redacted") && !label.contains("not sent"),
        "{label}"
    );
    let sent = r.saw("claude.stdin");
    assert!(sent.contains("error: failed"), "{sent}");
    for leaked in ["hunter2", "abc.def.ghi", "sk-live-0123456789"] {
        assert!(!sent.contains(leaked), "{leaked} reached the model: {sent}");
    }
    // The kinds a re-run and a paste carry say so in the footer and in the data.
    for (kind, said) in [
        ("captured", "from a re-run you allowed"),
        ("pasted", "as you pasted it"),
    ] {
        let v = json(&with(
            &FAILED,
            &[
                "--output-file",
                file,
                "--output-kind",
                kind,
                "--headless",
                "--",
                "why",
            ],
        ));
        assert_eq!(v["context"]["output"], kind);
        assert!(v["label"].as_str().unwrap().contains(said));
    }
    // The kind belongs to a file.
    let out = r.run(
        &with(&FAILED, &["--output-kind", "pasted", "--", "q"]),
        OK_JSON,
        "0",
    );
    assert_eq!(out.status.code(), Some(2), "{}", text(&out));
}

#[test]
fn off_a_terminal_the_plain_card_says_how_to_attach_the_output_and_never_draws_a_row() {
    let r = Rig::new();
    r.fake("claude", FAKE_CLAUDE);
    for env in [&[][..], &[("NO_COLOR", "1")][..], &[("TERM", "dumb")][..]] {
        let out = r.run_env(
            &with(&FAILED, &["--headless", "--", "why did that fail"]),
            OK_JSON,
            "0",
            env,
        );
        assert!(out.status.success(), "{}", text(&out));
        let t = text(&out);
        assert!(t.contains("Its output was not sent."), "{t}");
        assert!(t.contains("--output-file FILE"), "{t}");
        assert!(!t.contains('\u{1b}'), "plain has no escape codes: {t:?}");
        assert!(!t.contains("Re-run with output"), "{t}");
    }
    // A question that does not ask why says nothing of it; nor does one that has the output.
    let out = r.run(
        &with(&FAILED, &["--headless", "--", "list the files"]),
        OK_JSON,
        "0",
    );
    assert!(!text(&out).contains("Its output was not sent."));
}

#[test]
fn the_menu_offers_the_output_only_for_a_failed_command_and_a_why_question() {
    let r = Rig::new();
    let to = r.home.path().join("attached");
    let to_s = to.to_str().unwrap();
    let base = |asked: &'static str, exit: &'static str| {
        vec![
            "ask",
            "--menu",
            "--command=git status",
            "--asked",
            asked,
            "--failed-command",
            "false",
            "--failed-exit",
            exit,
            "--output-to",
        ]
    };
    let mut a = base("why did that fail", "1");
    a.push(to_s);
    let Some((_, screen)) = press_in_menu(&r, &a, "\u{1b}", &[]) else {
        return;
    };
    assert!(
        screen.contains("Re-run with output") && screen.contains("Paste output"),
        "{screen:?}"
    );
    // A succeeded command, or a question that does not ask why: neither is offered.
    for (asked, exit) in [("why did that fail", "0"), ("list the files", "1")] {
        let mut a = base(asked, exit);
        a.push(to_s);
        let (line, screen) = press_in_menu(&r, &a, "w", &[]).unwrap();
        assert_eq!(line, "");
        assert!(!screen.contains("Re-run with output"), "{screen:?}");
    }
    assert!(!to.exists() || std::fs::read_to_string(&to).unwrap().is_empty());
}

#[test]
fn a_rerun_asks_first_runs_once_and_keeps_only_the_scrubbed_end_of_its_output() {
    let r = Rig::new();
    let to = r.home.path().join("attached");
    let to_s = to.to_str().unwrap();
    let ran = r.home.path().join("ran");
    let command = format!(
        "echo run >> {ran}; echo boom-out; echo password=hunter2 >&2; exit 3",
        ran = ran.display()
    );
    let failed = format!("--failed-command={command}");
    let a = [
        "ask",
        "--menu",
        "--asked",
        "why did that fail",
        &failed,
        "--failed-exit",
        "3",
        "--output-to",
        to_s,
    ];
    let Some((line, screen)) = press_in_menu(&r, &a, "w\u{1f}n", &[]) else {
        return;
    };
    // The command is shown whole, and nothing ran on a no.
    assert!(screen.contains("It would run again"), "{screen:?}");
    assert!(screen.contains("boom-out"), "{screen:?}");
    assert_eq!(line, "", "{screen:?}");
    assert!(!ran.exists(), "a no runs nothing");
    // Esc on the question is a no too.
    let (line, _) = press_in_menu(&r, &a, "w\u{1f}\u{1b}", &[]).unwrap();
    assert_eq!(line, "");
    assert!(!ran.exists(), "Esc runs nothing");
    // A yes runs it once; the line is the question again, the kind follows a 0x1e.
    let (line, screen) = press_in_menu(&r, &a, "w\u{1f}y", &[]).unwrap();
    assert_eq!(line, "? why did that fail\u{1e}captured", "{screen:?}");
    assert_eq!(std::fs::read_to_string(&ran).unwrap().lines().count(), 1);
    let kept = std::fs::read_to_string(&to).unwrap();
    assert!(kept.contains("boom-out"), "{kept}");
    assert!(
        !kept.contains("hunter2"),
        "secrets never reach the file: {kept}"
    );
    assert!(screen.contains("lines of output kept"), "{screen:?}");
}

#[test]
fn a_rerun_is_bounded_in_time_and_in_size() {
    let r = Rig::new();
    let to = r.home.path().join("attached");
    let a = [
        "ask",
        "--menu",
        "--asked",
        "why did that fail",
        "--failed-command=yes line-of-noise",
        "--failed-exit",
        "1",
        "--output-to",
        to.to_str().unwrap(),
    ];
    // The re-run is stopped after 10 s; resizes are harmless steps that give it the time.
    let waits = "\u{1f}WINCH:x".repeat(24);
    let Some((line, screen)) = press_in_menu(&r, &a, &format!("w\u{1f}y{waits}"), &[]) else {
        return;
    };
    assert_eq!(line, "? why did that fail\u{1e}captured", "{screen:?}");
    assert!(screen.contains("stopped after"), "{screen:?}");
    assert!(std::fs::metadata(&to).unwrap().len() <= 16 * 1024);
}

#[test]
fn a_paste_is_kept_scrubbed_and_nothing_is_run() {
    let r = Rig::new();
    let to = r.home.path().join("attached");
    let ran = r.home.path().join("ran");
    let failed = format!("--failed-command=touch {}", ran.display());
    let a = [
        "ask",
        "--menu",
        "--asked",
        "why did that fail",
        &failed,
        "--failed-exit",
        "1",
        "--output-to",
        to.to_str().unwrap(),
    ];
    let Some((line, screen)) = press_in_menu(
        &r,
        &a,
        "p\u{1f}real error: no such file\ntoken=abc123\n\u{1f}\u{4}",
        &[],
    ) else {
        return;
    };
    assert_eq!(line, "? why did that fail\u{1e}pasted", "{screen:?}");
    let kept = std::fs::read_to_string(&to).unwrap();
    assert!(kept.contains("real error: no such file"), "{kept}");
    assert!(!kept.contains("abc123"), "{kept}");
    assert!(!ran.exists(), "a paste runs nothing");
}

// The hook end to end on a real pty: a failed command, a why question, the menu's re-run, and
// the next ask carries the output file and its kind; the file is removed after it is read; and
// the output never goes with another command or another ask.
#[test]
fn the_hook_carries_attached_output_to_the_next_ask_only() {
    if !have("zsh") || !have("python3") {
        return;
    }
    let z = Zsh::new();
    let (dir, file, bin) = (&z.dir, &z.file, &z.bin);
    let log = dir.path().join("calls.log");
    let seen = dir.path().join("seen.txt");
    // the kind. Otherwise (an ask): logs its arguments and what the output file held.
    z.fake(format!(
        r#"#!/bin/sh
echo "$*" >> {log}
case "$*" in
*--menu*)
  while [ $# -gt 0 ]; do [ "$1" = --output-to ] && to=$2; shift; done
  printf 'captured: the real error
' > "$to"
  printf '? why did that failcaptured';;
*)
  while [ $# -gt 0 ]; do [ "$1" = --output-file ] && cat "$2" >> {seen}; shift; done
  printf 'the answerecho hinotes.md';;
esac
"#,
        log = log.display(),
        seen = seen.display()
    ));
    let down = "RAW:\u{1b}[B";
    drive_zsh(
        file,
        bin,
        &[
            format!("source {}", file.display()),
            "false".to_owned(),
            "? why did that fail".to_owned(),
            down.to_owned(),
            // The line is the question again; Enter asks it with the output.
            "RAW:\r".to_owned(),
            // Another ask: no output goes with it.
            "? and again, why did that fail".to_owned(),
        ],
    );
    let calls = std::fs::read_to_string(&log).unwrap_or_default();
    // The asks segment's own refresh (`asks count`) is not one of the calls under test.
    let lines: Vec<&str> = calls.lines().filter(|l| !l.starts_with("asks ")).collect();
    assert_eq!(lines.len(), 4, "{calls}");
    assert!(
        !lines[0].contains("--output-file"),
        "the first ask has none: {calls}"
    );
    assert!(
        lines[1].contains("--menu")
            && lines[1].contains("--failed-command false")
            && lines[1].contains("--failed-exit 1")
            && lines[1].contains("--asked=why did that fail")
            && lines[1].contains("--output-to "),
        "{calls}"
    );
    assert!(
        lines[2].contains("--output-file ") && lines[2].contains("--output-kind captured"),
        "{calls}"
    );
    assert!(!lines[3].contains("--output-file"), "{calls}");
    assert_eq!(
        std::fs::read_to_string(&seen).unwrap(),
        "captured: the real error\n"
    );
    // The private file is gone once the ask has read it.
    let to = lines[2]
        .split("--output-file ")
        .nth(1)
        .and_then(|r| r.split(' ').next())
        .unwrap();
    assert!(!std::path::Path::new(to).exists(), "{to} was left behind");
}

#[test]
fn the_hook_sends_no_output_for_another_command_and_drops_a_menu_that_attached_nothing() {
    if Command::new("zsh").arg("--version").output().is_err() {
        return;
    }
    let z = Zsh::new();
    let (dir, file, bin) = (&z.dir, &z.file, &z.bin);
    z.fake("#!/bin/sh\necho \"ARGS: $*\" >&2\nprintf 'a\\036\\036'\n");
    let stale = dir.path().join("stale");
    std::fs::write(&stale, "old output").unwrap();
    let o = Command::new("zsh")
        .args([
            "-f",
            "-i",
            "-c",
            &format!(
                "source {f}; print -s 'cargo build'; _iohr_ask_failcmd='cargo test'; \
                 _iohr_ask_outfile={stale}; _iohr_ask_outkind=captured; \
                 _iohr_ask_q='? why did that fail'; _iohr_ask_precmd",
                f = file.display(),
                stale = stale.display()
            ),
        ])
        .current_dir(dir.path())
        .env_clear()
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .env("HOME", dir.path())
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(err.contains("ARGS:"), "{err}");
    assert!(!err.contains("--output-file"), "another command: {err}");
    assert!(!stale.exists(), "the file is removed either way");
}
