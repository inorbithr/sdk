//! `iohr ext dev [PATH] [--test]`: an extension being built, run from its folder with
//! the real token channel (PRD 0084). The manifest is linted first; the channel hands
//! out exactly its scopes and logs every request, a refusal in red; secrets resolve
//! from `IOHR_EXT_SECRET_<NAME>` or the Keychain; `STATE` and `CACHE` live under
//! `~/.inorbit/ext-dev/<name>/`; the child gets no `IOHR_TOKEN*` and no `iohr` on its
//! `PATH`; its output is streamed with the time. Nothing here reads or writes the
//! installed extensions or their lock.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::io::IsTerminal as _;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use iohr_auth::Redacted;
use tokio::io::{AsyncBufReadExt as _, AsyncRead};

use super::lint::lint;
use super::manifest::{Lang, SOURCE_FILE, SourceManifest};
use super::run::{ProfileMinter, api_origin, serve, strip_credentials};
use super::socket::Ctx;
use crate::Env;
use crate::cli::Global;
use crate::error::Error;

/// The Keychain service an extension's secret references live under.
pub(crate) const SECRET_SERVICE: &str = "hr.inorbit.iohr.ext";

/// Folders the `--test` check leaves out: build output and caches of the toolchains.
const BUILD_DIRS: &[&str] = &[
    "target",
    "Cargo.lock",
    ".git",
    "__pycache__",
    ".pytest_cache",
    "dist",
];

/// The time, as each log line starts.
fn now() -> String {
    let t = time::OffsetDateTime::now_utc();
    format!("{:02}:{:02}:{:02}Z", t.hour(), t.minute(), t.second())
}

fn colour() -> bool {
    std::env::var_os("NO_COLOR").is_none() && std::io::stderr().is_terminal()
}

/// One log line on stderr: the time, where it came from, the text; red when `alarm`.
pub(crate) fn log_line(from: &str, text: &str, alarm: bool) {
    use std::io::Write as _;
    let line = format!("{} {from:<4} {text}", now());
    let mut err = std::io::stderr().lock();
    let _ = if alarm && colour() {
        writeln!(err, "\x1b[31m{line}\x1b[0m")
    } else {
        writeln!(err, "{line}")
    };
}

/// Streams `r` line by line with the time, to stdout or stderr.
fn pump<R: AsyncRead + Unpin + Send + 'static>(
    r: R,
    tag: &'static str,
    to_stdout: bool,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut lines = tokio::io::BufReader::new(r).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let line: String = line
                .chars()
                .filter(|c| *c == '\t' || !c.is_control())
                .collect();
            if to_stdout {
                use std::io::Write as _;
                let mut out = std::io::stdout().lock();
                let _ = writeln!(out, "{} {tag:<4} {line}", now());
                let _ = out.flush();
            } else {
                log_line(tag, &line, false);
            }
        }
    })
}

/// `IOHR_EXT_SECRET_GITHUB_TOKEN` for the secret named `github_token` or `github-token`.
#[must_use]
pub(crate) fn secret_var(name: &str) -> String {
    format!("IOHR_EXT_SECRET_{}", crate::env_name(name))
}

/// The environment variables the child never sees: iohr's credentials, the SDK's
/// credential variables, and the development secrets (given only as each one's `env`).
fn hidden(key: &str) -> bool {
    key == "IOHR_TOKEN"
        || key.starts_with("IOHR_TOKEN_")
        || key.starts_with("IOHR_EXT_SECRET_")
        || key.starts_with("INORBIT_TOKEN")
        || key.starts_with("INORBIT_KEY_")
}

