//! `iohr ask --role` and `iohr init --role` (RFC 0211): the binary with a private HOME, a fake
//! agent on the PATH that records its arguments, and a local API (port 0) for the account's
//! Roles and the job brief. No real agent starts, no browser opens, nothing of the machine
//! running the test is read.

#![cfg(unix)]

use std::error::Error;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

type Res = Result<(), Box<dyn Error>>;

const ROLES: &str = "/v1/coordinator/personalities";

/// A fake agent: records its arguments, one per line, in `$FAKE_OUT/<name>.args`.
fn fake_agent(bin: &Path, name: &str) -> Res {
    std::fs::create_dir_all(bin)?;
    let file = bin.join(name);
    std::fs::write(
        &file,
        format!("#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$FAKE_OUT/{name}.args\"\nexit 0\n"),
    )?;
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755))?;
    Ok(())
}

fn token() -> String {
    let enc = |v: Value| URL_SAFE_NO_PAD.encode(v.to_string());
    format!(
        "{}.{}.TOKENSIGNATUREMARKER",
        enc(json!({"alg": "RS256"})),
        enc(json!({
            "sub": "ak_tok1", "aud": ["iohr-api"], "exp": 4_102_444_800_i64,
            "scp": ["coordinator:read", "account:read"], "org": "acc_test1",
            "plan": "free", "key": "key_1"
        }))
    )
}

/// A private home with a project folder (`work/`, with `atlas/` so it is a project root).
struct Rig {
    home: tempfile::TempDir,
}

impl Rig {
    fn new() -> Result<Self, Box<dyn Error>> {
        let r = Self {
            home: tempfile::tempdir()?,
        };
        std::fs::create_dir_all(r.out())?;
        std::fs::create_dir_all(r.work().join("atlas/roles"))?;
        fake_agent(&r.bin(), "claude")?;
        Ok(r)
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
    /// A Role pack in the project's `atlas/roles/`.
    fn pack(&self, name: &str, model: Option<&str>, core: &str) -> Res {
        let model = model.map(|m| format!("model: {m}\n")).unwrap_or_default();
        std::fs::write(
            self.work().join(format!("atlas/roles/{name}.md")),
            format!(
                "---\nname: {name}\n{model}---\n# {name}\n\n## Core: always in the brief\n\n{core}\n\n## Recall: found by topic\n\n### Later\n\n- not in the brief\n"
            ),
        )?;
        Ok(())
    }
    fn config(&self, text: &str) -> Res {
        let dir = self.home.path().join("config");
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join("config.toml"), text)?;
        Ok(())
    }
    fn run(&self, base: Option<&str>, args: &[&str]) -> Result<Output, Box<dyn Error>> {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_iohr"));
        cmd.args(args)
            .current_dir(self.work())
            .env_clear()
            .env("PATH", self.bin())
            .env("IOHR_NO_BROWSER", "1")
            .env("IOHR_TOOLCHAIN_UPSTREAM", "off")
            .env("NO_COLOR", "1")
            .env("HOME", self.home.path())
            .env("USERPROFILE", self.home.path())
            .env("IOHR_CONFIG_DIR", self.home.path().join("config"))
            .env("FAKE_OUT", self.out())
            .stdin(Stdio::null());
        if let Some(b) = base {
            cmd.env("IOHR_BASE_URL", b).env("IOHR_TOKEN", token());
        }
        Ok(cmd.output()?)
    }
    /// Everything the fake agent received, as recorded (the system prompt spans lines).
    fn raw(&self) -> String {
        std::fs::read_to_string(self.out().join("claude.args")).unwrap_or_default()
    }
    /// The arguments the fake agent received, one per line.
    fn args(&self) -> Vec<String> {
        std::fs::read_to_string(self.out().join("claude.args"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }
}

fn text(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

/// The value after `flag` in a command line.
fn after<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.windows(2)
        .find(|w| w[0] == flag)
        .map(|w| w[1].as_str())
}

fn json_of(o: &Output) -> Result<Value, Box<dyn Error>> {
    Ok(serde_json::from_slice(&o.stdout).map_err(|e| format!("not json ({e}): {}", text(o)))?)
}

async fn me(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/v1/me"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "subject": "ak_tok1", "kind": "client", "org": "acc_test1",
            "scopes": ["coordinator:read"]
        })))
        .mount(server)
        .await;
}

