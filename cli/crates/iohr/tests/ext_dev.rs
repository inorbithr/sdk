//! The local extension loop end to end (PRD 0084 slice 1): `iohr ext init` for every
//! language passes `iohr ext lint`; `iohr ext dev` runs the shell scaffold to a heartbeat
//! with the real token channel, refuses and logs a scope the manifest does not declare,
//! hides `IOHR_TOKEN*` and `iohr` from the child and never touches the installed
//! extensions; `--test` fails when the tests write outside `$STATE` and `$CACHE`; each
//! lint rule fails on its own, and `--json` carries the same preview as the text.

#![cfg(unix)]
#![allow(
    clippy::unwrap_used,
    reason = "test helpers fail the test by panicking"
)]

use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

/// A JWT-shaped token the profile hands out; never inspected beyond its claims.
fn token(scopes: &[&str]) -> String {
    use base64::Engine as _;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;

    let enc = |v: serde_json::Value| URL_SAFE_NO_PAD.encode(v.to_string());
    format!(
        "{}.{}.TOKENSIGNATUREMARKER",
        enc(serde_json::json!({"alg": "RS256"})),
        enc(serde_json::json!({
            "sub": "ak_tok1", "aud": ["iohr-api"], "exp": 4_102_444_800_i64,
            "scp": scopes, "org": "acc_test1"
        }))
    )
}

struct Box_ {
    dir: tempfile::TempDir,
}

impl Box_ {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    fn home(&self) -> PathBuf {
        self.dir.path().join("home")
    }

    fn state(&self, name: &str) -> PathBuf {
        self.home()
            .join(".inorbit/ext-dev")
            .join(name)
            .join("state")
    }

    /// `iohr` with nothing of this machine's but PATH, a HOME of its own, and a token
    /// that holds identity:read.
    fn cmd(&self, cwd: &Path, args: &[&str]) -> Command {
        std::fs::create_dir_all(self.home()).unwrap();
        let mut c = Command::new(env!("CARGO_BIN_EXE_iohr"));
        c.args(args)
            .env_clear()
            .env("IOHR_BASE_URL", "http://127.0.0.1:9")
            .env("IOHR_CONFIG_DIR", self.dir.path().join("config"))
            .env("IOHR_DATA_DIR", self.dir.path().join("data"))
            .env("HOME", self.home())
            .env("IOHR_TOKEN", token(&["identity:read"]))
            .current_dir(cwd);
        for k in ["PATH", "TMPDIR"] {
            if let Some(v) = std::env::var_os(k) {
                c.env(k, v);
            }
        }
        c
    }

    fn run(&self, cwd: &Path, args: &[&str]) -> Output {
        self.cmd(cwd, args).stdin(Stdio::null()).output().unwrap()
    }

    /// `iohr ext init inorbit/NAME --lang LANG` in the box; returns the folder.
    fn init(&self, name: &str, lang: &str) -> PathBuf {
        let o = self.run(
            self.dir.path(),
            &["ext", "init", &format!("inorbit/{name}"), "--lang", lang],
        );
        assert_eq!(code(&o), 0, "{}", text(&o));
        self.dir.path().join(name)
    }

    /// Nothing installed, no lock: `iohr ext dev` never touched them.
    fn nothing_installed(&self) {
        assert!(
            !self.dir.path().join("data").exists(),
            "iohr ext dev wrote the extensions' data directory"
        );
        let o = self.run(self.dir.path(), &["ext", "list", "--json"]);
        assert_eq!(
            String::from_utf8_lossy(&o.stdout).trim(),
            "[]",
            "{}",
            text(&o)
        );
    }
}

fn text(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

fn code(o: &Output) -> i32 {
    o.status.code().unwrap()
}

fn have(program: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(program).is_file()))
}

fn edit(file: &Path, from: &str, to: &str) {
    let t = std::fs::read_to_string(file).unwrap();
    assert!(t.contains(from), "{} has no {from:?}", file.display());
    std::fs::write(file, t.replace(from, to)).unwrap();
}

