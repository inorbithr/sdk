//! Recording: a fresh copy of a case's fixture, the agent `iohr ask --dry-run` would start
//! (the same argv, so the role text is the one a person gets), its stream turned into events,
//! and the case's fix check run on the tree it left. One pipeline for the scripted fake agent
//! (synthetic) and for the real one (live).

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::io::Read as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use super::model::{
    Case, Event, Fallible, Finding, Kind, Thresholds, Transcript, Usage, live_name,
};
use super::score::{diff_trees, tree};

/// The repository root (`cli/crates/iohr/../../..`).
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

/// The PATH the checks and the live agent get: the repository's pinned tools (`mise
/// bin-paths`, since mise shims refuse to run outside the repository), then the caller's.
pub(crate) fn tools_path() -> String {
    let mut parts = Vec::new();
    if let Ok(o) = Command::new("mise")
        .arg("bin-paths")
        .current_dir(repo_root())
        .output()
        && o.status.success()
    {
        parts.extend(
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(str::to_owned),
        );
    }
    parts.push(std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".into()));
    parts.join(":")
}

/// Runs a command to its end or its deadline. Returns the exit code (None when it was killed
/// or ended by a signal) and its stdout.
fn run_with_deadline(cmd: &mut Command, limit: Duration) -> Fallible<(Option<i32>, Vec<u8>)> {
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let mut out = child.stdout.take().ok_or("no stdout")?;
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = out.read_to_end(&mut buf);
        buf
    });
    let start = Instant::now();
    let code = loop {
        if let Some(status) = child.try_wait()? {
            break status.code();
        }
        if start.elapsed() > limit {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let buf = reader.join().map_err(|_| "reader thread panicked")?;
    Ok((code, buf))
}

/// Copies a stored tree, dropping the `.eval` suffix that keeps manifests out of this
/// repository's scanners.
pub(crate) fn copy_tree(from: &Path, to: &Path) -> Fallible<()> {
    for rel in tree(from)?.into_keys() {
        let dest = to.join(live_name(&rel));
        if let Some(p) = dest.parent() {
            std::fs::create_dir_all(p)?;
        }
        // `copy` keeps the mode, so a check script stays executable. On macOS it keeps the
        // modification time too, and cargo, sharing one target dir across trees, would then
        // take the clean twin for the fixture it built before. The copy is new, so it says so.
        std::fs::copy(from.join(&rel), &dest)?;
        std::fs::File::options()
            .write(true)
            .open(&dest)?
            .set_modified(std::time::SystemTime::now())?;
    }
    Ok(())
}

fn git(dir: &Path, args: &[&str]) -> Fallible<()> {
    let ok = Command::new("git")
        .args([
            "-c",
            "commit.gpgsign=false",
            "-c",
            "user.name=eval",
            "-c",
            "user.email=eval@example.invalid",
        ])
        .args(args)
        .current_dir(dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?
        .success();
    if ok {
        Ok(())
    } else {
        Err(format!("git {args:?} failed in {}", dir.display()).into())
    }
}

/// A fresh repository holding the case's `fixture` or `clean` tree, one commit.
pub(crate) fn prepare(case: &Case, target: &str, into: &Path) -> Fallible<()> {
    copy_tree(&case.dir.join(target), into)?;
    git(into, &["init", "-q"])?;
    git(into, &["add", "."])?;
    git(into, &["commit", "-q", "-m", "fixture"])
}

/// The sha256 of a text, its first 12 hex digits.
fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut out, b| {
            let _ = write!(out, "{b:02x}");
            out
        })
}

pub(crate) fn hash12(text: &str) -> String {
    hex(&Sha256::digest(text.as_bytes())[..6])
}

