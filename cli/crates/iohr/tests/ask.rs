//! `iohr ask` with the local backend (PRD 0088.1, slice ask-local; the headless
//! answer is tested in `shell_ask.rs`): the binary with a private
//! HOME, config folder and PATH holding a fake agent, so no real agent ever starts and nothing
//! of the machine running the test is read.

#![allow(
    clippy::unwrap_used,
    reason = "test helpers fail the test by panicking"
)]
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// A fake agent: records its working folder and every argument (one per line) in
/// `$FAKE_OUT/<name>.*`, prints the recorded stream in `$FAKE_STREAM` (when set) on stdout (shell builtins only: PATH holds nothing else),
/// then exits with `$FAKE_EXIT`.
fn fake_agent(bin: &Path, name: &str) {
    std::fs::create_dir_all(bin).unwrap();
    let path = bin.join(name);
    std::fs::write(
        &path,
        format!(
            "#!/bin/sh\npwd > \"$FAKE_OUT/{name}.cwd\"\nprintf '%s\\n' \"$@\" > \"$FAKE_OUT/{name}.args\"\nif [ -n \"$FAKE_STREAM\" ]; then while IFS= read -r l; do printf '%s\\n' \"$l\"; done < \"$FAKE_STREAM\"; fi\nexit \"${{FAKE_EXIT:-0}}\"\n"
        ),
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn data(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(name)
}

struct Rig {
    home: tempfile::TempDir,
}

impl Rig {
    fn new() -> Self {
        let r = Self {
            home: tempfile::tempdir().unwrap(),
        };
        std::fs::create_dir_all(r.out()).unwrap();
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
    fn config(&self, text: &str) {
        let dir = self.home.path().join("config");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("config.toml"), text).unwrap();
    }
    /// Runs `iohr` in the work folder with only the fake bin folder on PATH.
    fn run(&self, args: &[&str], exit: &str) -> Output {
        self.run_with(args, exit, None)
    }
    /// The same, the fake agent printing the recorded stream `stream`.
    fn run_with(&self, args: &[&str], exit: &str, stream: Option<&str>) -> Output {
        Command::new(env!("CARGO_BIN_EXE_iohr"))
            .args(args)
            .current_dir(self.work())
            .env_clear()
            .env("PATH", self.bin())
            .env("IOHR_NO_BROWSER", "1")
            .env("HOME", self.home.path())
            .env("USERPROFILE", self.home.path())
            .env("FAKE_STREAM", stream.map(data).unwrap_or_default())
            .env("IOHR_CONFIG_DIR", self.home.path().join("config"))
            .env("FAKE_OUT", self.out())
            .env("FAKE_EXIT", exit)
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }
    fn saw(&self, file: &str) -> Option<String> {
        std::fs::read_to_string(self.out().join(file)).ok()
    }
    /// The arguments the fake agent received.
    fn args(&self, name: &str) -> Vec<String> {
        self.saw(&format!("{name}.args"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }
    /// The question the fake agent received: its last argument.
    fn question(&self, name: &str) -> Option<String> {
        self.args(name).pop()
    }
}

fn text(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

fn has_pair(args: &[String], a: &str, b: &str) -> bool {
    args.windows(2).any(|w| w[0] == a && w[1] == b)
}

#[test]
fn the_agent_gets_the_question_in_this_folder_and_the_default_is_claude() {
    let r = Rig::new();
    fake_agent(&r.bin(), "claude");
    fake_agent(&r.bin(), "codex");
    let out = r.run(&["ask", "generate me a README in this folder"], "0");
    assert!(out.status.success(), "{}", text(&out));
    assert_eq!(
        r.question("claude").as_deref(),
        Some("generate me a README in this folder")
    );
    let cwd = PathBuf::from(r.saw("claude.cwd").unwrap().trim());
    assert_eq!(
        cwd.canonicalize().unwrap(),
        r.work().canonicalize().unwrap()
    );
    assert!(r.saw("codex.args").is_none(), "codex must not start");
}

#[test]
fn claude_runs_once_in_print_mode_with_the_small_model_and_no_session() {
    let r = Rig::new();
    fake_agent(&r.bin(), "claude");
    let out = r.run(&["ask", "q"], "0");
    assert!(out.status.success(), "{}", text(&out));
    let a = r.args("claude");
    // The exact list, so a changed flag shows up here.
    assert_eq!(
        a,
        [
            "-p",
            "--output-format",
            "stream-json",
            "--verbose",
            "--include-partial-messages",
            "--model",
            "haiku",
            "--no-session-persistence",
            "--",
            "q"
        ]
    );
    // Edits are not allowed unless asked for, and nothing allows Bash.
    assert!(
        !a.iter()
            .any(|x| x.contains("permission") || x.contains("Bash"))
    );
}

#[test]
fn the_model_comes_from_the_flag_then_the_config_then_haiku() {
    let r = Rig::new();
    fake_agent(&r.bin(), "claude");
    r.config("[ask]\nmodel = \"sonnet\"\n");
    assert!(r.run(&["ask", "q"], "0").status.success());
    assert!(has_pair(&r.args("claude"), "--model", "sonnet"));
    assert!(
        r.run(&["ask", "--model", "opus", "q"], "0")
            .status
            .success()
    );
    assert!(has_pair(&r.args("claude"), "--model", "opus"));
    // A model that would read as a flag is a usage error and starts nothing.
    let bad = r.run(&["ask", "--model=--dangerously-skip-permissions", "q"], "0");
    assert_eq!(bad.status.code(), Some(2), "{}", text(&bad));
}

#[test]
fn edits_are_allowed_only_by_the_flag_or_the_config() {
    let r = Rig::new();
    fake_agent(&r.bin(), "claude");
    assert!(r.run(&["ask", "--allow-edits", "q"], "0").status.success());
    assert!(has_pair(
        &r.args("claude"),
        "--permission-mode",
        "acceptEdits"
    ));
    r.config("[ask]\nallow_edits = true\n");
    assert!(r.run(&["ask", "q"], "0").status.success());
    assert!(has_pair(
        &r.args("claude"),
        "--permission-mode",
        "acceptEdits"
    ));
}

#[test]
fn words_without_quotes_are_one_question() {
    let r = Rig::new();
    fake_agent(&r.bin(), "claude");
    let out = r.run(&["ask", "add", "a", "license"], "0");
    assert!(out.status.success(), "{}", text(&out));
    assert_eq!(r.question("claude").as_deref(), Some("add a license"));
}

#[test]
fn a_question_that_looks_like_a_flag_stays_a_question() {
    let r = Rig::new();
    fake_agent(&r.bin(), "claude");
    let out = r.run(&["ask", "--", "--dangerously-skip-permissions"], "0");
    assert!(out.status.success(), "{}", text(&out));
    let a = r.args("claude");
    assert_eq!(a[a.len() - 2..], ["--", "--dangerously-skip-permissions"]);
}

#[test]
fn the_agents_exit_code_is_passed_through() {
    let r = Rig::new();
    fake_agent(&r.bin(), "claude");
    let out = r.run(&["ask", "q"], "7");
    assert_eq!(out.status.code(), Some(7), "{}", text(&out));
}

#[test]
fn plain_prints_only_the_answer_and_the_ai_label_is_one_line_on_stderr() {
    let r = Rig::new();
    fake_agent(&r.bin(), "claude");
    let out = r.run_with(
        &["ask", "the-secret-question"],
        "0",
        Some("ask-claude.jsonl"),
    );
    assert!(out.status.success(), "{}", text(&out));
    // The recorded run thought, used tools and answered in pieces: only the answer is printed.
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "I couldn't read `hello.txt`: it doesn't exist in the working directory (`.../scratchpad/scratch`), and that directory is empty. I didn't run a filesystem-wide search because it needs approval here. If the file is somewhere else, give me the path and I'll read it.\n\nFive words: **Hello, this is Claude speaking.**\n"
    );
    let err = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_eq!(err.lines().count(), 1, "{err}");
    assert!(
        err.contains("Io -- Io running on Claude Code (local)"),
        "{err}"
    );
    assert!(err.contains("your own subscription"), "{err}");
    assert!(err.contains("folder goes to that provider"), "{err}");
    assert!(
        !text(&out).contains("the-secret-question"),
        "{}",
        text(&out)
    );
}

#[test]
fn a_dumb_terminal_or_no_color_is_plain_too() {
    // The test has no terminal on stderr, so this is plain already; the env rules are the
    // unit tests of `ui::fancy_for`. Setting them must not change the output.
    let r = Rig::new();
    fake_agent(&r.bin(), "claude");
    let plain = r.run_with(&["ask", "q"], "0", Some("ask-claude.jsonl"));
    let again = Command::new(env!("CARGO_BIN_EXE_iohr"))
        .args(["ask", "q"])
        .current_dir(r.work())
        .env_clear()
        .env("PATH", r.bin())
        .env("HOME", r.home.path())
        .env("USERPROFILE", r.home.path())
        .env("IOHR_CONFIG_DIR", r.home.path().join("config"))
        .env("FAKE_OUT", r.out())
        .env("FAKE_STREAM", data("ask-claude.jsonl"))
        .env("NO_COLOR", "1")
        .env("TERM", "dumb")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(plain.stdout, again.stdout);
}

#[test]
fn json_passes_the_agents_events_through_as_lines() {
    let r = Rig::new();
    fake_agent(&r.bin(), "claude");
    let out = r.run_with(
        &["--json", "ask", "the-secret-question"],
        "3",
        Some("ask-claude.jsonl"),
    );
    assert_eq!(out.status.code(), Some(3));
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let recorded = std::fs::read_to_string(data("ask-claude.jsonl")).unwrap();
    let lines: Vec<&str> = stdout.lines().collect();
    // Every recorded line, untouched and in order, then one summary line.
    let (summary, events) = lines.split_last().unwrap();
    assert_eq!(events, recorded.lines().collect::<Vec<_>>());
    let v: serde_json::Value = serde_json::from_str(summary).unwrap();
    let mut keys: Vec<_> = v.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    assert_eq!(keys, ["agent", "duration_ms", "exit_code", "type", "via"]);
    assert_eq!(v["type"], "iohr_summary");
    assert_eq!(v["exit_code"], 3);
    assert!(!text(&out).contains("the-secret-question"));
}

#[test]
fn an_agent_that_reports_a_failed_run_fails_the_command() {
    let r = Rig::new();
    fake_agent(&r.bin(), "claude");
    let out = r.run_with(&["ask", "q"], "0", Some("ask-claude-failed.jsonl"));
    assert_eq!(out.status.code(), Some(1), "{}", text(&out));
    assert!(text(&out).contains("Not logged in"), "{}", text(&out));
    assert!(out.stdout.is_empty(), "{}", text(&out));
}

#[test]
fn a_missing_agent_names_the_install_step_and_starts_nothing() {
    let r = Rig::new();
    std::fs::create_dir_all(r.bin()).unwrap();
    let out = r.run(&["ask", "the-secret-question"], "0");
    assert!(!out.status.success());
    let t = text(&out);
    assert!(t.contains("`claude` is not on your PATH"), "{t}");
    assert!(
        t.contains("npm install -g @anthropic-ai/claude-code"),
        "{t}"
    );
    assert!(!t.contains("the-secret-question"), "{t}");
}

// PRD 0197: `--mode` and `[ask] mode` reach the agent. Plan is Claude's plan mode with the
// shell and the secret reads closed, and Codex's read-only sandbox (it has no plan mode) even
// beside `--allow-edits`; auto is what `--allow-edits` was on this path (no broker, no shell).
#[test]
fn the_mode_reaches_claude_and_codex_and_plan_beats_allow_edits() {
    let r = Rig::new();
    fake_agent(&r.bin(), "claude");
    fake_agent(&r.bin(), "codex");
    assert!(
        r.run(&["ask", "--mode", "plan", "--allow-edits", "q"], "0")
            .status
            .success()
    );
    let a = r.args("claude");
    assert!(has_pair(&a, "--permission-mode", "plan"), "{a:?}");
    assert!(!a.iter().any(|x| x == "acceptEdits"), "{a:?}");
    assert!(a.iter().any(|x| x == "--disallowedTools=Bash"), "{a:?}");
    assert!(r.run(&["ask", "--mode", "auto", "q"], "0").status.success());
    assert!(has_pair(
        &r.args("claude"),
        "--permission-mode",
        "acceptEdits"
    ));
    // Manual is the default: nothing added.
    assert!(
        r.run(&["ask", "--mode", "manual", "q"], "0")
            .status
            .success()
    );
    assert!(!r.args("claude").iter().any(|x| x == "--permission-mode"));
    // The config says it when no flag does; the flag beats the config.
    r.config("[ask]\nmode = \"plan\"\n");
    assert!(r.run(&["ask", "q"], "0").status.success());
    assert!(has_pair(&r.args("claude"), "--permission-mode", "plan"));
    assert!(
        r.run(&["ask", "--mode", "manual", "q"], "0")
            .status
            .success()
    );
    assert!(!r.args("claude").iter().any(|x| x == "--permission-mode"));
    // Codex has no plan mode: the read-only sandbox, also beside `--allow-edits`.
    let out = r.run(
        &[
            "ask",
            "--agent",
            "codex",
            "--mode",
            "plan",
            "--allow-edits",
            "q",
        ],
        "0",
    );
    assert!(out.status.success(), "{}", text(&out));
    assert!(has_pair(&r.args("codex"), "--sandbox", "read-only"));
}

#[test]
fn a_bad_mode_is_a_usage_error_that_names_the_three() {
    let r = Rig::new();
    fake_agent(&r.bin(), "claude");
    let out = r.run(&["ask", "--mode", "wild", "q"], "0");
    assert_eq!(out.status.code(), Some(2), "{}", text(&out));
    assert!(
        text(&out).contains("manual, plan or auto"),
        "{}",
        text(&out)
    );
    assert!(r.saw("claude.args").is_none(), "nothing may start");
    let out = r.run(&["ask", "--dry-run", "--mode", "plan", "q"], "0");
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("--permission-mode plan"),
        "{}",
        text(&out)
    );
}

#[test]
fn codex_runs_exec_json_read_only_and_its_events_draw_like_claudes() {
    let r = Rig::new();
    fake_agent(&r.bin(), "codex");
    let out = r.run_with(
        &["ask", "--agent", "codex", "hi there"],
        "0",
        Some("ask-codex.jsonl"),
    );
    assert!(out.status.success(), "{}", text(&out));
    assert_eq!(
        r.args("codex"),
        [
            "exec",
            "--json",
            "--ephemeral",
            "--skip-git-repo-check",
            "--sandbox",
            "read-only",
            "--",
            "hi there"
        ]
    );
    // The same renderer, plain: just the answer.
    assert_eq!(String::from_utf8_lossy(&out.stdout), "Hello from codex.\n");
    assert!(text(&out).contains("Codex (local)"), "{}", text(&out));
    // `--model` and `--allow-edits` reach codex too, only when given.
    assert!(
        r.run(
            &[
                "ask",
                "--agent",
                "codex",
                "--model",
                "gpt-x",
                "--allow-edits",
                "q"
            ],
            "0"
        )
        .status
        .success()
    );
    let a = r.args("codex");
    assert!(has_pair(&a, "--model", "gpt-x"));
    assert!(has_pair(&a, "--sandbox", "workspace-write"));
}

#[test]
fn the_codex_config_is_honoured_and_json_passes_its_events_through() {
    let r = Rig::new();
    fake_agent(&r.bin(), "claude");
    fake_agent(&r.bin(), "codex");
    r.config("[ask]\nagent = \"codex\"\n");
    let out = r.run_with(&["--json", "ask", "one"], "0", Some("ask-codex.jsonl"));
    assert!(out.status.success(), "{}", text(&out));
    assert!(r.saw("claude.args").is_none());
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(stdout.contains(r#""type":"thread.started""#), "{stdout}");
    // The flag wins over the config for one call.
    assert!(
        r.run(&["ask", "--agent", "claude", "two"], "0")
            .status
            .success()
    );
    assert_eq!(r.question("claude").as_deref(), Some("two"));
    std::fs::remove_file(r.home.path().join("config/config.toml")).unwrap();
    assert!(r.run(&["ask", "three"], "0").status.success());
    assert_eq!(r.question("claude").as_deref(), Some("three"));
}

#[test]
fn dry_run_prints_the_exact_command_line_and_starts_nothing() {
    let r = Rig::new();
    fake_agent(&r.bin(), "claude");
    fake_agent(&r.bin(), "codex");
    let out = r.run(&["ask", "--dry-run", "what is", "this?"], "0");
    assert!(out.status.success(), "{}", text(&out));
    let line = String::from_utf8_lossy(&out.stdout).into_owned();
    let bin = r.bin().join("claude");
    assert_eq!(
        line.trim_end(),
        format!(
            "{} -p --output-format stream-json --verbose --include-partial-messages --model haiku --no-session-persistence -- 'what is this?'",
            bin.display()
        )
    );
    assert!(r.saw("claude.args").is_none(), "nothing may start");
    // As data, for a program; it needs no agent on the PATH either.
    let j = r.run(
        &[
            "--json",
            "ask",
            "--dry-run",
            "--agent",
            "codex",
            "--allow-edits",
            "q",
        ],
        "0",
    );
    let v: serde_json::Value = serde_json::from_slice(&j.stdout).unwrap();
    assert_eq!(v["dry_run"], true);
    assert_eq!(v["agent"], "codex");
    assert_eq!(v["args"][0], "exec");
    assert!(r.saw("codex.args").is_none());
    std::fs::remove_file(r.bin().join("codex")).unwrap();
    let missing = r.run(&["ask", "--dry-run", "--agent", "codex", "q"], "0");
    assert!(missing.status.success(), "{}", text(&missing));
    assert!(String::from_utf8_lossy(&missing.stdout).starts_with("codex exec --json"));
}

#[test]
fn via_io_is_reserved_and_never_starts_an_agent() {
    let r = Rig::new();
    fake_agent(&r.bin(), "claude");
    let out = r.run(&["ask", "--via", "io", "q"], "0");
    assert!(!out.status.success());
    assert!(text(&out).contains("not built yet"), "{}", text(&out));
    assert!(r.saw("claude.args").is_none());
    // Set in the config, the same; `--via local` brings the agent back for one call.
    r.config("[ask]\nvia = \"io\"\n");
    let out = r.run(&["ask", "q"], "0");
    assert!(text(&out).contains("not built yet"), "{}", text(&out));
    assert!(r.run(&["ask", "--via", "local", "q"], "0").status.success());
    assert_eq!(r.question("claude").as_deref(), Some("q"));
}

#[test]
fn an_unknown_agent_via_or_thinking_is_a_usage_error() {
    let r = Rig::new();
    fake_agent(&r.bin(), "claude");
    for args in [
        ["ask", "--agent", "gemini", "q"],
        ["ask", "--via", "cloud", "q"],
        ["ask", "--thinking", "loud", "q"],
    ] {
        let out = r.run(&args, "0");
        assert_eq!(out.status.code(), Some(2), "{}", text(&out));
    }
    r.config("[ask]\nagent = \"nope\"\n");
    assert_eq!(r.run(&["ask", "q"], "0").status.code(), Some(2));
    assert!(r.saw("claude.args").is_none());
}

#[test]
fn help_says_what_it_does_and_names_only_what_works() {
    let r = Rig::new();
    let out = r.run(&["ask", "--help"], "0");
    let t = text(&out);
    assert!(out.status.success(), "{t}");
    for want in [
        "--via",
        "--agent",
        "--model",
        "--allow-edits",
        "--dry-run",
        "--thinking",
        "haiku",
        "claude",
        "codex",
        "shell",
    ] {
        assert!(t.contains(want), "{want} in {t}");
    }
    assert!(!t.to_lowercase().contains("coming"), "{t}");
    assert!(!t.contains("not built yet"), "{t}");
}