fn heartbeat(state: &Path) -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(state.join("heartbeat.json")).unwrap()).unwrap()
}

#[test]
fn every_language_scaffolds_a_folder_that_passes_the_lint() {
    let b = Box_::new();
    for (name, lang) in [
        ("hello-sh", "shell"),
        ("hello-py", "python"),
        ("hello-rs", "rust"),
    ] {
        let dir = b.init(name, lang);
        let o = b.run(&dir, &["ext", "lint"]);
        assert_eq!(code(&o), 0, "{lang}: {}", text(&o));
        assert!(dir.join(".github/workflows/release.yml").is_file());
        assert!(dir.join("README.md").is_file());
        assert!(dir.join("tests").is_dir());
        let o = b.run(b.dir.path(), &["ext", "lint", name, "--preview"]);
        assert!(
            String::from_utf8_lossy(&o.stdout)
                .contains("May ask iohr for API tokens with identity:read, and nothing else."),
            "{}",
            text(&o)
        );
    }
    // A name holding tbd is refused and nothing is written.
    let o = b.run(
        b.dir.path(),
        &["ext", "init", "inorbit/hello-tbd", "--lang", "shell"],
    );
    assert_eq!(code(&o), 2, "{}", text(&o));
    assert!(text(&o).contains("tbd"), "{}", text(&o));
    assert!(!b.dir.path().join("hello-tbd").exists());
    // An existing folder with files is left alone.
    let o = b.run(
        b.dir.path(),
        &["ext", "init", "inorbit/hello-sh", "--lang", "shell"],
    );
    assert_eq!(code(&o), 2, "{}", text(&o));
}