/// `PATH` without the folder this `iohr` is in and without any folder that holds an
/// `iohr` program: an extension must not get a full-scope token from `iohr auth token`
/// around the channel's scope check. Returns the new `PATH` and the folders left out.
fn child_path(path: &OsStr, me: Option<&Path>) -> (OsString, Vec<PathBuf>) {
    let me = me
        .and_then(|p| p.parent())
        .and_then(|p| p.canonicalize().ok());
    let mut kept = Vec::new();
    let mut dropped = Vec::new();
    for dir in std::env::split_paths(path) {
        let canon = dir.canonicalize().ok();
        let holds_iohr = ["iohr", "iohr.exe"].iter().any(|n| dir.join(n).is_file());
        if holds_iohr || (canon.is_some() && canon == me) {
            if !dropped.contains(&dir) {
                dropped.push(dir);
            }
        } else {
            kept.push(dir);
        }
    }
    (std::env::join_paths(kept).unwrap_or_default(), dropped)
}

/// Resolves each secret: `IOHR_EXT_SECRET_<NAME>` first, then the Keychain item
/// (service [`SECRET_SERVICE`], account the reference).
async fn secrets(source: &SourceManifest) -> Result<Vec<(String, Redacted<String>)>, Error> {
    let mut out = Vec::new();
    for s in &source.secrets {
        let var = secret_var(&s.name);
        if let Some(v) = std::env::var(&var).ok().filter(|v| !v.is_empty()) {
            out.push((s.env.clone(), Redacted::new(v)));
            continue;
        }
        let reference = s.reference.clone();
        let found = tokio::task::spawn_blocking(move || {
            iohr_auth::KeyringStore::open()?.get_named(SECRET_SERVICE, &reference)
        })
        .await
        .map_err(|e| Error::Failed(e.to_string()))?;
        match found {
            Ok(Some(v)) => out.push((s.env.clone(), v)),
            Ok(None) | Err(_) => {
                return Err(Error::Usage(format!(
                    "the secret {} ({}) is not set: give it as {var} while you develop, or keep \
                     it in the Keychain (service {SECRET_SERVICE}, account {}; on macOS \
                     `security add-generic-password -s {SECRET_SERVICE} -a {} -w`)",
                    s.name, s.reference, s.reference, s.reference
                )));
            }
        }
    }
    Ok(out)
}

/// A file's size and modification time, to see what a test run changed.
type Snapshot = BTreeMap<PathBuf, (u64, Option<SystemTime>)>;

fn snapshot(root: &Path) -> Snapshot {
    fn walk(root: &Path, dir: &Path, out: &mut Snapshot) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let name = e.file_name();
            if BUILD_DIRS.iter().any(|b| OsStr::new(b) == name) {
                continue;
            }
            let path = e.path();
            let Ok(meta) = e.metadata() else { continue };
            if meta.is_dir() {
                walk(root, &path, out);
            } else {
                let rel = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
                out.insert(rel, (meta.len(), meta.modified().ok()));
            }
        }
    }
    let mut out = Snapshot::new();
    walk(root, root, &mut out);
    out
}

fn changed(before: &Snapshot, after: &Snapshot) -> Vec<String> {
    after
        .iter()
        .filter(|(p, v)| before.get(*p) != Some(v))
        .map(|(p, _)| p.display().to_string())
        .chain(
            before
                .keys()
                .filter(|p| !after.contains_key(*p))
                .map(|p| format!("{} (removed)", p.display())),
        )
        .collect()
}

fn private_dir(p: &Path) -> Result<(), Error> {
    std::fs::create_dir_all(p)
        .map_err(|e| Error::Failed(format!("cannot create {}: {e}", p.display())))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| Error::Failed(format!("cannot restrict {}: {e}", p.display())))?;
    }
    Ok(())
}

/// Watches `$STATE/heartbeat.json`: each new one is logged and marked `"dev": true`.
fn watch_heartbeat(state: PathBuf) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let file = state.join("heartbeat.json");
        // A heartbeat left by an earlier run is not this run's.
        let mut seen: Option<SystemTime> = std::fs::metadata(&file).and_then(|m| m.modified()).ok();
        loop {
            let modified = std::fs::metadata(&file).and_then(|m| m.modified()).ok();
            if modified.is_some() && modified != seen {
                if let Some(text) = mark_dev(&file) {
                    log_line("iohr", &format!("heartbeat: {text}"), false);
                }
                // Marking it rewrote the file: remember that time, not the one before.
                seen = std::fs::metadata(&file).and_then(|m| m.modified()).ok();
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    })
}

