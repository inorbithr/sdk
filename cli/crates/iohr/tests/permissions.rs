//! Permissions for `iohr ask` (PRD 0197): the binary with a private HOME and a fake `claude`
//! (a Python script) that speaks to the real hidden permission server the way Claude Code does
//! (`--permission-prompt-tool`, <https://code.claude.com/docs/en/cli-reference>). Real
//! processes, a real unix socket, a real pseudo-terminal for the keys; nothing of ours is
//! mocked. Tests skip when `python3` is missing.

#![allow(
    clippy::unwrap_used,
    reason = "test helpers fail the test by panicking"
)]
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::{Value, json};

/// The fake agent. It reads `calls` (a JSON list of `{tool_name, input}`) from `$HOME/out`,
/// asks the server `--mcp-config` names about each, runs a Bash call only when it was allowed,
/// and prints a result naming what happened to each call.
const FAKE_CLAUDE: &str = r#"#!__PY__
import json, os, subprocess, sys
out = os.environ["HOME"] + "/out"
args = sys.argv[1:]
open(out + "/claude.args", "w").write("\n".join(args) + "\n")
sys.stdin.read()
cfg = json.load(open(args[args.index("--mcp-config") + 1]))
srv = cfg["mcpServers"]["iohrperm"]
p = subprocess.Popen([srv["command"]] + srv["args"], stdin=subprocess.PIPE,
                     stdout=subprocess.PIPE, text=True)
def rpc(m):
    p.stdin.write(json.dumps(m) + "\n"); p.stdin.flush()
    return json.loads(p.stdout.readline()) if "id" in m else None
rpc({"jsonrpc": "2.0", "id": 0, "method": "server/discover"})
rpc({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-11-25"}})
rpc({"jsonrpc": "2.0", "method": "notifications/initialized"})
rpc({"jsonrpc": "2.0", "id": 2, "method": "tools/list"})
seen = []
for n, c in enumerate(json.load(open(out + "/calls"))):
    r = rpc({"jsonrpc": "2.0", "id": 10 + n, "method": "tools/call", "params": {
        "name": "approve",
        "arguments": {"tool_name": c["tool_name"], "input": c["input"], "tool_use_id": "t%d" % n}}})
    v = json.loads(r["result"]["content"][0]["text"])
    ran = ""
    if v["behavior"] == "allow" and c["tool_name"] == "Bash":
        ran = subprocess.run(c["input"]["command"], shell=True, capture_output=True, text=True).stdout.strip()
    seen.append("%s:%s" % (v["behavior"], ran))
p.stdin.close()
print(json.dumps({"type": "result", "subtype": "success", "is_error": False,
                  "result": "; ".join(seen), "duration_ms": 5, "num_turns": 1}))
"#;

/// Sends one key per ask over a pseudo-terminal and prints `{"out": ..., "status": ...}`.
const PTY_DRIVER: &str = r#"
import fcntl, json, os, pty, select, struct, sys, termios, time
spec = json.loads(sys.argv[1])
pid, fd = pty.fork()
if pid == 0:
    os.chdir(spec["cwd"])
    os.execve(spec["argv"][0], spec["argv"], dict(e.split("=", 1) for e in spec["env"]))
fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 120, 0, 0))
buf = b""
keys = list(spec["keys"])  # one entry per ask
answered = 0
deadline = time.time() + 60
status = None
while time.time() < deadline:
    r, _, _ = select.select([fd], [], [], 0.2)
    if r:
        try:
            d = os.read(fd, 4096)
        except OSError:
            d = b""
        if not d:
            break
        buf += d
    asks = buf.count(spec.get("marker", "· n no").encode())
    while answered < asks and keys:
        time.sleep(0.2)
        os.write(fd, keys.pop(0).encode())
        answered += 1
try:
    if time.time() >= deadline:
        os.kill(pid, 9)
    _, status = os.waitpid(pid, 0)
except ChildProcessError:
    pass
print(json.dumps({"out": buf.decode("utf-8", "replace"), "status": status}))
"#;

fn python() -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|p| {
        std::env::split_paths(&p)
            .map(|d| d.join("python3"))
            .find(|c| c.is_file())
    })
}

