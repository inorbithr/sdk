//! `iohr monitors` as a binary: the project's `.inorbit/monitors` read offline, run from here,
//! listed, silenced, cut and installed (RFC 0008, RFC 0234). The cli checks run the binary just
//! built in throwaway folders and leave nothing behind; the project's files are as they were.

#![allow(
    clippy::unwrap_used,
    reason = "test helpers fail the test by panicking"
)]

mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{Value, json};
use support::kept_env;

/// A fresh folder, removed when dropped.
struct Dir(tempfile::TempDir);

impl Dir {
    fn new() -> Dir {
        Dir(tempfile::tempdir().unwrap())
    }
    fn path(&self) -> &Path {
        self.0.path()
    }
    /// What is left in it: names only.
    fn left(&self) -> Vec<String> {
        match fs::read_dir(self.path()) {
            Ok(d) => d
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => panic!("{e}"),
        }
    }
}

/// `iohr monitors --dir <dir> <args>`, its state in `state` and its throwaway folders in `tmp`.
fn monitors(dir: &Path, state: &Path, tmp: &Path, args: &[&str]) -> Output {
    monitors_with(dir, state, tmp, args, &[])
}

fn monitors_with(
    dir: &Path,
    state: &Path,
    tmp: &Path,
    args: &[&str],
    more: &[(&str, &str)],
) -> Output {
    Command::new(env!("CARGO_BIN_EXE_iohr"))
        .arg("monitors")
        .arg("--dir")
        .arg(dir)
        .args(args)
        .env_clear()
        .env("NO_COLOR", "1")
        .envs(kept_env())
        .env("IOHR_MONITORS_STATE", state)
        .env("IOHR_MONITORS_TMP", tmp)
        .envs(more.iter().copied())
        .output()
        .unwrap()
}

/// The answer of a `--json` call: its exit code and the JSON of stdout (null when stdout is
/// empty, as it is for a refusal, which says why in the error envelope on stderr).
fn json_of(dir: &Path, state: &Path, tmp: &Path, args: &[&str]) -> (i32, Value) {
    let out = monitors(dir, state, tmp, &[args, &["--json"]].concat());
    let body = serde_json::from_slice(&out.stdout).unwrap_or(Value::Null);
    (out.status.code().unwrap_or(-1), body)
}

fn put(root: &Path, rel: &str, body: &str) -> PathBuf {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(&p, body).unwrap();
    p
}

fn schema(name: &str, body: &str) -> String {
    format!(
        r#"{{"$schema":"https://json-schema.org/draft/2020-12/schema","$id":"https://schemas.inorbit.hr/iohr/{name}/v0.json",{body}}}"#
    )
}

/// Every file of `root`, with its bytes, in name order: what a run could change.
fn snapshot(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut all = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else {
                let b = fs::read(&p).unwrap();
                all.push((p, b));
            }
        }
    }
    all.sort();
    all
}

#[test]
fn the_template_example_is_valid_and_off() {
    let repo =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("templates/project/base/dot-inorbit/monitors");
    let (state, tmp) = (Dir::new(), Dir::new());
    let (code, body) = json_of(&repo, state.path(), tmp.path(), &["check"]);
    assert_eq!(code, 0, "{body}");
    assert_eq!(body["problems"], json!([]));
    assert_eq!(body["warnings"], json!([]));
    assert_eq!(body["monitors"], json!(["example-health"]));
    // Off by default: a run lists it and does not run it.
    let (code, body) = json_of(&repo, state.path(), tmp.path(), &["run"]);
    assert_eq!(code, 0, "{body}");
    assert_eq!(body["ran"][0]["verdict"], "skip");
}