/// Reads a heartbeat, writes it back with `"dev": true` when it lacks it, and returns
/// it as one line.
fn mark_dev(file: &Path) -> Option<String> {
    let text = std::fs::read_to_string(file).ok()?;
    let mut v: serde_json::Value = serde_json::from_str(&text).ok()?;
    let obj = v.as_object_mut()?;
    if obj.get("dev") != Some(&serde_json::Value::Bool(true)) {
        obj.insert("dev".into(), serde_json::Value::Bool(true));
        let tmp = file.with_extension("json.iohr-dev");
        std::fs::write(&tmp, format!("{v}\n")).ok()?;
        std::fs::rename(&tmp, file).ok()?;
    }
    Some(v.to_string())
}

/// What to run for `lang`: the program and its arguments, for a run or the tests.
fn plan(
    folder: &Path,
    source: &SourceManifest,
    name: &str,
    lang: Lang,
    test: bool,
) -> Result<Vec<(PathBuf, Vec<OsString>)>, Error> {
    let module = name.replace('-', "_");
    let args: Vec<OsString> = source.args.iter().map(OsString::from).collect();
    Ok(match (lang, test) {
        (Lang::Shell, false) => {
            let program = folder.join("bin").join(name);
            if !program.is_file() {
                return Err(Error::Usage(format!(
                    "{} is missing: a shell extension's program is bin/{name}",
                    program.display()
                )));
            }
            vec![(program, args)]
        }
        (Lang::Shell, true) => {
            let mut tests: Vec<PathBuf> = std::fs::read_dir(folder.join("tests"))
                .map_err(|e| Error::Usage(format!("cannot read tests/: {e}")))?
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "sh"))
                .collect();
            tests.sort();
            if tests.is_empty() {
                return Err(Error::Usage("tests/ holds no *.sh test".into()));
            }
            tests
                .into_iter()
                .map(|t| (PathBuf::from("sh"), vec![t.into_os_string()]))
                .collect()
        }
        (Lang::Python, false) => {
            let mut a = vec![OsString::from("-m"), OsString::from(&module)];
            a.extend(args);
            vec![(PathBuf::from("python3"), a)]
        }
        (Lang::Python, true) => vec![(
            PathBuf::from("python3"),
            ["-m", "unittest", "discover", "-s", "tests", "-v"]
                .iter()
                .map(OsString::from)
                .collect(),
        )],
        (Lang::Rust, false) => {
            let mut a: Vec<OsString> = ["run", "--quiet", "--"]
                .iter()
                .map(OsString::from)
                .collect();
            a.extend(args);
            vec![(PathBuf::from("cargo"), a)]
        }
        (Lang::Rust, true) => vec![(
            PathBuf::from("cargo"),
            vec![OsString::from("test"), OsString::from("--quiet")],
        )],
    })
}

/// Finds `program` on `path` (an absolute path is used as it is).
fn which(program: &Path, path: &OsStr) -> Option<PathBuf> {
    if program.components().count() > 1 {
        return Some(program.to_path_buf());
    }
    std::env::split_paths(path)
        .map(|d| d.join(program))
        .find(|p| p.is_file())
}