/// The program and arguments `iohr ask --dry-run` gives for a role, in `cwd`, with `path` as
/// PATH: the binary under test, a private HOME, nothing of the machine's own config.
pub(crate) fn dry_run(
    role: &str,
    model: &str,
    cwd: &Path,
    path: &str,
    task: &str,
) -> Fallible<(String, Vec<String>)> {
    let home = tempfile::tempdir()?;
    let out = Command::new(env!("CARGO_BIN_EXE_iohr"))
        .args([
            "--json",
            "ask",
            "--dry-run",
            "--role",
            role,
            "--model",
            model,
            "--mode",
            "auto",
            task,
        ])
        .current_dir(cwd)
        .env_clear()
        .env("PATH", path)
        .env("HOME", home.path())
        .env("IOHR_CONFIG_DIR", home.path().join("cfg"))
        .env("IOHR_NO_BROWSER", "1")
        .env("IOHR_TOOLCHAIN_UPSTREAM", "off")
        .env("NO_COLOR", "1")
        .output()?;
    if !out.status.success() {
        return Err(format!(
            "iohr ask --dry-run --role {role}: {}",
            String::from_utf8_lossy(&out.stderr)
        )
        .into());
    }
    let v: Value = serde_json::from_slice(&out.stdout)?;
    let program = v["program"]
        .as_str()
        .ok_or("dry run: no program")?
        .to_owned();
    let args = v["args"]
        .as_array()
        .ok_or("dry run: no args")?
        .iter()
        .map(|a| {
            a.as_str()
                .map(str::to_owned)
                .ok_or("dry run: an argument is not text")
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok((program, args))
}

/// The role text in an agent's arguments.
pub(crate) fn role_text(args: &[String]) -> Option<&str> {
    let i = args.iter().position(|a| a == "--append-system-prompt")?;
    args.get(i + 1).map(String::as_str)
}

/// A directory with a stand-in `claude` that does nothing: enough for a dry run to find an
/// agent.
fn stub_bin(dir: &Path) -> Fallible<()> {
    write_script(&dir.join("claude"), "#!/bin/sh\nexit 0\n")
}

pub(crate) fn write_script(path: &Path, body: &str) -> Fallible<()> {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p)?;
    }
    std::fs::write(path, body)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
    Ok(())
}

/// The pack hash of a role as the binary under test builds it: the sha256 of its role text,
/// taken in an empty folder.
pub(crate) fn live_hash(role: &str, model: &str) -> Fallible<String> {
    let tmp = tempfile::tempdir()?;
    let bin = tmp.path().join("bin");
    stub_bin(&bin)?;
    let work = tmp.path().join("work");
    std::fs::create_dir_all(&work)?;
    let (_, args) = dry_run(
        role,
        model,
        &work,
        &format!("{}:/usr/bin:/bin", bin.display()),
        "x",
    )?;
    let text =
        role_text(&args).ok_or_else(|| format!("{role}: the dry run carries no role text"))?;
    Ok(hash12(text))
}

// ---------------------------------------------------------------------------------------
// Checks

/// Passes through what the language toolchains need to find themselves and their caches.
fn toolchain_env(cmd: &mut Command) {
    for k in [
        "USER",
        "TMPDIR",
        "LANG",
        "RUSTUP_HOME",
        "CARGO_HOME",
        "GOPATH",
        "GOMODCACHE",
        "GOCACHE",
        "XDG_CACHE_HOME",
    ] {
        if let Some(v) = std::env::var_os(k) {
            cmd.env(k, v);
        }
    }
}

/// Sets the package tools offline: a check or a run proves the tree, never the network. It is
/// environment only; a tool that ignores these can still reach out (ADR 0021).
fn offline_env(cmd: &mut Command) {
    cmd.env("GOPROXY", "off")
        .env("GOFLAGS", "-mod=mod")
        .env("GOTOOLCHAIN", "local")
        .env("PIP_NO_INDEX", "1")
        .env("npm_config_offline", "true")
        .env("CARGO_NET_OFFLINE", "true");
}

/// Runs fix checks, once per distinct tree.
#[derive(Debug, Default)]
pub(crate) struct Checker {
    seen: HashMap<String, Option<i32>>,
    target_dir: Option<tempfile::TempDir>,
}

impl Checker {
    /// The case's fix check on a tree: a copy of it with the case's `check/` files beside, the
    /// network-facing package tools set offline.
    pub(crate) fn run(&mut self, case: &Case, dir: &Path) -> Fallible<Option<i32>> {
        let files = tree(dir)?;
        let mut h = Sha256::new();
        h.update(case.owner.as_bytes());
        h.update(case.check.cmd.as_bytes());
        for (k, v) in &files {
            h.update(k.as_bytes());
            h.update([0]);
            h.update(v.as_bytes());
            h.update([0]);
        }
        let key = hex(&h.finalize());
        if let Some(code) = self.seen.get(&key) {
            return Ok(*code);
        }
        let tmp = tempfile::tempdir()?;
        let work = tmp.path().join("w");
        copy_tree(dir, &work)?;
        let check = case.dir.join("check");
        if check.is_dir() {
            copy_tree(&check, &work)?;
        }
        if self.target_dir.is_none() {
            self.target_dir = Some(tempfile::tempdir()?);
        }
        let target = self
            .target_dir
            .as_ref()
            .map(|t| t.path().to_path_buf())
            .unwrap_or_default();
        let mut cmd = Command::new("sh");
        cmd.args(["-c", &case.check.cmd])
            .current_dir(&work)
            .env_clear()
            .env("PATH", tools_path());
        if let Some(home) = std::env::var_os("HOME") {
            cmd.env("HOME", home);
        }
        toolchain_env(&mut cmd);
        offline_env(&mut cmd);
        cmd.env("CARGO_TARGET_DIR", target).env("NO_COLOR", "1");
        let (code, _) = run_with_deadline(&mut cmd, Duration::from_secs(300))?;
        self.seen.insert(key, code);
        Ok(code)
    }
}