async fn account_role(server: &MockServer, name: &str, model: &str, core: &str) {
    Mock::given(method("GET"))
        .and(path(format!("{ROLES}/{name}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "personality": {"name": name, "model": model},
            "core": core, "recall": "", "lines": 3
        })))
        .mount(server)
        .await;
}

#[test]
fn a_project_role_gives_its_model_and_its_core_to_the_agent_and_is_named_in_json() -> Res {
    let r = Rig::new()?;
    r.pack("shipper", Some("opus"), "- Ship small changes.")?;
    let o = r.run(
        None,
        &["--json", "ask", "--dry-run", "--role", "shipper", "q"],
    )?;
    assert!(o.status.success(), "{}", text(&o));
    let v = json_of(&o)?;
    let args: Vec<String> = serde_json::from_value(v["args"].clone())?;
    assert_eq!(after(&args, "--model"), Some("opus"), "{args:?}");
    let sys = after(&args, "--append-system-prompt").ok_or("no system prompt")?;
    assert!(sys.contains("Ship small changes."), "{sys}");
    assert!(
        !sys.contains("not in the brief"),
        "only the Core goes in: {sys}"
    );
    assert_eq!(v["role"]["name"], "shipper");
    assert_eq!(v["role"]["source"], "project");
    assert_eq!(v["role"]["brief"], "role");
    assert_eq!(v["role"]["model"], "opus");
    assert!(!v["role"].to_string().contains("Ship small changes."));
    Ok(())
}

#[test]
fn the_model_order_is_flag_then_config_then_role_then_default() -> Res {
    let r = Rig::new()?;
    r.pack("shipper", Some("opus"), "- Ship.")?;
    let model = |extra: &[&str]| -> Result<String, Box<dyn Error>> {
        let mut a = vec!["--json", "ask", "--dry-run", "--role", "shipper"];
        a.extend_from_slice(extra);
        a.push("q");
        let v = json_of(&r.run(None, &a)?)?;
        let args: Vec<String> = serde_json::from_value(v["args"].clone())?;
        Ok(after(&args, "--model").unwrap_or_default().to_owned())
    };
    // The Role's model when nothing else names one.
    assert_eq!(model(&[])?, "opus");
    // The config's model comes before the Role's.
    r.config("[ask]\nmodel = \"sonnet\"\n")?;
    assert_eq!(model(&[])?, "sonnet");
    // The flag comes before both.
    assert_eq!(model(&["--model", "haiku"])?, "haiku");
    Ok(())
}

#[test]
fn a_role_with_no_model_leaves_the_default_and_no_role_changes_nothing() -> Res {
    let r = Rig::new()?;
    r.pack("plain", None, "- Be plain.")?;
    let with = json_of(&r.run(
        None,
        &["--json", "ask", "--dry-run", "--role", "plain", "q"],
    )?)?;
    let without = json_of(&r.run(None, &["--json", "ask", "--dry-run", "q"])?)?;
    let a: Vec<String> = serde_json::from_value(with["args"].clone())?;
    let b: Vec<String> = serde_json::from_value(without["args"].clone())?;
    assert_eq!(after(&a, "--model"), after(&b, "--model"), "{a:?} {b:?}");
    assert!(after(&b, "--append-system-prompt").is_none(), "{b:?}");
    assert!(without.get("role").is_none(), "{without}");
    Ok(())
}

