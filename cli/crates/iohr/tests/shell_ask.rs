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
            "agent",
            "answer",
            "command",
            "duration_ms",
            "exit_code",
            "label",
            "model",
            "questions",
            "via"
        ]
    );
    assert_eq!(v["questions"], serde_json::json!([]));
    assert_eq!(v["answer"], "It is Rayleigh scattering.");
    assert_eq!(v["agent"], "claude");
    assert_eq!(v["via"], "local");
    assert_eq!(v["model"], "haiku");
    assert_eq!(v["exit_code"], 0);
    assert_eq!(v["duration_ms"], 321);
    // The AI label (AI Act Art. 50) travels with the answer.
    let label = v["label"].as_str().unwrap();
    assert!(
        label.contains("Io · AI answer -- Io running on Claude Code (local)"),
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
    assert!(err.starts_with("Io · AI answer -- Io running on Claude Code (local)"));
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
    assert!(lines[0].starts_with("Io · AI answer -- Io running on Claude Code (local)"));
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
// and the record on stdout is the answer, the command and the link, in that order.
#[test]
fn the_hook_record_is_three_fields_and_the_plain_card_has_no_row() {
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
        assert_eq!(f.len(), 3, "{stdout:?}");
        assert!(
            f[0].contains("run:") && f[0].contains("git status"),
            "{f:?}"
        );
        assert_eq!(f[1], "git status");
        assert_eq!(f[2], "notes.md");
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
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("hook.zsh");
    std::fs::write(&file, hook("zsh").stdout).unwrap();
    let n = Command::new("zsh").arg("-n").arg(&file).output().unwrap();
    assert!(n.status.success(), "{}", String::from_utf8_lossy(&n.stderr));
    // The precmd, with a fake `iohr` that prints its arguments.
    let bin = dir.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let fake = bin.join("iohr");
    std::fs::write(
        &fake,
        "#!/bin/sh\necho \"ARGS: $*\" >&2\nprintf 'the answer\\036ls -la\\036README.md'\n",
    )
    .unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
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
    let have = |c: &str| Command::new(c).arg("--version").output().is_ok();
    if !have("zsh") || !have("python3") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("hook.zsh");
    std::fs::write(&file, hook("zsh").stdout).unwrap();
    let bin = dir.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let fake = bin.join("iohr");
    std::fs::write(
        &fake,
        "#!/bin/sh\necho \"ASKED: $*\" >&2\necho the answer >&2\necho the answer\n",
    )
    .unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let f = file.display();
    // A second `source ~/.zshrc`, then once more with a shell variable of the old guard
    // unset (state a plugin may reset): the wiring must not copy our own widget into the one
    // accept-line calls, or Enter loops until FUNCNEST.
    let out = drive_zsh(
        &file,
        &bin,
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

// The menu's letters: Run and Explain print the one line for the prompt and nothing else;
// Copy and Esc print nothing; Copy goes through tmux when inside it. Nothing here runs the
// command: the only place a command could run is the shell, and it only gets a line.
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
    assert_eq!(line, "git status", "{screen:?}");
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
    assert_eq!(line, "git status", "1 is Run: {screen:?}");
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
// line it prints lands in the buffer and nothing runs until Enter; with text on the line, or
// with no answer yet, the key does what it did.
#[test]
fn the_menu_key_fills_the_prompt_line_and_nothing_runs_until_enter() {
    let have = |c: &str| Command::new(c).arg("--version").output().is_ok();
    if !have("zsh") || !have("python3") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("hook.zsh");
    std::fs::write(&file, hook("zsh").stdout).unwrap();
    let bin = dir.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let log = dir.path().join("menu.log");
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
    let down = "RAW:\u{1b}[B";
    let out = drive_zsh(
        &file,
        &bin,
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

// Shift+Tab on a `?` line goes on from manual (where every shell starts) to plan, auto and round
// again, says which, and every
// question after it carries `--mode`. On another line the key is the shell's own.
#[test]
fn shift_tab_on_a_question_line_cycles_the_mode_and_every_ask_carries_it() {
    let have = |c: &str| Command::new(c).arg("--version").output().is_ok();
    if !have("zsh") || !have("python3") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("hook.zsh");
    std::fs::write(&file, hook("zsh").stdout).unwrap();
    let bin = dir.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let log = dir.path().join("asks.log");
    let fake = bin.join("iohr");
    std::fs::write(
        &fake,
        format!(
            "#!/bin/sh\necho \"$*\" >> {log}\nprintf 'the answer\\036ls\\036notes.md'\n",
            log = log.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let tab = "RAW:\u{1b}[Z";
    let out = drive_zsh(
        &file,
        &bin,
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
    let have = |c: &str| Command::new(c).arg("--version").output().is_ok();
    if !have("zsh") || !have("python3") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("hook.zsh");
    std::fs::write(&file, hook("zsh").stdout).unwrap();
    let bin = dir.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    // The answer is fake; the menu is the real binary.
    let fake = bin.join("iohr");
    std::fs::write(
        &fake,
        format!(
            "#!/bin/sh\ncase \"$*\" in\n*--menu*) exec {real} \"$@\";;\n*) printf 'the answer\\036git status\\036notes.md';;\nesac\n",
            real = env!("CARGO_BIN_EXE_iohr")
        ),
    )
    .unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let out = drive_zsh(
        &file,
        &bin,
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