struct Rig {
    home: tempfile::TempDir,
    py: PathBuf,
}

impl Rig {
    /// `None` when there is no `python3`: the test is skipped.
    fn new() -> Option<Self> {
        let py = python()?;
        let r = Self {
            home: tempfile::tempdir().unwrap(),
            py,
        };
        for d in [r.out(), r.bin(), r.work()] {
            std::fs::create_dir_all(d).unwrap();
        }
        let fake = r.bin().join("claude");
        std::fs::write(
            &fake,
            FAKE_CLAUDE.replace("__PY__", &r.py.display().to_string()),
        )
        .unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        Some(r)
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
    fn calls(&self, calls: &Value) {
        std::fs::write(self.out().join("calls"), calls.to_string()).unwrap();
    }
    fn env(&self) -> Vec<(String, String)> {
        vec![
            (
                "PATH".into(),
                format!(
                    "{}:{}:/usr/bin:/bin",
                    self.bin().display(),
                    self.py.parent().unwrap().display()
                ),
            ),
            ("HOME".into(), self.home.path().display().to_string()),
            (
                "IOHR_CONFIG_DIR".into(),
                self.home.path().join("config").display().to_string(),
            ),
        ]
    }
    fn iohr(&self, args: &[&str], extra: &[(&str, &str)]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_iohr"))
            .args(args)
            .current_dir(self.work())
            .env_clear()
            .envs(self.env())
            .envs(extra.iter().copied())
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }
    /// Runs `iohr` on a pseudo-terminal, answering each ask with the next key.
    fn on_terminal(&self, args: &[&str], keys: &[&str]) -> (String, bool) {
        self.on_terminal_with(args, keys, &[], "\u{b7} n no")
    }
    /// [`Self::on_terminal`] with more environment (`TERM` makes the look drawn) and the text
    /// that tells the driver an ask is open.
    fn on_terminal_with(
        &self,
        args: &[&str],
        keys: &[&str],
        extra: &[(&str, &str)],
        marker: &str,
    ) -> (String, bool) {
        let mut command = vec![env!("CARGO_BIN_EXE_iohr").to_owned()];
        command.extend(args.iter().map(|a| (*a).to_owned()));
        let env: Vec<String> = self
            .env()
            .into_iter()
            .chain(
                extra
                    .iter()
                    .map(|(k, v)| ((*k).to_owned(), (*v).to_owned())),
            )
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        let spec = json!({
            "argv": command, "env": env, "cwd": self.work(), "keys": keys, "marker": marker,
        });
        let o = Command::new(&self.py)
            .args(["-I", "-c", PTY_DRIVER, &spec.to_string()])
            .output()
            .unwrap();
        let v: Value = serde_json::from_slice(&o.stdout)
            .unwrap_or_else(|_| panic!("{}", String::from_utf8_lossy(&o.stderr)));
        (
            v["out"].as_str().unwrap().to_owned(),
            v["status"] == json!(0),
        )
    }
    fn saw(&self, f: &str) -> String {
        std::fs::read_to_string(self.out().join(f)).unwrap_or_default()
    }
    fn project_file(&self) -> PathBuf {
        self.work().join(".inorbit/permissions.toml")
    }
}

fn text(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

fn bash(cmd: &str) -> Value {
    json!({"tool_name": "Bash", "input": {"command": cmd}})
}

fn answer(o: &Output) -> String {
    let v: Value =
        serde_json::from_slice(&o.stdout).unwrap_or_else(|_| panic!("not json: {}", text(o)));
    v["answer"].as_str().unwrap().to_owned()
}

macro_rules! rig {
    () => {
        match Rig::new() {
            Some(r) => r,
            None => return,
        }
    };
}

// ------------------------------------------------------------ the commands

#[test]
fn the_commands_keep_rules_in_files_and_never_allow_a_secret() {
    let r = rig!();
    let allow = r.iohr(&["permissions", "allow", "Bash(ls:*)"], &[]);
    assert!(allow.status.success(), "{}", text(&allow));
    let user = r.iohr(&["permissions", "deny", "Bash(rm:*)", "--user"], &[]);
    assert!(user.status.success(), "{}", text(&user));
    // Project rules sit in the project's folder, user rules in the config dir.
    assert!(r.project_file().is_file());
    assert!(r.home.path().join("config/permissions.toml").is_file());
    let list = r.iohr(&["--json", "permissions", "list"], &[]);
    let v: Value = serde_json::from_slice(&list.stdout).unwrap();
    let rules = v["rules"].as_array().unwrap();
    assert_eq!(rules.len(), 2, "{v}");
    assert!(
        rules
            .iter()
            .any(|x| x["rule"] == "Bash(ls:*)" && x["kind"] == "allow" && x["scope"] == "project")
    );
    assert!(
        rules
            .iter()
            .any(|x| x["rule"] == "Bash(rm:*)" && x["kind"] == "deny" && x["scope"] == "user")
    );
    // check: deny beats allow, the secret list beats everything, no rule means ask.
    let check =
        |tool: &str, input: &str| text(&r.iohr(&["permissions", "check", tool, input], &[]));
    assert!(
        check("Bash", "ls -la").starts_with("allow"),
        "{}",
        check("Bash", "ls -la")
    );
    assert!(check("Bash", "rm -rf x").starts_with("deny"));
    assert!(check("Bash", "git push").starts_with("ask"));
    assert!(check("Read", "./.env").starts_with("deny"));
    // A line that substitutes a command is never allowed by a rule for its first word.
    assert!(check("Bash", "ls $(echo hi)").starts_with("ask"));
    // Every part of a compound line must be allowed.
    assert!(check("Bash", "ls && git push").starts_with("ask"));
    // Remove.
    let gone = r.iohr(&["permissions", "remove", "Bash(ls:*)"], &[]);
    assert!(gone.status.success(), "{}", text(&gone));
    assert!(check("Bash", "ls -la").starts_with("ask"));
}

#[test]
fn a_rule_that_could_open_a_secret_file_cannot_be_allowed_and_wins_even_when_written_by_hand() {
    let r = rig!();
    for rule in [
        "Read(./.env)",
        "Read(**/.env)",
        "Read(~/.ssh/id_rsa)",
        "Read(**/*.pem)",
        "Read(**/*secret*)",
        "Read(~/.aws/credentials)",
    ] {
        for scope in ["--project", "--user"] {
            let o = r.iohr(&["permissions", "allow", rule, scope], &[]);
            assert_eq!(o.status.code(), Some(2), "{rule} {scope}: {}", text(&o));
            assert!(text(&o).contains("never allowed"), "{}", text(&o));
        }
    }
    assert!(!r.project_file().exists());
    // Blanket rules are refused too.
    for rule in ["Bash(*)", "Bash"] {
        let o = r.iohr(&["permissions", "allow", rule], &[]);
        assert_eq!(o.status.code(), Some(2), "{rule}: {}", text(&o));
    }
    // A file edited by hand still loses to the list: the call is refused and the file is not read.
    std::fs::create_dir_all(r.work().join(".inorbit")).unwrap();
    std::fs::write(
        r.project_file(),
        "allow = [\"Read(./.env)\", \"Read(./notes.txt)\"]\n",
    )
    .unwrap();
    std::fs::write(r.work().join(".env"), "TOKEN=must-not-be-read\n").unwrap();
    let env = |p: &str| text(&r.iohr(&["permissions", "check", "Read", p], &[]));
    assert!(env("./.env").starts_with("deny"), "{}", env("./.env"));
    assert!(env("./notes.txt").starts_with("allow"));
    r.calls(&json!([{"tool_name": "Read", "input": {"file_path": "./.env"}}]));
    let o = r.iohr(&["--json", "ask", "--headless", "q"], &[]);
    assert!(o.status.success(), "{}", text(&o));
    assert_eq!(answer(&o), "deny:");
}

// ------------------------------------------------------------ the run

#[test]
fn rules_run_what_they_allow_and_refuse_the_rest_without_a_terminal() {
    let r = rig!();
    assert!(
        r.iohr(&["permissions", "allow", "Bash(echo:*)"], &[])
            .status
            .success()
    );
    assert!(
        r.iohr(&["permissions", "deny", "Bash(echo no)"], &[])
            .status
            .success()
    );
    r.calls(&json!([
        bash("echo hi"),
        bash("echo no"),
        bash("touch made.txt")
    ]));
    // No terminal, no flag: the default is `rules`.
    let o = r.iohr(&["--json", "ask", "--headless", "q"], &[]);
    assert!(o.status.success(), "{}", text(&o));
    assert_eq!(answer(&o), "allow:hi; deny:; deny:");
    assert!(!r.work().join("made.txt").exists());
    // The run offers Bash behind the prompt tool, keeps the secret list and never skips checks.
    let args = r.saw("claude.args");
    let lines: Vec<&str> = args.lines().collect();
    let at = |f: &str| lines.iter().position(|l| *l == f).unwrap();
    assert_eq!(lines[at("--tools") + 1], "Read,Grep,Glob,Bash", "{args}");
    assert_eq!(
        lines[at("--permission-prompt-tool") + 1],
        "mcp__iohrperm__approve"
    );
    assert!(lines.contains(&"--restricted") && lines.contains(&"--strict-mcp-config"));
    assert!(lines.contains(&"--disallowedTools=Read(./.env)"), "{args}");
    for bad in [
        "--dangerously-skip-permissions",
        "--allowedTools",
        "--bare",
        "--settings",
    ] {
        assert!(!lines.contains(&bad), "{bad} in {args}");
    }
    // The private folder is gone with the run.
    let dir = Path::new(lines[at("--mcp-config") + 1]).parent().unwrap();
    assert!(!dir.exists(), "{} was left behind", dir.display());
    // The audit log names what was decided and never what the command printed.
    let log = std::fs::read_to_string(r.work().join(".inorbit/permissions.log")).unwrap();
    assert!(
        log.contains("echo hi") || log.contains("Bash(echo:*)"),
        "{log}"
    );
    assert!(log.contains("allow") && log.contains("den"), "{log}");
    assert!(!log.contains("made.txt\n\nhi"), "{log}");
    // The plain answer lists what was refused, with the command that would allow it.
    let o = r.iohr(&["ask", "--headless", "q"], &[]);
    let t = text(&o);
    assert!(t.contains("touch made.txt"), "{t}");
    assert!(t.contains("permissions allow"), "{t}");
}

#[test]
fn deny_mode_offers_no_bash_and_the_environment_and_config_choose_it() {
    let r = rig!();
    r.calls(&json!([]));
    for (args, env) in [
        (
            vec!["ask", "--headless", "--permissions", "deny", "q"],
            vec![],
        ),
        (
            vec!["ask", "--headless", "q"],
            vec![("IOHR_ASK_PERMISSIONS", "deny")],
        ),
    ] {
        let _ = r.iohr(&args, &env);
        // The fake needs the prompt tool, so it may exit; only what it was started with counts.
        let a = r.saw("claude.args");
        assert!(!a.contains("--mcp-config") && !a.contains("Bash"), "{a}");
    }
    // The config key.
    std::fs::create_dir_all(r.home.path().join("config")).unwrap();
    std::fs::write(
        r.home.path().join("config/config.toml"),
        "[ask]\npermissions = \"deny\"\n",
    )
    .unwrap();
    let _ = r.iohr(&["ask", "--headless", "q"], &[]);
    assert!(!r.saw("claude.args").contains("--mcp-config"));
    // A word that is no mode is a usage error.
    let o = r.iohr(&["ask", "--headless", "--permissions", "always", "q"], &[]);
    assert_eq!(o.status.code(), Some(2), "{}", text(&o));
}

// ------------------------------------------------------------ the terminal

#[test]
fn y_runs_it_n_refuses_it_and_a_saves_the_narrowest_rule_so_the_next_run_does_not_ask() {
    let r = rig!();
    let args = [
        "ask",
        "--headless",
        "--card",
        "--permissions",
        "prompt",
        "q",
    ];
    // y: the card says who asks and what, the command runs, nothing is saved.
    r.calls(&json!([bash("echo hello")]));
    let (screen, ok) = r.on_terminal(&args, &["y"]);
    assert!(ok, "{screen}");
    assert!(
        screen.contains("Io (AI) wants to run `echo hello` in work"),
        "{screen}"
    );
    assert!(screen.contains("y once"), "{screen}");
    assert!(screen.contains("echo hello allowed this time"), "{screen}");
    assert!(screen.contains("allow:hello"), "{screen}");
    assert!(!r.project_file().exists());
    // n: it does not run.
    r.calls(&json!([bash("touch made.txt")]));
    let (screen, ok) = r.on_terminal(&args, &["n"]);
    assert!(ok, "{screen}");
    assert!(screen.contains("touch made.txt not allowed"), "{screen}");
    assert!(screen.contains("deny:"), "{screen}");
    assert!(!r.work().join("made.txt").exists());
    // An unknown key keeps waiting; Enter is no.
    let (screen, _) = r.on_terminal(&args, &["x\r"]);
    assert!(screen.contains("touch made.txt not allowed"), "{screen}");
    assert!(!r.work().join("made.txt").exists());
    // a: the rule is the narrowest, never a blanket, and the next run goes without asking.
    r.calls(&json!([bash("echo again")]));
    let (screen, ok) = r.on_terminal(&args, &["a"]);
    assert!(ok, "{screen}");
    // With no card to carry an Undo action the plain note names the command that does it.
    assert!(
        screen.contains("echo allowed in this project from now on"),
        "{screen}"
    );
    assert!(screen.contains("Bash(echo:*)"), "{screen}");
    assert!(
        screen.contains("permissions remove 'Bash(echo:*)'"),
        "{screen}"
    );
    assert!(!screen.contains("Allowed always"), "{screen}");
    let file = std::fs::read_to_string(r.project_file()).unwrap();
    assert!(
        file.contains("Bash(echo:*)") && !file.contains("Bash(*)"),
        "{file}"
    );
    let (screen, ok) = r.on_terminal(&args, &[]);
    assert!(ok, "{screen}");
    assert!(!screen.contains("wants to run"), "asked again: {screen}");
    assert!(screen.contains("allow:again"), "{screen}");
    // The log has one line per decision, with no output in it.
    let log = std::fs::read_to_string(r.work().join(".inorbit/permissions.log")).unwrap();
    assert!(log.lines().count() >= 4, "{log}");
    assert!(
        !log.contains("hello\n") || log.contains("echo hello"),
        "{log}"
    );
}

#[test]
fn undo_in_the_cards_menu_takes_the_saved_rule_back_through_the_one_remover() {
    let r = rig!();
    let ask = [
        "ask",
        "--headless",
        "--card",
        "--permissions",
        "prompt",
        "q",
    ];
    r.calls(&json!([bash("echo again")]));
    let (screen, ok) = r.on_terminal(&ask, &["a"]);
    assert!(ok, "{screen}");
    let file = std::fs::read_to_string(r.project_file()).unwrap();
    assert!(file.contains("Bash(echo:*)"), "{file}");
    // The hook hands the rule to the menu; `u` is Undo and the rule is gone from the file.
    let work = r.work().display().to_string();
    let menu = ["ask", "--menu", "--cwd", &work, "--undo", "Bash(echo:*)"];
    let term = [("TERM", "xterm-256color")];
    let (screen, _) = r.on_terminal_with(&menu, &["u"], &term, "What next?");
    let screen = console::strip_ansi_codes(&screen).into_owned();
    assert!(screen.contains("Undo"), "{screen}");
    assert!(
        screen.contains("echo is no longer allowed in this project"),
        "{screen}"
    );
    let file = std::fs::read_to_string(r.project_file()).unwrap();
    assert!(!file.contains("Bash(echo:*)"), "{file}");
    // The next run asks again: the rule really is gone, not hidden.
    r.calls(&json!([bash("echo again")]));
    let (screen, _) = r.on_terminal(&ask, &["n"]);
    assert!(screen.contains("wants to run"), "{screen}");
}

#[test]
fn on_a_colour_terminal_the_ask_is_an_io_card_that_starts_on_no() {
    let r = rig!();
    let args = [
        "ask",
        "--headless",
        "--card",
        "--permissions",
        "prompt",
        "q",
    ];
    let term = [("TERM", "xterm-256color")];
    // The colours are the card's; what a person reads is the text between them.
    let run = |keys: &[&str]| {
        let (screen, ok) = r.on_terminal_with(&args, keys, &term, "AI request");
        (console::strip_ansi_codes(&screen).into_owned(), ok)
    };
    // The card: the AI badge, who asks and what, the folder and the numbered rows.
    r.calls(&json!([bash("touch made.txt")]));
    let (screen, ok) = run(&["\r"]);
    assert!(ok, "{screen}");
    for want in [
        "AI request",
        "Io (AI) wants to run",
        "touch made.txt",
        "in work",
        "[1] Allow once",
        "[2] Always in this project",
        "[3] No",
    ] {
        assert!(screen.contains(want), "{want}: {screen}");
    }
    // Enter on the card is No: a stray Enter never allows anything.
    // The answer card carries the result as a quiet status row inside it, not a loose line.
    assert!(
        screen.contains("\u{2717} touch made.txt not allowed"),
        "{screen}"
    );
    assert!(!screen.contains("Not allowed"), "{screen}");
    assert!(!r.work().join("made.txt").exists());
    // 1 is Allow once; nothing is saved.
    r.calls(&json!([bash("echo hello")]));
    let (screen, ok) = run(&["1"]);
    assert!(ok, "{screen}");
    assert!(
        screen.contains("\u{2713} echo hello allowed this time") && screen.contains("allow:hello"),
        "{screen}"
    );
    assert!(!r.project_file().exists());
    // y, n and Esc-free letters still answer.
    r.calls(&json!([bash("touch made.txt")]));
    let (screen, _) = run(&["n"]);
    assert!(
        screen.contains("\u{2717} touch made.txt not allowed"),
        "{screen}"
    );
    assert!(!r.work().join("made.txt").exists());
    // Up from No lands on Always; Enter takes it, and the narrowest rule is saved.
    r.calls(&json!([bash("echo again")]));
    let (screen, ok) = run(&["\x1b[A\r"]);
    assert!(ok, "{screen}");
    // In the card: the plain sentence, then the rule and the file dimmed under it. Nothing
    // of it is loose above the card, and the undo is the card's own action.
    assert!(
        screen.contains("\u{2713} echo allowed in this project from now on"),
        "{screen}"
    );
    assert!(
        screen.contains("Bash(echo:*) \u{b7} .inorbit/permissions.toml"),
        "{screen}"
    );
    assert!(!screen.contains("Allowed always"), "{screen}");
    let card_at = screen.find("AI answer").unwrap_or_default();
    assert!(
        screen
            .find("allowed in this project from now on")
            .unwrap_or_default()
            > card_at,
        "{screen}"
    );
    assert!(r.project_file().exists());
    // With a colour-less terminal the ask is the one plain line, as before.
    r.calls(&json!([bash("printf plain")]));
    let (screen, ok) = r.on_terminal_with(
        &args,
        &["y"],
        &[("NO_COLOR", "1"), ("TERM", "xterm")],
        "\u{b7} n no",
    );
    assert!(ok, "{screen}");
    assert!(
        screen.contains("wants to run `printf plain` in work"),
        "{screen}"
    );
    assert!(
        !screen.contains("AI request") && !screen.contains('\u{256d}'),
        "{screen}"
    );
}

#[test]
fn a_secret_file_is_refused_without_an_ask() {
    let r = rig!();
    std::fs::write(r.work().join(".env"), "TOKEN=must-not-be-read\n").unwrap();
    r.calls(&json!([{"tool_name": "Read", "input": {"file_path": "./.env"}}]));
    let (screen, ok) = r.on_terminal(
        &[
            "ask",
            "--headless",
            "--card",
            "--permissions",
            "prompt",
            "q",
        ],
        &["y"],
    );
    assert!(ok, "{screen}");
    assert!(!screen.contains("wants to"), "{screen}");
    assert!(screen.contains("deny:"), "{screen}");
}