/// Runs `iohr ext dev`; returns the exit code of the extension (or of the first test
/// that failed).
#[allow(clippy::too_many_lines, reason = "the steps of one run, in order")]
pub(crate) async fn dev(global: &Global, env: &Env, path: &Path, test: bool) -> Result<u8, Error> {
    let folder = path
        .canonicalize()
        .map_err(|e| Error::Usage(format!("cannot open {}: {e}", path.display())))?;
    let file = folder.join(SOURCE_FILE);
    let manifest = std::fs::read_to_string(&file).map_err(|e| {
        Error::Usage(format!(
            "cannot read {}: {e}. `iohr ext init PUBLISHER/NAME --lang shell` makes a folder with one",
            file.display()
        ))
    })?;
    let report = lint(&manifest);
    if !report.ok {
        for f in &report.findings {
            log_line("lint", &format!("{}: {}", f.rule, f.message), true);
        }
        return Err(Error::Findings(format!(
            "{SOURCE_FILE} has {} problem(s); `iohr ext lint --preview` shows them",
            report.findings.len()
        )));
    }
    let source = SourceManifest::parse(&manifest).map_err(|e| Error::Usage(e.0))?;
    let (_, name) = source.split_id().map_err(|e| Error::Usage(e.0))?;
    let name = name.to_owned();
    let lang = source.dev.as_ref().map(|d| d.lang).ok_or_else(|| {
        Error::Usage(format!(
            "{SOURCE_FILE} has no [dev] lang: add `[dev]` with `lang = \"rust\"`, \"python\" or \"shell\""
        ))
    })?;

    let home = etcetera::home_dir()
        .map_err(|e| Error::Failed(format!("cannot find the home folder: {e}")))?;
    let base = home.join(".inorbit").join("ext-dev").join(&name);
    let state = base.join("state");
    let cache = base.join("cache");
    private_dir(&state)?;
    private_dir(&cache)?;
    let secrets = secrets(&source).await?;

    let (path_var, dropped) = child_path(
        &std::env::var_os("PATH").unwrap_or_default(),
        std::env::current_exe().ok().as_deref(),
    );
    for d in &dropped {
        log_line(
            "iohr",
            &format!(
                "left {} out of the extension's PATH: it holds iohr, and an extension gets tokens from the channel only",
                d.display()
            ),
            false,
        );
    }
    // Under --test, a fresh HOME and TMPDIR show what the tests wrote outside STATE and
    // CACHE; the toolchains keep their own homes.
    let sandbox = test.then(|| base.join("test-home"));
    if let Some(h) = &sandbox {
        let _ = std::fs::remove_dir_all(h);
        private_dir(h)?;
        private_dir(&cache.join("tmp"))?;
    }
    let api = api_origin(&global.base_url)?;
    let api = api.as_str().trim_end_matches('/').to_owned();
    let scopes = source.permissions.scopes.clone();
    log_line(
        "iohr",
        &format!(
            "{} {} ({}): state {}, token channel for {}",
            source.id,
            source.version,
            lang.as_str(),
            state.display(),
            if scopes.is_empty() {
                "no scopes".to_owned()
            } else {
                scopes.join(" ")
            }
        ),
        false,
    );
    let before = test.then(|| snapshot(&folder));
    let watcher = watch_heartbeat(state.clone());
    let mut code = 0u8;
    for (program, args) in plan(&folder, &source, &name, lang, test)? {
        let resolved = which(&program, &path_var).ok_or_else(|| {
            Error::Usage(format!(
                "{} is not on PATH: a {} extension needs it",
                program.display(),
                lang.as_str()
            ))
        })?;
        let mut c = tokio::process::Command::new(&resolved);
        c.args(&args)
            .current_dir(&folder)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        strip_credentials(&mut c);
        for (k, _) in std::env::vars_os() {
            if k.to_str().is_some_and(hidden) {
                c.env_remove(&k);
            }
        }
        c.env("PATH", &path_var)
            .env("STATE", &state)
            .env("CACHE", &cache)
            .env("IOHR_EXT_DEV", "1");
        if lang == Lang::Python {
            let src = folder.join("src");
            let mut paths = vec![src];
            if let Some(p) = std::env::var_os("PYTHONPATH") {
                paths.extend(std::env::split_paths(&p));
            }
            c.env(
                "PYTHONPATH",
                std::env::join_paths(paths).unwrap_or_default(),
            );
        }
        if let Some(h) = &sandbox {
            for (var, dir) in [("CARGO_HOME", ".cargo"), ("RUSTUP_HOME", ".rustup")] {
                if std::env::var_os(var).is_none() {
                    c.env(var, home.join(dir));
                }
            }
            c.env("HOME", h)
                .env("TMPDIR", cache.join("tmp"))
                .env("PYTHONDONTWRITEBYTECODE", "1");
        }
        for (var, value) in &secrets {
            c.env(var, value.expose());
        }
        let ctx = Arc::new(Ctx {
            minter: ProfileMinter::new(global, env),
            allowed: scopes.clone(),
            verbose: global.verbose,
            log: true,
        });
        log_line(
            "iohr",
            format!(
                "running {} {}",
                program.display(),
                args.iter()
                    .map(|a| a.to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join(" ")
            )
            .trim_end(),
            false,
        );
        code = serve(c, &resolved, &api, ctx, |child| {
            let mut tasks = Vec::new();
            if let Some(out) = child.stdout.take() {
                tasks.push(pump(out, "out", true));
            }
            if let Some(err) = child.stderr.take() {
                tasks.push(pump(err, "err", false));
            }
            tasks
        })
        .await?;
        if code != 0 {
            break;
        }
    }
    // One last look, so a heartbeat written just before the exit is logged and marked.
    tokio::time::sleep(Duration::from_millis(600)).await;
    watcher.abort();

    if let Some(before) = before {
        let mut outside = changed(&before, &snapshot(&folder));
        if let Some(h) = &sandbox {
            let home_files: Vec<String> = snapshot(h)
                .keys()
                .map(|p| format!("$HOME/{}", p.display()))
                .collect();
            outside.extend(home_files);
            let _ = std::fs::remove_dir_all(h);
        }
        if !outside.is_empty() {
            for p in &outside {
                log_line(
                    "iohr",
                    &format!("written outside $STATE and $CACHE: {p}"),
                    true,
                );
            }
            return Err(Error::Findings(format!(
                "the tests wrote {} file(s) outside $STATE and $CACHE",
                outside.len()
            )));
        }
        if code == 0 {
            log_line(
                "iohr",
                "tests passed; nothing written outside $STATE and $CACHE",
                false,
            );
        }
    }
    Ok(code)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::{child_path, hidden, mark_dev, secret_var};

    #[test]
    fn iohr_and_its_folder_leave_the_path() {
        let dir = tempfile::tempdir().unwrap();
        let with = dir.path().join("with");
        let without = dir.path().join("without");
        let mine = dir.path().join("mine");
        for d in [&with, &without, &mine] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(with.join("iohr"), "").unwrap();
        std::fs::write(mine.join("iohr-dev"), "").unwrap();
        let path = std::env::join_paths([&with, &without, &mine]).unwrap();
        let (p, dropped) = child_path(&path, Some(&mine.join("iohr-dev")));
        assert_eq!(std::env::split_paths(&p).collect::<Vec<_>>(), [without]);
        assert_eq!(dropped, [with, mine]);
        assert_eq!(child_path(&OsString::new(), None).1.len(), 0);
    }

    #[test]
    fn credentials_and_dev_secrets_are_hidden() {
        for k in [
            "IOHR_TOKEN",
            "IOHR_TOKEN_CI",
            "IOHR_EXT_SECRET_GITHUB_TOKEN",
            "INORBIT_TOKEN",
            "INORBIT_KEY_SECRET",
        ] {
            assert!(hidden(k), "{k}");
        }
        for k in [
            "IOHR_TOKENS",
            "PATH",
            "HOME",
            "IOHR_EXT_API",
            "GITHUB_TOKEN",
        ] {
            assert!(!hidden(k), "{k}");
        }
        assert_eq!(secret_var("github-token"), "IOHR_EXT_SECRET_GITHUB_TOKEN");
    }

    #[test]
    fn a_heartbeat_is_marked_dev() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("heartbeat.json");
        std::fs::write(&f, r#"{"state":"ok"}"#).unwrap();
        assert!(mark_dev(&f).unwrap().contains(r#""dev":true"#));
        let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&f).unwrap()).unwrap();
        assert_eq!(v["dev"], true);
        std::fs::write(&f, "not json").unwrap();
        assert!(mark_dev(&f).is_none());
    }
}