#[test]
fn a_project_with_schemas_checks_every_route_and_cli_step_names_its_shape() {
    // The neutral form of Eye's own repo test: every JSON route names a shape and every
    // shape, request and cli step schema is a file of the project.
    let proj = Dir::new();
    let dir = proj.path().join(".inorbit/monitors");
    put(
        proj.path(),
        "schemas/api/health/v0.json",
        &schema("api/health", r#""type":"object""#),
    );
    put(
        proj.path(),
        "schemas/cli/app/v0.json",
        &schema(
            "cli/app",
            r#""type":"object","$defs":{"request":{"type":"array"}}"#,
        ),
    );
    put(
        proj.path(),
        ".inorbit/monitors/app.toml",
        "[[check]]\nname = \"app-api\"\ntarget = \"http://127.0.0.1:7171/api/health\"\nevery = \"60s\"\nexpect = { status = 200, headers = { content-type = \"application/json\" }, shape = \"schemas/api/health/v0.json\" }\n\n[[check]]\nname = \"app-cli\"\ntarget = { cli = \"app\" }\nargv = [\"--json\"]\nexpect = { shape = \"schemas/cli/app/v0.json\", request = \"schemas/cli/app/v0.json#/$defs/request\" }\n",
    );
    let (state, tmp) = (Dir::new(), Dir::new());
    let (code, body) = json_of(&dir, state.path(), tmp.path(), &["check"]);
    assert_eq!(code, 0, "{body}");
    assert_eq!(body["warnings"], json!([]));
    assert_eq!(body["monitors"], json!(["app-api", "app-cli"]));
    let (_, shown) = json_of(&dir, state.path(), tmp.path(), &["show", "app-api"]);
    let shape = shown["monitor"]["definition"]["expect"]["shape"]
        .as_str()
        .unwrap();
    assert!(proj.path().join(shape).is_file(), "{shape}");
}

const CLI_OK: &str = "[[check]]\nname = \"version\"\nlabel = \"the binary reports its version\"\ntarget = { cli = \"app\" }\nargv = [\"--version\"]\nexpect = { max_ms = 20000 }\n\n[[check]]\nname = \"chain\"\ntarget = { cli = \"app\" }\nexpect = { max_ms = 20000 }\n[[check.steps]]\nargv = [\"--version\"]\n[[check.steps]]\nargv = [\"no-such-command\"]\nexpect = { exit = 2 }\n";

#[test]
fn every_cli_monitor_passes_and_leaves_nothing_behind() {
    let proj = Dir::new();
    let dir = proj.path().join(".inorbit/monitors");
    put(proj.path(), ".inorbit/monitors/cli.toml", CLI_OK);
    put(proj.path(), "atlas/scrum/board.json", "{}");
    let (state, tmp) = (Dir::new(), Dir::new());
    let before = snapshot(proj.path());
    let (code, body) = json_of(&dir, state.path(), tmp.path(), &["run", "--surface", "cli"]);
    let ran = body["ran"].as_array().unwrap();
    let failed: Vec<&Value> = ran.iter().filter(|r| r["verdict"] != "pass").collect();
    assert!(failed.is_empty(), "{failed:#?}");
    assert_eq!((code, ran.len()), (0, 2));
    assert_eq!(
        tmp.left(),
        Vec::<String>::new(),
        "the throwaway root is not empty"
    );
    assert!(
        before == snapshot(proj.path()),
        "a cli check changed the project"
    );
}

#[test]
fn a_broken_cli_check_fails_by_place_and_type_and_still_cleans_up() {
    let proj = Dir::new();
    let dir = proj.path().join(".inorbit/monitors");
    put(
        proj.path(),
        "schemas/cli/check/v0.json",
        &schema("cli/check", r#""type":"object","required":["nope"]"#),
    );
    put(
        proj.path(),
        "schemas/cli/req/v0.json",
        &schema(
            "cli/req",
            r#""$defs":{"request":{"type":"array","prefixItems":[{"const":"first"}]}}"#,
        ),
    );
    let check = |name: &str, rest: &str| {
        format!("[[check]]\nname = \"{name}\"\ntarget = {{ cli = \"app\" }}\n{rest}\n")
    };
    let file = [
        check("wrong-exit", "argv = [\"no-such-command\"]"),
        check("wrong-shape", "argv = [\"--json\", \"monitors\", \"--dir\", \"{monitors}\", \"check\"]\nexpect = { shape = \"schemas/cli/check/v0.json\" }"),
        check("wrong-request", "argv = [\"--version\"]\nexpect = { request = \"schemas/cli/req/v0.json#/$defs/request\" }"),
        check("second-step", "expect = { max_ms = 20000 }\n[[check.steps]]\nargv = [\"--version\"]\n[[check.steps]]\nargv = [\"--version\"]\nexpect = { exit = 3 }"),
    ]
    .concat();
    put(proj.path(), ".inorbit/monitors/cli.toml", &file);
    let (state, tmp) = (Dir::new(), Dir::new());
    let (code, body) = json_of(&dir, state.path(), tmp.path(), &["run"]);
    assert_eq!(code, 1, "{body}");
    let why = |name: &str| {
        let r = body["ran"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["name"] == name)
            .unwrap();
        assert_eq!(r["verdict"], "fail", "{r}");
        r["why"].as_str().unwrap().to_owned()
    };
    assert_eq!(why("wrong-exit"), "step 1 /exit: expected 0, found 2");
    assert_eq!(
        why("wrong-shape"),
        "shape step 1 /nope: expected a field, found none"
    );
    assert!(
        why("wrong-request").starts_with("request step 1 /0: expected "),
        "{}",
        why("wrong-request")
    );
    assert_eq!(why("second-step"), "step 2 /exit: expected 3, found 0");
    for r in body["ran"].as_array().unwrap() {
        let w = r["why"].as_str().unwrap_or_default();
        assert!(
            !w.contains(&*tmp.path().to_string_lossy()) && !w.contains("monitors/cli.toml"),
            "a path in {w}"
        );
    }
    assert_eq!(
        tmp.left(),
        Vec::<String>::new(),
        "the throwaway root is not empty after failures and a timeout"
    );
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one story told in order, as Eye's test was"
)]
fn list_show_history_silence_and_delete_work_on_a_copy() {
    let proj = Dir::new();
    let state = Dir::new();
    let tmp = Dir::new();
    let dir = proj.path().join(".inorbit/monitors");
    let file = "# The file's own note.\n\n[[check]]\nname = \"help\"\nlabel = \"the binary prints its help\"\ntarget = { cli = \"app\" }\nargv = [\"monitors\", \"--help\"]\n\n# Nothing listens on port 1.\n[[check]]\nname = \"dead\"\nlabel = \"a port nothing serves\"\ntarget = \"127.0.0.1:1\"\nevery = \"5m\"\n\n[[check]]\nname = \"sleepy\"\ntarget = \"127.0.0.1:2\"\nevery = \"5m\"\nenabled = false\n";
    let path = put(proj.path(), ".inorbit/monitors/app.toml", file);
    let iohr = |args: &[&str]| json_of(&dir, state.path(), tmp.path(), args);
    let row = |body: &Value, name: &str| {
        body["ran"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["name"] == name)
            .cloned()
            .unwrap()
    };

    // A run: the dead port fails with a hint, the one off by default is listed, not run.
    let (code, body) = iohr(&["run"]);
    assert_eq!(code, 1, "{body}");
    let dead = row(&body, "dead");
    assert_eq!(
        (
            dead["verdict"].as_str(),
            dead["label"].as_str(),
            dead["target"].as_str()
        ),
        (
            Some("fail"),
            Some("a port nothing serves"),
            Some("tcp 127.0.0.1:1")
        )
    );
    assert!(
        dead["hint"]
            .as_str()
            .unwrap()
            .starts_with("nothing listens on that port"),
        "{dead}"
    );
    let sleepy = row(&body, "sleepy");
    assert_eq!(
        (sleepy["verdict"].as_str(), sleepy["state"].as_str()),
        (Some("skip"), Some("off"))
    );
    assert_eq!(row(&body, "help")["verdict"], "pass");

    // Silenced for 2h, it is not run and the run is green; named filters and unknown names.
    let (code, body) = iohr(&["disable", "dead", "--for", "2h", "--reason", "port moved"]);
    assert_eq!(
        (code, body["state"].as_str(), body["reason"].as_str()),
        (0, Some("silenced"), Some("port moved")),
        "{body}"
    );
    assert!(body["until"].as_str().unwrap().ends_with('Z'));
    let (code, body) = iohr(&["run"]);
    assert_eq!(
        (code, row(&body, "dead")["state"].as_str()),
        (0, Some("silenced")),
        "{body}"
    );
    assert_eq!(iohr(&["run", "help"]).1["ran"].as_array().unwrap().len(), 1);
    assert_eq!(iohr(&["run", "nope"]).0, 2);
    assert_eq!(iohr(&["run", "--tag", "nokey"]).0, 2);

    // list, show and history read the definitions, the state and the log.
    let (code, body) = iohr(&["list"]);
    assert_eq!((code, body["monitors"].as_array().unwrap().len()), (0, 3));
    let (_, body) = iohr(&["list", "--off"]);
    let off: Vec<&str> = body["monitors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["name"].as_str().unwrap())
        .collect();
    assert_eq!(off, ["dead", "sleepy"]);
    let (_, body) = iohr(&["list", "--failing"]);
    assert_eq!(
        body["monitors"].as_array().unwrap().len(),
        1,
        "dead failed on its last run"
    );
    let (code, body) = iohr(&["get", "dead"]);
    assert_eq!(code, 0);
    let m = &body["monitor"];
    assert_eq!(
        (m["file"].as_str(), m["line"].as_u64(), m["checks"].as_str()),
        (
            Some("app.toml"),
            Some(10),
            Some("tcp 127.0.0.1:1 accepts a connection")
        ),
        "{body}"
    );
    assert_eq!(m["definition"]["every"], "5m");
    assert_eq!(
        (
            body["history"]["runs"].as_u64(),
            body["history"]["recent"][0]["verdict"].as_str()
        ),
        (Some(1), Some("fail"))
    );
    assert_eq!(iohr(&["show", "nope"]).0, 2);
    let (_, body) = iohr(&["history"]);
    let help = body["monitors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["name"] == "help")
        .unwrap();
    assert_eq!(
        (help["runs"].as_u64(), help["passed"].as_u64()),
        (Some(3), Some(3)),
        "{body}"
    );
    assert!(
        fs::read_to_string(state.path().join("runs.jsonl"))
            .unwrap()
            .lines()
            .all(|l| !l.contains("Usage")),
        "no bodies in the log"
    );

    // enable ends a silence and turns on one its file has off; the file is not edited.
    assert_eq!(iohr(&["enable", "dead"]).1["state"], "on");
    assert_eq!(iohr(&["enable", "sleepy"]).1["state"], "on");
    assert_eq!(
        iohr(&["list", "--off"]).1["monitors"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), file);
    // A bare disable puts one its file has off back to off, not to an endless silence.
    assert_eq!(iohr(&["disable", "sleepy"]).1["state"], "off");
    assert_eq!(
        fs::read_to_string(state.path().join("silences.json"))
            .unwrap()
            .trim(),
        "{}"
    );

    // delete prints what it would cut and changes nothing; with --yes it cuts only that entry.
    let (code, body) = iohr(&["delete", "dead"]);
    assert_eq!(
        (code, body["done"].as_bool(), body["lines"].clone()),
        (0, Some(false), json!([9, 15])),
        "{body}"
    );
    assert!(
        body["removed"]
            .as_str()
            .unwrap()
            .starts_with("# Nothing listens on port 1.\n[[check]]")
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), file);
    assert_eq!(iohr(&["delete", "dead", "--yes"]).1["done"], true);
    let after = fs::read_to_string(&path).unwrap();
    assert!(
        !after.contains("dead")
            && after.starts_with("# The file's own note.")
            && after.contains("name = \"sleepy\""),
        "{after}"
    );
    let (code, body) = iohr(&["check"]);
    assert_eq!(
        (code, body["monitors"].clone()),
        (0, json!(["help", "sleepy"]))
    );
}

/// A stand-in for the agent: `agent checks lint --file <f>` passes when the file has a check.
#[cfg(unix)]
fn fake_agent(at: &Path) -> String {
    use std::os::unix::fs::PermissionsExt;
    let p = at.join("fake-agent.sh");
    fs::write(&p, "#!/bin/sh\n[ \"$1 $2 $3 $4\" = \"agent checks lint --file\" ] && grep -q '^\\[\\[check\\]\\]' \"$5\"\n").unwrap();
    fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
    p.to_string_lossy().into_owned()
}

#[cfg(unix)]
#[test]
fn a_lint_that_fails_leaves_the_checks_file_as_it_was() {
    let proj = Dir::new();
    let dir = proj.path().join(".inorbit/monitors");
    put(
        proj.path(),
        ".inorbit/monitors/web.toml",
        "[[check]]\nname = \"web-api\"\ntarget = \"http://127.0.0.1:7171/h\"\nevery = \"60s\"\n",
    );
    let (state, tmp, home) = (Dir::new(), Dir::new(), Dir::new());
    let file = put(home.path(), "checks.toml", "# mine\n");
    let f = file.to_string_lossy().into_owned();
    let out = monitors_with(
        &dir,
        state.path(),
        tmp.path(),
        &["install", "--file", &f, "--block", "web"],
        &[("IOHR_MONITORS_LINT", "false")],
    );
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("the checks file is unchanged"));
    assert_eq!(fs::read_to_string(&file).unwrap(), "# mine\n");
    assert_eq!(
        home.left(),
        ["checks.toml"],
        "no candidate or backup is left"
    );
}

#[cfg(unix)]
#[test]
fn install_writes_the_block_the_agent_lints_and_refuses_cli_files() {
    let proj = Dir::new();
    let dir = proj.path().join(".inorbit/monitors");
    put(
        proj.path(),
        ".inorbit/monitors/web.toml",
        "[[check]]\nname = \"web-api\"\ntarget = \"http://127.0.0.1:7171/api/health\"\nevery = \"60s\"\nexpect = { status = 200 }\n\n[[check]]\nname = \"web-off\"\ntarget = \"127.0.0.1:2\"\nevery = \"5m\"\nenabled = false\n",
    );
    put(proj.path(), ".inorbit/monitors/cli.toml", CLI_OK);
    let (state, tmp, home) = (Dir::new(), Dir::new(), Dir::new());
    let file = home.path().join("checks.toml");
    let file_arg = file.to_string_lossy().into_owned();
    let lint = fake_agent(state.path());
    let monitors = |d: &Path, s: &Path, t: &Path, a: &[&str]| {
        monitors_with(d, s, t, a, &[("IOHR_MONITORS_LINT", &lint)])
    };
    let json_of = |d: &Path, s: &Path, t: &Path, a: &[&str]| {
        let out = monitors(d, s, t, &[a, &["--json"]].concat());
        (
            out.status.code().unwrap_or(-1),
            serde_json::from_slice(&out.stdout).unwrap_or(Value::Null),
        )
    };
    // A named cli file is refused; nothing is written.
    let out = monitors(
        &dir,
        state.path(),
        tmp.path(),
        &["install", "cli", "--file", &file_arg],
    );
    assert_eq!(
        out.status.code(),
        Some(2),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stderr).contains("never installed"));
    assert!(!file.exists());
    // A dry run lints and writes nothing.
    let (code, body) = json_of(
        &dir,
        state.path(),
        tmp.path(),
        &[
            "install",
            "--file",
            &file_arg,
            "--block",
            "web",
            "--dry-run",
        ],
    );
    assert_eq!(
        (code, body["dry_run"].clone(), body["entries"].clone()),
        (0, json!(true), json!(1)),
        "{body}"
    );
    assert!(!file.exists());
    // The install: the cli checks and the one with enabled = false are left out and counted.
    let (code, body) = json_of(
        &dir,
        state.path(),
        tmp.path(),
        &["install", "--file", &file_arg, "--block", "web"],
    );
    assert_eq!(code, 0, "{body}");
    assert_eq!(
        (body["cli_left_out"].clone(), body["off_left_out"].clone()),
        (json!(2), json!(1))
    );
    let text = fs::read_to_string(&file).unwrap();
    assert!(
        text.starts_with("# BEGIN web (from project web .inorbit/monitors)\n")
            && text.contains("# END web"),
        "{text}"
    );
    assert!(
        text.contains("web-api") && !text.contains("web-off") && !text.contains("version"),
        "{text}"
    );
    // The text the agent lints is the same; the real agent's verdict is part of the preview
    // proof, on a machine that has it.
    assert_eq!(text.matches("[[check]]").count(), 1, "{text}");
    // Again: one block, a backup of the old file.
    let (code, body) = json_of(
        &dir,
        state.path(),
        tmp.path(),
        &["install", "--file", &file_arg, "--block", "web"],
    );
    assert_eq!(code, 0, "{body}");
    assert_eq!(fs::read_to_string(&file).unwrap(), text);
    let backup = body["backup"].as_str().unwrap();
    assert!(
        backup.contains("checks.toml.bak-") && fs::read_to_string(backup).unwrap() == text,
        "{body}"
    );
}