#[test]
fn a_built_in_role_works_with_no_pack_and_no_account() -> Res {
    let r = Rig::new()?;
    let o = r.run(
        None,
        &["--json", "ask", "--dry-run", "--role", "rust-engineer", "q"],
    )?;
    assert!(o.status.success(), "{}", text(&o));
    let v = json_of(&o)?;
    assert_eq!(v["role"]["source"], "catalogue");
    let args: Vec<String> = serde_json::from_value(v["args"].clone())?;
    assert!(after(&args, "--append-system-prompt").is_some(), "{args:?}");
    Ok(())
}

#[test]
fn an_unknown_role_or_a_bad_name_is_a_usage_error_and_nothing_starts() -> Res {
    let r = Rig::new()?;
    let o = r.run(None, &["ask", "--role", "no-such-role", "q"])?;
    assert_eq!(o.status.code(), Some(2), "{}", text(&o));
    assert!(text(&o).contains("no Role `no-such-role`"), "{}", text(&o));
    assert!(r.args().is_empty(), "the agent must not start");
    let o = r.run(None, &["ask", "--role", "../x", "q"])?;
    assert_eq!(o.status.code(), Some(2), "{}", text(&o));
    // A job needs a Role.
    let o = r.run(None, &["ask", "--job", "job_1", "q"])?;
    assert_eq!(o.status.code(), Some(2), "{}", text(&o));
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn a_role_only_the_account_has_is_found_there_after_the_project() -> Res {
    let server = MockServer::start().await;
    me(&server).await;
    account_role(&server, "auditor", "opus", "- Check every claim.").await;
    let r = Rig::new()?;
    let base = server.uri();
    let o = tokio::task::spawn_blocking({
        let (home, work) = (r.home.path().to_owned(), r.work());
        move || -> Result<Output, String> {
            run_at(
                &home,
                &work,
                &base,
                &["--json", "ask", "--role", "auditor", "q"],
            )
        }
    })
    .await??;
    assert!(o.status.success(), "{}", text(&o));
    let args = r.args();
    assert_eq!(after(&args, "--model"), Some("opus"), "{args:?}");
    assert!(
        args.iter().any(|a| a == "--append-system-prompt"),
        "{args:?}"
    );
    assert!(r.raw().contains("Check every claim."), "{}", r.raw());
    // A project pack of the same name wins over the account's.
    r.pack("auditor", Some("sonnet"), "- The project's way.")?;
    let base = server.uri();
    let o = tokio::task::spawn_blocking({
        let (home, work) = (r.home.path().to_owned(), r.work());
        move || {
            run_at(
                &home,
                &work,
                &base,
                &["--json", "ask", "--role", "auditor", "q"],
            )
        }
    })
    .await??;
    assert!(o.status.success(), "{}", text(&o));
    let args = r.args();
    assert_eq!(after(&args, "--model"), Some("sonnet"), "{args:?}");
    assert!(r.raw().contains("The project's way."), "{}", r.raw());
    assert!(!r.raw().contains("Check every claim."), "{}", r.raw());
    Ok(())
}

/// `Rig::run` for a thread that cannot hold the rig: the same environment.
fn run_at(home: &Path, work: &Path, base: &str, args: &[&str]) -> Result<Output, String> {
    Command::new(env!("CARGO_BIN_EXE_iohr"))
        .args(args)
        .current_dir(work)
        .env_clear()
        .env("PATH", home.join("bin"))
        .env("IOHR_NO_BROWSER", "1")
        .env("IOHR_TOOLCHAIN_UPSTREAM", "off")
        .env("NO_COLOR", "1")
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("IOHR_CONFIG_DIR", home.join("config"))
        .env("FAKE_OUT", home.join("out"))
        .env("IOHR_BASE_URL", base)
        .env("IOHR_TOKEN", token())
        .stdin(Stdio::null())
        .output()
        .map_err(|e| e.to_string())
}

#[tokio::test(flavor = "multi_thread")]
async fn a_job_gives_the_roles_brief_for_it_and_a_failed_brief_falls_back_to_the_role() -> Res {
    let server = MockServer::start().await;
    me(&server).await;
    Mock::given(method("POST"))
        .and(path(format!("{ROLES}/shipper/brief")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "text": "BRIEF FOR THE JOB: mind the migration.", "core": "", "recall": "",
            "truncated": false
        })))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    let r = Rig::new()?;
    r.pack("shipper", Some("opus"), "- Ship small changes.")?;
    let ask = |base: String, home: PathBuf, work: PathBuf| {
        tokio::task::spawn_blocking(move || {
            run_at(
                &home,
                &work,
                &base,
                &["--json", "ask", "--role", "shipper", "--job", "job_1", "q"],
            )
        })
    };
    let o = ask(server.uri(), r.home.path().to_owned(), r.work()).await??;
    assert!(o.status.success(), "{}", text(&o));
    let sys = r.raw();
    assert!(sys.contains("--append-system-prompt"), "{sys}");
    assert!(sys.contains("BRIEF FOR THE JOB"), "{sys}");
    assert!(
        !sys.contains("Ship small changes."),
        "the brief replaces the text: {sys}"
    );
    let posted: Vec<Value> = server
        .received_requests()
        .await
        .ok_or("no request log")?
        .iter()
        .filter(|q| q.method.as_str() == "POST")
        .map(|q| serde_json::from_slice(&q.body))
        .collect::<Result<_, _>>()?;
    assert_eq!(posted.len(), 1, "{posted:?}");
    assert_eq!(posted[0]["job_id"], "job_1");
    // The next brief fails: the run goes on with the Role's own text and says so.
    Mock::given(method("POST"))
        .and(path(format!("{ROLES}/shipper/brief")))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    let o = ask(server.uri(), r.home.path().to_owned(), r.work()).await??;
    assert!(o.status.success(), "{}", text(&o));
    let sys = r.raw();
    assert!(sys.contains("Ship small changes."), "{sys}");
    assert!(
        text(&o).contains("the Role's own text is used"),
        "{}",
        text(&o)
    );
    Ok(())
}