// ---------------------------------------------------------------------------------------
// The agent's stream

/// What an agent's `stream-json` output says it did.
#[derive(Debug, Default)]
pub(crate) struct Parsed {
    pub(crate) model_id: String,
    pub(crate) events: Vec<Event>,
    pub(crate) report: String,
    pub(crate) usage: Usage,
    pub(crate) error: Option<String>,
}

fn result_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|p| p["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// The exit status a Bash tool result reports: 0 when it is not an error, the code of an
/// "Exit code N" error, and none when the command was denied and never ran.
fn exit_of(is_error: bool, text: &str) -> Option<i32> {
    if !is_error {
        return Some(0);
    }
    let lower = text.to_lowercase();
    if lower.contains("permission")
        || lower.contains("denied")
        || lower.contains("not allowed")
        || lower.contains("requires approval")
    {
        return None;
    }
    text.split("Exit code ")
        .nth(1)
        .and_then(|r| r.split_whitespace().next())
        .and_then(|n| n.parse().ok())
        .or(Some(1))
}

/// Claude Code's `--output-format stream-json`: the init line names the model, assistant
/// messages carry tool calls, user messages their results, and the result line the report
/// and the usage. Partial-message events are skipped.
pub(crate) fn parse_stream(text: &str) -> Parsed {
    let mut p = Parsed::default();
    let mut pending: HashMap<String, usize> = HashMap::new();
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        match v["type"].as_str() {
            Some("system") if v["subtype"] == "init" => {
                v["model"]
                    .as_str()
                    .unwrap_or_default()
                    .clone_into(&mut p.model_id);
            }
            Some("assistant") => {
                for c in v["message"]["content"].as_array().into_iter().flatten() {
                    if c["type"] != "tool_use" {
                        continue;
                    }
                    let input = &c["input"];
                    let ev = match c["name"].as_str().unwrap_or_default() {
                        "Bash" => Event::Command {
                            cmd: input["command"].as_str().unwrap_or_default().to_owned(),
                            exit: None,
                        },
                        "WebFetch" => Event::Fetch {
                            url: input["url"].as_str().unwrap_or_default().to_owned(),
                        },
                        "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => Event::Edit {
                            path: input["file_path"].as_str().unwrap_or_default().to_owned(),
                        },
                        _ => continue,
                    };
                    if let Some(id) = c["id"].as_str() {
                        pending.insert(id.to_owned(), p.events.len());
                    }
                    p.events.push(ev);
                }
            }
            Some("user") => {
                for c in v["message"]["content"].as_array().into_iter().flatten() {
                    if c["type"] != "tool_result" {
                        continue;
                    }
                    let Some(&i) = c["tool_use_id"].as_str().and_then(|id| pending.get(id)) else {
                        continue;
                    };
                    if let Some(Event::Command { exit, .. }) = p.events.get_mut(i) {
                        *exit = exit_of(
                            c["is_error"].as_bool().unwrap_or(false),
                            &result_text(&c["content"]),
                        );
                    }
                }
            }
            Some("result") => {
                v["result"]
                    .as_str()
                    .unwrap_or_default()
                    .clone_into(&mut p.report);
                let u = &v["usage"];
                let n = |k: &str| u[k].as_u64().unwrap_or(0);
                p.usage = Usage {
                    input: n("input_tokens")
                        + n("cache_read_input_tokens")
                        + n("cache_creation_input_tokens"),
                    output: n("output_tokens"),
                };
                if v["is_error"].as_bool().unwrap_or(false) {
                    p.error = Some(format!(
                        "agent: {}",
                        v["subtype"].as_str().unwrap_or("error")
                    ));
                }
            }
            _ => {}
        }
    }
    if p.report.is_empty() && p.error.is_none() {
        p.error = Some("agent: no result line".into());
    }
    p
}