/// The shell scaffold needs no toolchain: from the folder `iohr ext init` writes to a
/// heartbeat marked dev, with the token granted, well within 60 s, and stopped like a
/// Ctrl-C at a terminal stops it.
#[test]
fn dev_runs_the_shell_scaffold_to_a_heartbeat() {
    let b = Box_::new();
    let dir = b.init("hello", "shell");
    let start = Instant::now();
    let child = b
        .cmd(&dir, &["ext", "dev"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()
        .unwrap();
    let state = b.state("hello");
    while !state.join("heartbeat.json").exists()
        || heartbeat(&state).get("dev") != Some(&serde_json::Value::Bool(true))
    {
        assert!(
            start.elapsed() < Duration::from_secs(60),
            "no heartbeat within 60 s"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    let beat = heartbeat(&state);
    assert_eq!(beat["state"], "ok", "{beat}");
    assert_eq!(beat["token"], "granted", "{beat}");
    assert_eq!(beat["dev"], true, "{beat}");
    // Ctrl-C at a terminal reaches the whole group.
    let pgid = child.id().to_string();
    Command::new("kill")
        .args(["-INT", "--", &format!("-{pgid}")])
        .status()
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        start.elapsed() < Duration::from_secs(30),
        "a Ctrl-C took {:?} to stop it",
        start.elapsed()
    );
    let log = text(&out);
    assert!(log.contains("token granted: identity:read"), "{log}");
    assert!(log.contains(" out  heartbeat at "), "{log}");
    assert!(!log.contains("TOKENSIGNATUREMARKER"), "{log}");
    b.nothing_installed();
}

#[test]
fn a_scope_outside_the_manifest_is_refused_and_logged() {
    let b = Box_::new();
    let dir = b.init("hello", "shell");
    edit(
        &dir.join("bin/hello"),
        r#"{"scopes":["identity:read"]}"#,
        r#"{"scopes":["tokens:write"]}"#,
    );
    let o = b
        .cmd(&dir, &["ext", "dev"])
        .env("IOHR_EXT_ONCE", "1")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(code(&o), 0, "{}", text(&o));
    let log = String::from_utf8_lossy(&o.stderr);
    assert!(
        log.contains(
            "token REFUSED: tokens:write: tokens:write not declared in permissions.scopes"
        ),
        "{log}"
    );
    assert_eq!(heartbeat(&b.state("hello"))["token"], "refused");
}

/// The child never sees the person's tokens, the development secrets' variables or an
/// `iohr` it could ask for a full-scope token; it does get its declared secret.
#[test]
fn the_child_gets_no_credentials_and_no_iohr() {
    let b = Box_::new();
    let dir = b.init("hello", "shell");
    edit(
        &dir.join("extension.toml"),
        "secrets = []\n",
        "secrets = [{ name = \"github_token\", ref = \"github-token\", env = \"GITHUB_TOKEN\" }]\n",
    );
    edit(
        &dir.join("bin/hello"),
        "log \"$ID $VERSION starting; state in $STATE\"",
        "env > \"$STATE/env.txt\"; command -v iohr > \"$STATE/which.txt\" || echo none > \"$STATE/which.txt\"",
    );
    // A folder with another iohr in it, besides the one running.
    let other = b.dir.path().join("other-bin");
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(other.join("iohr"), "#!/bin/sh\necho owned\n").unwrap();
    std::fs::set_permissions(
        other.join("iohr"),
        std::os::unix::fs::PermissionsExt::from_mode(0o755),
    )
    .unwrap();
    let me = PathBuf::from(env!("CARGO_BIN_EXE_iohr"));
    let path = std::env::join_paths(
        [other.clone(), me.parent().unwrap().to_path_buf()]
            .into_iter()
            .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();
    let o = b
        .cmd(&dir, &["ext", "dev"])
        .env("PATH", path)
        .env("IOHR_TOKEN_CI", token(&["identity:read"]))
        .env("INORBIT_TOKEN", "sdk-token-value")
        .env("IOHR_EXT_SECRET_GITHUB_TOKEN", "the-dev-secret")
        .env("IOHR_EXT_ONCE", "1")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(code(&o), 0, "{}", text(&o));
    let state = b.state("hello");
    let env = std::fs::read_to_string(state.join("env.txt")).unwrap();
    assert!(!env.contains("TOKENSIGNATUREMARKER"), "{env}");
    assert!(!env.contains("sdk-token-value"), "{env}");
    for var in [
        "IOHR_TOKEN=",
        "IOHR_TOKEN_CI=",
        "INORBIT_TOKEN=",
        "IOHR_EXT_SECRET_",
    ] {
        assert!(
            !env.lines().any(|l| l.starts_with(var)),
            "{var} reached the extension: {env}"
        );
    }
    assert!(env.contains("GITHUB_TOKEN=the-dev-secret"), "{env}");
    assert!(env.contains("IOHR_EXT_DEV=1"), "{env}");
    assert!(env.contains("IOHR_EXT_TOKEN_SOCKET="), "{env}");
    assert_eq!(
        std::fs::read_to_string(state.join("which.txt"))
            .unwrap()
            .trim(),
        "none",
        "iohr is on the extension's PATH"
    );
    let log = String::from_utf8_lossy(&o.stderr);
    assert!(
        log.contains(&format!(
            "left {} out of the extension's PATH",
            other.display()
        )),
        "{log}"
    );
    b.nothing_installed();
}

#[test]
fn a_missing_secret_says_how_to_give_it() {
    let b = Box_::new();
    let dir = b.init("hello", "shell");
    edit(
        &dir.join("extension.toml"),
        "secrets = []\n",
        "secrets = [{ name = \"github_token\", ref = \"github-token\", env = \"GITHUB_TOKEN\" }]\n",
    );
    let o = b
        .cmd(&dir, &["ext", "dev"])
        .env("IOHR_EXT_ONCE", "1")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(code(&o), 2, "{}", text(&o));
    assert!(
        text(&o).contains("IOHR_EXT_SECRET_GITHUB_TOKEN"),
        "{}",
        text(&o)
    );
}

#[test]
fn the_self_test_passes_and_fails_when_it_writes_outside_state_and_cache() {
    let b = Box_::new();
    let dir = b.init("hello", "shell");
    let o = b.run(&dir, &["ext", "dev", "--test"]);
    assert_eq!(code(&o), 0, "{}", text(&o));
    assert!(text(&o).contains("selftest ok"), "{}", text(&o));
    assert!(
        text(&o).contains("nothing written outside $STATE and $CACHE"),
        "{}",
        text(&o)
    );

    // Into its own folder: the self-test's own check is bypassed, iohr's is not.
    let leak = dir.join("tests/zz-leak.sh");
    std::fs::write(
        &leak,
        "#!/bin/sh\necho x > \"$(dirname \"$0\")/../leak.txt\"\n",
    )
    .unwrap();
    let o = b.run(&dir, &["ext", "dev", "--test"]);
    assert_eq!(code(&o), 1, "{}", text(&o));
    assert!(
        text(&o).contains("written outside $STATE and $CACHE: leak.txt"),
        "{}",
        text(&o)
    );
    std::fs::remove_file(dir.join("leak.txt")).unwrap();

    // Into HOME.
    std::fs::write(&leak, "#!/bin/sh\necho x > \"$HOME/leak.txt\"\n").unwrap();
    let o = b.run(&dir, &["ext", "dev", "--test"]);
    assert_eq!(code(&o), 1, "{}", text(&o));
    assert!(text(&o).contains("$HOME/leak.txt"), "{}", text(&o));

    // A failing test fails the run with its code.
    std::fs::write(&leak, "#!/bin/sh\nexit 3\n").unwrap();
    let o = b.run(&dir, &["ext", "dev", "--test"]);
    assert_eq!(code(&o), 3, "{}", text(&o));
    b.nothing_installed();
}

#[test]
fn the_python_scaffold_runs_and_tests() {
    if !have("python3") {
        eprintln!("python3 is not on PATH; skipped");
        return;
    }
    let b = Box_::new();
    let dir = b.init("hello-py", "python");
    let o = b
        .cmd(&dir, &["ext", "dev"])
        .env("IOHR_EXT_ONCE", "1")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(code(&o), 0, "{}", text(&o));
    let beat = heartbeat(&b.state("hello-py"));
    assert_eq!(
        (beat["token"].as_str(), beat["dev"].as_bool()),
        (Some("granted"), Some(true))
    );
    let o = b.run(&dir, &["ext", "dev", "--test"]);
    assert_eq!(code(&o), 0, "{}", text(&o));
    // The self-test asks for an undeclared scope on purpose: refused and logged.
    assert!(
        text(&o).contains("token REFUSED: tokens:write"),
        "{}",
        text(&o)
    );
}

/// The Rust scaffold against this repository's `inorbithr` (it is not published with
/// `ExtSocketToken` yet), built by cargo: slow, so only with `IOHR_TEST_EXT_RUST=1`.
#[test]
fn the_rust_scaffold_runs_and_tests() {
    if std::env::var_os("IOHR_TEST_EXT_RUST").is_none() || !have("cargo") {
        eprintln!("set IOHR_TEST_EXT_RUST=1 to build the Rust scaffold; skipped");
        return;
    }
    let b = Box_::new();
    let dir = b.init("hello-rs", "rust");
    let sdk = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../rust");
    std::fs::create_dir_all(dir.join(".cargo")).unwrap();
    std::fs::write(
        dir.join(".cargo/config.toml"),
        format!(
            "[patch.crates-io]\ninorbithr = {{ path = {:?} }}\n",
            sdk.canonicalize().unwrap()
        ),
    )
    .unwrap();
    let real_home = std::env::var_os("HOME").unwrap();
    let cargo_env = |c: &mut Command| {
        for (k, v) in [("CARGO_HOME", ".cargo"), ("RUSTUP_HOME", ".rustup")] {
            let v = std::env::var_os(k).unwrap_or_else(|| Path::new(&real_home).join(v).into());
            c.env(k, v);
        }
        if let Some(t) = std::env::var_os("CARGO_TARGET_DIR") {
            c.env("CARGO_TARGET_DIR", t);
        }
    };
    let mut c = b.cmd(&dir, &["ext", "dev"]);
    cargo_env(&mut c);
    let o = c
        .env("IOHR_EXT_ONCE", "1")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(code(&o), 0, "{}", text(&o));
    let beat = heartbeat(&b.state("hello-rs"));
    assert_eq!(
        (beat["token"].as_str(), beat["dev"].as_bool()),
        (Some("granted"), Some(true))
    );
    let mut c = b.cmd(&dir, &["ext", "dev", "--test"]);
    cargo_env(&mut c);
    let o = c.stdin(Stdio::null()).output().unwrap();
    assert_eq!(code(&o), 0, "{}", text(&o));
}

#[test]
fn each_lint_failure_exits_non_zero_and_json_matches_the_preview() {
    let b = Box_::new();
    let dir = b.init("hello", "shell");
    let manifest = dir.join("extension.toml");
    let good = std::fs::read_to_string(&manifest).unwrap();

    let o = b.run(&dir, &["ext", "lint", "--preview"]);
    assert_eq!(code(&o), 0, "{}", text(&o));
    let lines: Vec<String> = String::from_utf8_lossy(&o.stdout)
        .lines()
        .map(str::to_owned)
        .collect();
    let o = b.run(&dir, &["ext", "lint", "--json"]);
    assert_eq!(code(&o), 0, "{}", text(&o));
    let json: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(json["ok"], true);
    assert_eq!(json["preview"]["lines"], serde_json::json!(lines));
    assert_eq!(
        json["preview"]["scopes"],
        serde_json::json!(["identity:read"])
    );
    assert_eq!(json["config"]["name"], "hello");
    assert_eq!(
        json["config"]["scopes"],
        serde_json::json!(["identity:read"])
    );

    for (from, to, rule) in [
        (
            "description = \"",
            "description = \"ghp_0123456789abcdefghijklmnopqrstuvwxyz \"\nignored = \"",
            "secret_value",
        ),
        (
            "network = []",
            "network = [\"postgres://app:hunter2@db.internal:5432\"]",
            "secret_value",
        ),
        (
            "leaves = [\"extension.health\"]",
            "leaves = [\"request.body\"]",
            "data_class",
        ),
        (
            "kinds = [\"desktop\", \"host\"]",
            "kinds = [\"phone\"]",
            "target_kind",
        ),
        (
            "id = \"inorbit/hello\"",
            "id = \"inorbit/hello-tbd\"",
            "tbd_name",
        ),
        ("api = ", "colour = \"red\"\napi = ", "unknown_key"),
        ("exec = []", "exec = []\nshell = true", "unknown_key"),
    ] {
        std::fs::write(&manifest, good.replace(from, to)).unwrap();
        let o = b.run(&dir, &["ext", "lint"]);
        assert_eq!(code(&o), 1, "{rule}: {}", text(&o));
        assert!(
            text(&o).contains(&format!("error[{rule}]")),
            "{rule}: {}",
            text(&o)
        );
        let o = b.run(&dir, &["ext", "lint", "--json"]);
        assert_eq!(code(&o), 1, "{rule}");
        let json: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
        assert_eq!(json["ok"], false);
        assert!(json["config"].is_null(), "no config from a failed lint");
        assert!(
            json["findings"]
                .as_array()
                .unwrap()
                .iter()
                .any(|f| f["rule"] == rule),
            "{rule}: {json}"
        );
        // And dev refuses to run it.
        let o = b
            .cmd(&dir, &["ext", "dev"])
            .env("IOHR_EXT_ONCE", "1")
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert_eq!(code(&o), 1, "{rule}: {}", text(&o));
    }
    assert!(!b.state("hello").join("heartbeat.json").exists());
}