#[test]
fn init_takes_the_same_role_and_names_it_in_the_dry_run_plan() -> Res {
    let r = Rig::new()?;
    r.pack("builder", Some("opus"), "- Build it small.")?;
    let dir = r.home.path().join("w/shop");
    let first = dir.to_str().ok_or("path")?;
    let o = r.run(
        None,
        &[
            "--json",
            "init",
            "shop",
            "--template",
            "blank",
            "--lang",
            "python",
            "--no-smoke",
            "--yes",
            "--dir",
            first,
            "--describe",
            "a small shop",
            "--dry-run",
            "--role",
            "builder",
        ],
    )?;
    assert!(o.status.success(), "{}", text(&o));
    let v = json_of(&o)?;
    let agent = &v["agent"];
    assert_eq!(agent["role"]["name"], "builder", "{v}");
    assert_eq!(agent["role"]["source"], "project");
    assert_eq!(agent["model"], "opus");
    let cmd: Vec<String> = serde_json::from_value(agent["command"].clone())?;
    assert!(
        after(&cmd, "--append-system-prompt").is_some_and(|s| s.contains("Build it small.")),
        "{cmd:?}"
    );
    // A Role that is nowhere ends the run before anything is made.
    let second = r.home.path().join("w/other");
    let o = r.run(
        None,
        &[
            "init",
            "other",
            "--template",
            "blank",
            "--lang",
            "python",
            "--no-smoke",
            "--yes",
            "--dir",
            second.to_str().ok_or("path")?,
            "--role",
            "no-such-role",
        ],
    )?;
    assert_eq!(o.status.code(), Some(2), "{}", text(&o));
    assert!(!second.exists(), "nothing is made for an unknown Role");
    Ok(())
}