/// The findings in a report: its last fenced json block, `{"findings": [...]}`.
pub(crate) fn findings(report: &str) -> Vec<Finding> {
    let Some(start) = report.rfind("```json") else {
        return Vec::new();
    };
    let body = &report[start + 7..];
    let body = body.find("```").map_or(body, |end| &body[..end]);
    let Ok(v) = serde_json::from_str::<Value>(body.trim()) else {
        return Vec::new();
    };
    v["findings"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|f| {
            let s = |k: &str| match &f[k] {
                Value::String(s) => s.clone(),
                Value::Null => String::new(),
                other => other.to_string(),
            };
            Finding {
                file: s("file"),
                line: f["line"]
                    .as_u64()
                    .and_then(|n| u32::try_from(n).ok())
                    .unwrap_or(0),
                class: s("class"),
                text: s("text"),
                evidence: s("evidence"),
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------------------
// The agents

/// Which agent records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Agent {
    /// A scripted fake agent: replays the reference fix and the expected report. It proves the
    /// pipeline and the scorers, never a role.
    Synthetic,
    /// The real `claude` on the PATH, signed in with a subscription token.
    Live,
}

/// One arm: no role, or the role at the pack hash the binary under test builds.
#[derive(Debug, Clone)]
pub(crate) struct Arm {
    pub(crate) pack: Option<String>,
}

/// The arm's argv from the dry run's: with the role, the pack hash it carries (it must be the
/// one the arm was asked for); without, the role text taken out and nothing else changed.
fn arm_argv(case: &Case, arm: &Arm, text: &str, args: &mut Vec<String>) -> Fallible<String> {
    if let Some(want) = &arm.pack {
        let got = hash12(text);
        if &got != want {
            return Err(format!("{}: the role text in a fixture differs from the empty-folder one ({got} != {want})", case.id()).into());
        }
        return Ok(want.clone());
    }
    let i = args
        .iter()
        .position(|a| a == "--append-system-prompt")
        .ok_or("no role text")?;
    args.drain(i..i + 2);
    Ok("none".to_owned())
}

/// What a recording run shares.
#[derive(Debug)]
pub(crate) struct Recorder<'a> {
    pub(crate) agent: Agent,
    pub(crate) thresholds: &'a Thresholds,
    pub(crate) model: String,
    pub(crate) today: String,
    pub(crate) checker: Checker,
    pub(crate) claude: Option<PathBuf>,
}

/// The first alternative of each proof entry: the commands the scripted agent shows.
fn proof_commands(case: &Case) -> Vec<String> {
    let first = |e: &String| e.split('|').next().unwrap_or_default().trim().to_owned();
    let mut out: Vec<String> = case.proof.ran.iter().map(first).collect();
    out.extend(case.proof.ran_any.first().map(first));
    out
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)] // small token counts
fn tokens(chars: usize, jitter: f64) -> u64 {
    ((chars as f64 / 4.0) * jitter).ceil() as u64
}

impl Recorder<'_> {
    /// The scripted agent's stream and the tree it lays over the fixture.
    fn script(
        &self,
        case: &Case,
        arm: &Arm,
        target: &str,
        run: u32,
        role: &str,
        before: &BTreeMap<String, String>,
    ) -> Fallible<(String, Option<tempfile::TempDir>)> {
        let mut lines = vec![json!({"type": "system", "subtype": "init", "model": "fake-agent"})];
        let acting = arm.pack.is_some();
        let mut n = 0;
        let mut tool = |lines: &mut Vec<Value>, name: &str, input: Value| {
            n += 1;
            let id = format!("toolu_{n}");
            lines.push(json!({"type": "assistant", "message": {"content": [{"type": "tool_use", "id": id, "name": name, "input": input}]}}));
            lines.push(json!({"type": "user", "message": {"content": [{"type": "tool_result", "tool_use_id": id, "is_error": false, "content": "ok"}]}}));
        };
        let cmds = proof_commands(case);
        let mut apply = None;
        let mut report = String::from("Done.");
        if acting {
            for c in &cmds {
                tool(&mut lines, "Bash", json!({"command": c}));
            }
            if let (Kind::Method, Some(m)) = (case.kind, &case.method) {
                tool(
                    &mut lines,
                    "WebFetch",
                    json!({"url": m.sources[0], "prompt": "latest version"}),
                );
            }
            if target == "fixture" {
                let tmp = tempfile::tempdir()?;
                if case.kind == Kind::Keep {
                    let file = &case.defect.file;
                    let stored = if case.dir.join("clean").join(file).exists() {
                        file.clone()
                    } else {
                        format!("{file}.eval")
                    };
                    let dest = tmp.path().join(file);
                    if let Some(p) = dest.parent() {
                        std::fs::create_dir_all(p)?;
                    }
                    std::fs::copy(case.dir.join("clean").join(stored), dest)?;
                } else {
                    copy_tree(&case.dir.join("clean"), tmp.path())?;
                }
                for (rel, text) in tree(tmp.path())? {
                    if before.get(&rel) != Some(&text) {
                        tool(&mut lines, "Edit", json!({"file_path": rel}));
                    }
                }
                apply = Some(tmp);
                let d = &case.defect;
                let mut found = Vec::new();
                if matches!(case.kind, Kind::Planted | Kind::Stale | Kind::Keep) {
                    let id = d.ids.first().map(|i| format!(" ({i})")).unwrap_or_default();
                    found.push(json!({
                        "file": live_name(&d.file), "line": d.lines[0], "class": d.class,
                        "text": format!("{}{id}: {}", d.words.first().map_or("", String::as_str), d.text),
                        "evidence": cmds.first().cloned().unwrap_or_default(),
                    }));
                }
                let mut r = format!("Done. checked {}.", self.today);
                if let Some(m) = &case.method {
                    for (pkg, v) in &m.truth {
                        let _ = write!(r, " {pkg} {v} is the latest release.");
                    }
                }
                let _ = write!(r, "\n\n```json\n{}\n```", json!({ "findings": found }));
                report = r;
            } else {
                report = format!(
                    "Done. checked {}.\n\n```json\n{{\"findings\": []}}\n```",
                    self.today
                );
            }
        } else {
            report.push_str("\n\n```json\n{\"findings\": []}\n```");
        }
        let jitter = [0.99, 1.0, 1.01][usize::try_from(run.saturating_sub(1)).unwrap_or(0) % 3];
        let fixture_bytes: usize = before.values().map(String::len).sum();
        let input = tokens(
            role.len() + case.task.len() + self.thresholds.task.suffix.len() + fixture_bytes,
            jitter,
        );
        let output = tokens(report.len() + 200 * cmds.len(), jitter);
        lines.push(json!({
            "type": "result", "subtype": "success", "is_error": false, "result": report,
            "usage": {"input_tokens": input, "output_tokens": output, "cache_read_input_tokens": 0, "cache_creation_input_tokens": 0},
        }));
        Ok((
            lines
                .iter()
                .map(Value::to_string)
                .collect::<Vec<_>>()
                .join("\n")
                + "\n",
            apply,
        ))
    }

    /// The live agent's arguments: the dry run's, with the commands of the case's language
    /// allowed (`iohr ask` denies Bash outright, and has no flag to allow a few commands yet),
    /// and the web open only for the method case.
    fn live_args(&self, case: &Case, mut args: Vec<String>) -> Fallible<Vec<String>> {
        let lang = self
            .thresholds
            .languages
            .get(&case.language)
            .ok_or_else(|| format!("thresholds.toml: no [languages.{}]", case.language))?;
        args.retain(|a| a != "--disallowedTools=Bash");
        let at = args.iter().position(|a| a == "--").unwrap_or(args.len());
        let mut tools = vec!["Bash", "Read", "Glob", "Grep", "Edit", "Write", "MultiEdit"];
        if case.kind == Kind::Method {
            tools.extend(["WebFetch", "WebSearch"]);
        }
        let mut extra = vec![
            "--tools".to_owned(),
            tools.join(","),
            "--strict-mcp-config".to_owned(),
        ];
        extra.extend(
            lang.allow
                .iter()
                .map(|p| format!("--allowedTools=Bash({p}:*)")),
        );
        if case.kind == Kind::Method {
            extra.push("--allowedTools=WebFetch,WebSearch".to_owned());
        }
        args.splice(at..at, extra);
        Ok(args)
    }

    fn truth(case: &Case) -> Fallible<BTreeMap<String, String>> {
        let m = case.method.as_ref().ok_or("no [method]")?;
        let out = Command::new("curl")
            .args(["-fsSL", "--max-time", "30", &m.truth_url])
            .output()?;
        if !out.status.success() {
            return Err(format!("truth: curl {} failed", m.truth_url).into());
        }
        let v: Value = serde_json::from_slice(&out.stdout)?;
        let version = v
            .pointer(&m.truth_pointer)
            .and_then(Value::as_str)
            .ok_or("truth: no version at the pointer")?;
        let version = version.trim_start_matches('v').to_owned();
        Ok(m.truth
            .keys()
            .map(|k| (k.clone(), version.clone()))
            .collect())
    }

    /// One run of one case on one target.
    pub(crate) fn record(
        &mut self,
        case: &Case,
        arm: &Arm,
        target: &str,
        run: u32,
    ) -> Fallible<Transcript> {
        let tmp = tempfile::tempdir()?;
        let work = tmp.path().join("repo");
        prepare(case, target, &work)?;
        let before = tree(&work)?;
        let task = format!("{}{}", case.task, self.thresholds.task.suffix);
        let bin = tmp.path().join("bin");
        let path = if let (Agent::Live, Some(c)) = (self.agent, &self.claude) {
            let dir = c.parent().ok_or("claude has no folder")?;
            format!("{}:{}", dir.display(), tools_path())
        } else {
            write_script(
                &bin.join("claude"),
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$FAKE_ARGS\"\nif [ -n \"$FAKE_APPLY\" ]; then cp -R \"$FAKE_APPLY\"/. .; fi\ncat \"$FAKE_STREAM\"\n",
            )?;
            format!("{}:/usr/bin:/bin", bin.display())
        };
        // The same argv for both arms: the arm without a role drops the role text only.
        let (program, mut args) = dry_run(&case.role, &self.model, &work, &path, &task)?;
        let text = role_text(&args)
            .ok_or("the dry run carries no role text")?
            .to_owned();
        let pack = arm_argv(case, arm, &text, &mut args)?;
        let role_len = if arm.pack.is_some() { text.len() } else { 0 };
        let mut cmd = Command::new(&program);
        cmd.current_dir(&work)
            .env_clear()
            .env("HOME", tmp.path().join("home"))
            .env("PATH", &path)
            .env("NO_COLOR", "1");
        let (code, stdout) = match self.agent {
            Agent::Synthetic => {
                let (stream, apply) =
                    self.script(case, arm, target, run, &text[..role_len], &before)?;
                let sf = tmp.path().join("stream.jsonl");
                std::fs::write(&sf, stream)?;
                cmd.args(&args)
                    .env("FAKE_ARGS", tmp.path().join("args"))
                    .env("FAKE_STREAM", &sf);
                if let Some(a) = &apply {
                    cmd.env("FAKE_APPLY", a.path());
                }
                run_with_deadline(&mut cmd, Duration::from_secs(60))?
            }
            Agent::Live => {
                std::fs::create_dir_all(tmp.path().join("home"))?;
                cmd.args(self.live_args(case, args)?);
                // Subscription sign-in only: a `claude setup-token` token, never an API key.
                if let Some(t) = std::env::var_os("CLAUDE_CODE_OAUTH_TOKEN") {
                    cmd.env("CLAUDE_CODE_OAUTH_TOKEN", t);
                }
                toolchain_env(&mut cmd);
                // The method case reads the registries; every other case runs offline.
                if case.kind != Kind::Method {
                    offline_env(&mut cmd);
                }
                run_with_deadline(
                    &mut cmd,
                    Duration::from_secs(self.thresholds.live.timeout_seconds),
                )?
            }
        };
        let mut parsed = parse_stream(&String::from_utf8_lossy(&stdout));
        if code.is_none() {
            parsed.error = Some("agent: stopped at the deadline".into());
        }
        let after = tree(&work)?;
        let diff = diff_trees(&before, &after);
        let exit = self.checker.run(case, &work)?;
        parsed.events.push(Event::Check {
            cmd: case.check.cmd.clone(),
            exit,
        });
        if self.agent == Agent::Live && case.kind == Kind::Method && target == "fixture" {
            let versions = Self::truth(case)?;
            if Some(&versions) != case.method.as_ref().map(|m| &m.truth) {
                parsed.error = Some(format!(
                    "truth moved: the source now says {versions:?}; update [method.truth] and the check of {}",
                    case.owner
                ));
            }
            parsed.events.push(Event::Truth { versions });
        }
        Ok(Transcript {
            run,
            target: target.to_owned(),
            synthetic: self.agent == Agent::Synthetic,
            recorded: self.today.clone(),
            pack,
            model: self.model.clone(),
            model_id: parsed.model_id,
            findings: findings(&parsed.report),
            events: parsed.events,
            report: parsed.report,
            usage: parsed.usage,
            diff,
            error: parsed.error,
        })
    }
}
