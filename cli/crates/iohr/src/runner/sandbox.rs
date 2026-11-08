//! The OS sandbox a turn's `claude` runs in (SR-35): `sandbox-exec` on macOS, `bwrap`
//! (bubblewrap) on Linux when it is installed and works. Nothing here is kernel work: the
//! runner only writes the profile or the argument list and starts `claude` under the tool
//! the system already has.
//!
//! What the sandbox allows:
//!
//! - Writes: the turn's checkout (or, for a turn with none, the runner's own working
//!   directory), the temporary folders, Claude Code's own state (`~/.claude`,
//!   `~/.claude.json` and its backups, or `CLAUDE_CONFIG_DIR`) and the user's cache
//!   folder. Never the checkout's `.git`: the runner commits and pushes itself, outside
//!   the sandbox.
//! - Reads: everything, except the contents of the home folder's secret folders
//!   ([`SECRET_DIRS`]) and of the runner's own configuration folder outside the turn's
//!   working directory.
//! - Network (macOS only): HTTPS (TCP 443), DNS and the loopback interface. `bwrap`
//!   cannot filter by port, so on Linux the network is left as it is; the file limits
//!   still apply.
//!
//! With no sandbox on the machine the runner refuses to start unless `sandbox = "off"`.

use std::path::{Path, PathBuf};

use crate::permissions::secrets::SECRET_DIRS;

/// `sandbox` in the `[runner]` table.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Mode {
    /// The OS sandbox when this machine has one; with none, the runner does not start.
    #[default]
    Auto,
    /// No sandbox: `claude` runs as the user, with the user's rights. Said at start.
    Off,
}

impl Mode {
    pub(crate) fn parse(v: &str) -> Option<Self> {
        match v.trim() {
            "auto" => Some(Self::Auto),
            "off" => Some(Self::Off),
            _ => None,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Off => "off",
        }
    }
}

/// The macOS sandbox tool.
pub(crate) const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";

/// The sandbox this machine has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Kind {
    /// macOS `sandbox-exec` with a generated profile.
    MacOs(PathBuf),
    /// Linux `bwrap`, at this path.
    Bwrap(PathBuf),
}

impl Kind {
    pub(crate) fn describe(&self) -> String {
        match self {
            Self::MacOs(p) | Self::Bwrap(p) => p.display().to_string(),
        }
    }
}

/// How a turn's `claude` is started, decided once at start.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) enum Confine {
    /// Not decided, or no sandbox was found with `sandbox = "auto"`: every turn is
    /// refused ([`no_sandbox`]). The default, so nothing runs unconfined by accident.
    #[default]
    Missing,
    /// `sandbox = "off"`: as the user, with the user's rights.
    Off,
    /// Under this sandbox.
    With(Kind),
}

/// The folders a sandboxed turn is limited by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Paths {
    /// Where the turn may write: its checkout, or the runner's working directory.
    pub(crate) write_root: PathBuf,
    /// The user's home folder.
    pub(crate) home: PathBuf,
    /// The child's temporary folder (`TMPDIR`), when it has one of its own.
    pub(crate) tmp: Option<PathBuf>,
    /// `CLAUDE_CONFIG_DIR`, when set; else `~/.claude` and `~/.claude.json`.
    pub(crate) claude_dir: Option<PathBuf>,
    /// The runner's configuration folder: its contents stay unreadable, except the turn's
    /// working directory inside it.
    pub(crate) config_dir: Option<PathBuf>,
    /// The runner's working directory, readable even inside `config_dir`.
    pub(crate) workdir: PathBuf,
    /// Folders inside `config_dir` a turn still reads: its checkout's git mirror.
    pub(crate) read_also: Vec<PathBuf>,
    /// Folders outside the write root a turn reads and writes: its repository's shared
    /// build directory (RFC 0189).
    pub(crate) write_also: Vec<PathBuf>,
}

/// An SBPL string literal: `"` and `\` escaped.
fn quoted(p: &Path) -> String {
    let s = p.display().to_string();
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// An SBPL regular expression for everything that starts with `p` (`~/.claude.json`,
/// `~/.claude.json.backup`, the temporary file it is renamed from).
fn prefix_regex(p: &Path) -> String {
    let mut out = String::from("#\"^");
    for c in p.display().to_string().chars() {
        if c.is_ascii_alphanumeric() || c == '/' || c == '_' || c == '-' || !c.is_ascii() {
            out.push(c);
        } else {
            out.push('\\');
            out.push(c);
        }
    }
    out.push('"');
    out
}

/// The macOS profile (SBPL) for `paths`. Later rules win, so each allow that narrows a
/// deny comes after it.
pub(crate) fn macos_profile(paths: &Paths) -> String {
    let home = &paths.home;
    let mut writes = vec![
        format!("(subpath {})", quoted(&paths.write_root)),
        "(subpath \"/private/tmp\")".to_owned(),
        "(subpath \"/private/var/folders\")".to_owned(),
        "(subpath \"/dev\")".to_owned(),
        format!("(subpath {})", quoted(&home.join("Library/Caches"))),
        format!("(subpath {})", quoted(&home.join(".cache"))),
    ];
    if let Some(t) = &paths.tmp {
        writes.push(format!("(subpath {})", quoted(t)));
    }
    writes.extend(
        paths
            .write_also
            .iter()
            .map(|w| format!("(subpath {})", quoted(w))),
    );
    if let Some(d) = &paths.claude_dir {
        writes.push(format!("(subpath {})", quoted(d)));
    } else {
        writes.push(format!("(subpath {})", quoted(&home.join(".claude"))));
        writes.push(format!(
            "(regex {})",
            prefix_regex(&home.join(".claude.json"))
        ));
    }
    let mut secrets: Vec<String> = SECRET_DIRS
        .iter()
        // The login keychain is read in the process by the Security framework when Claude
        // Code reads its own sign-in; denying it would sign every turn out. Claude Code's
        // own deny rules keep its tools out of it.
        .filter(|d| **d != "Library/Keychains")
        .map(|d| format!("(subpath {})", quoted(&home.join(d))))
        .collect();
    if let Some(c) = &paths.config_dir {
        secrets.push(format!("(subpath {})", quoted(c)));
    }
    let reads: Vec<String> = std::iter::once(&paths.workdir)
        .chain(&paths.read_also)
        .chain(&paths.write_also)
        .map(|r| format!("(subpath {})", quoted(r)))
        .collect();
    let git = paths.write_root.join(".git");
    format!(
        "(version 1)\n\
         (allow default)\n\
         (deny file-write*)\n\
         (allow file-write*\n  {writes})\n\
         (deny file-write* (literal {git}) (subpath {git}))\n\
         (deny file-read-data\n  {secrets})\n\
         (allow file-read-data {reads})\n\
         (deny network-outbound)\n\
         (allow network-outbound\n  \
         (remote tcp \"*:443\")\n  \
         (remote tcp \"*:53\")\n  \
         (remote udp \"*:53\")\n  \
         (remote ip \"localhost:*\")\n  \
         (literal \"/private/var/run/mDNSResponder\"))\n",
        writes = writes.join("\n  "),
        git = quoted(&git),
        secrets = secrets.join("\n  "),
        reads = reads.join(" "),
    )
}

/// The `bwrap` arguments for `paths`, before `--` and the program. Later mounts cover
/// earlier ones: the root read-only, then the secret folders hidden under empty ones, then
/// the folders a turn writes bound writable.
pub(crate) fn bwrap_args(paths: &Paths) -> Vec<String> {
    let s = |p: &Path| p.display().to_string();
    let mut a: Vec<String> = [
        "--die-with-parent",
        "--ro-bind",
        "/",
        "/",
        "--dev",
        "/dev",
        "--proc",
        "/proc",
        "--bind",
        "/tmp",
        "/tmp",
    ]
    .map(str::to_owned)
    .to_vec();
    for d in SECRET_DIRS {
        let p = paths.home.join(d);
        if p.is_dir() {
            a.extend(["--tmpfs".to_owned(), s(&p)]);
        }
    }
    if let Some(c) = &paths.config_dir {
        a.extend(["--tmpfs".to_owned(), s(c)]);
        // The working directory, inside the hidden folder, is seen again read-only; the
        // write root below makes its own part writable.
        a.extend([
            "--ro-bind-try".to_owned(),
            s(&paths.workdir),
            s(&paths.workdir),
        ]);
        for r in &paths.read_also {
            a.extend(["--ro-bind-try".to_owned(), s(r), s(r)]);
        }
    }
    let mut writable = vec![paths.write_root.clone()];
    writable.extend(paths.write_also.iter().cloned());
    if let Some(t) = &paths.tmp {
        writable.push(t.clone());
    }
    writable.push(paths.home.join(".cache"));
    if let Some(d) = &paths.claude_dir {
        writable.push(d.clone());
    } else {
        writable.push(paths.home.join(".claude"));
        writable.push(paths.home.join(".claude.json"));
    }
    for w in writable {
        a.extend(["--bind-try".to_owned(), s(&w), s(&w)]);
    }
    let git = paths.write_root.join(".git");
    a.extend(["--ro-bind-try".to_owned(), s(&git), s(&git)]);
    a
}

/// `program args` under the sandbox `kind`: the program to start and its arguments.
pub(crate) fn wrap(
    kind: &Kind,
    paths: &Paths,
    program: &Path,
    args: &[String],
) -> (PathBuf, Vec<String>) {
    let mut out = match kind {
        Kind::MacOs(tool) => {
            let _ = tool;
            vec!["-p".to_owned(), macos_profile(paths)]
        }
        Kind::Bwrap(_) => {
            let mut a = bwrap_args(paths);
            a.push("--".to_owned());
            a
        }
    };
    out.push(program.display().to_string());
    out.extend(args.iter().cloned());
    let tool = match kind {
        Kind::MacOs(t) | Kind::Bwrap(t) => t.clone(),
    };
    (tool, out)
}

/// The sandbox tool this machine has, if any, found without running it: `sandbox-exec`
/// on macOS, `bwrap` on `PATH` on Linux.
pub(crate) fn find(path_var: Option<&std::ffi::OsStr>) -> Option<Kind> {
    if cfg!(target_os = "macos") {
        let p = PathBuf::from(SANDBOX_EXEC);
        return p.is_file().then_some(Kind::MacOs(p));
    }
    if cfg!(target_os = "linux") {
        return std::env::split_paths(path_var?)
            .filter(|d| d.is_absolute())
            .map(|d| d.join("bwrap"))
            .find(|p| p.is_file())
            .map(Kind::Bwrap);
    }
    None
}

/// Whether `kind` works here: a trivial command under it (a `bwrap` with user namespaces
/// turned off is installed and still cannot run anything).
pub(crate) async fn works(kind: &Kind) -> Result<(), String> {
    let (tool, args) = match kind {
        Kind::MacOs(t) => (
            t.clone(),
            vec![
                "-p".to_owned(),
                "(version 1)(allow default)".to_owned(),
                "/usr/bin/true".to_owned(),
            ],
        ),
        Kind::Bwrap(t) => (
            t.clone(),
            ["--ro-bind", "/", "/", "--dev", "/dev", "--", "/bin/true"]
                .map(str::to_owned)
                .to_vec(),
        ),
    };
    let run = tokio::process::Command::new(&tool)
        .args(&args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .status();
    match tokio::time::timeout(std::time::Duration::from_secs(10), run).await {
        Ok(Ok(s)) if s.success() => Ok(()),
        Ok(Ok(s)) => Err(format!("{} could not run a command ({s})", tool.display())),
        Ok(Err(e)) => Err(format!("{} did not start: {e}", tool.display())),
        Err(_) => Err(format!("{} took too long to run a command", tool.display())),
    }
}

/// What the start says, and whether it may go on: a working sandbox, or `off` with a
/// warning; else the error that stops the start.
pub(crate) async fn at_start(mode: Mode, found: Option<Kind>) -> Result<(Confine, String), String> {
    if mode == Mode::Off {
        return Ok((
            Confine::Off,
            "warning: sandbox = \"off\": claude runs without the OS sandbox, with this user's rights".to_owned(),
        ));
    }
    let Some(kind) = found else {
        return Err(no_sandbox());
    };
    works(&kind)
        .await
        .map_err(|e| format!("{e}; {}", no_sandbox()))?;
    let line = format!("claude runs in the OS sandbox ({})", kind.describe());
    Ok((Confine::With(kind), line))
}

/// Why a runner with `sandbox = "auto"` and no sandbox does not start or run a turn.
pub(crate) fn no_sandbox() -> String {
    "no OS sandbox on this machine (sandbox-exec on macOS, bwrap on Linux): install bwrap, \
     or set sandbox = \"off\" in the [runner] table of config.toml to run claude without one"
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths() -> Paths {
        Paths {
            write_root: PathBuf::from("/home/u/.config/iohr/runner/coordinate/checkouts/job_1"),
            home: PathBuf::from("/home/u"),
            tmp: Some(PathBuf::from("/var/folders/xy/T")),
            claude_dir: None,
            config_dir: Some(PathBuf::from("/home/u/.config/iohr")),
            workdir: PathBuf::from("/home/u/.config/iohr/runner/coordinate"),
            read_also: vec![PathBuf::from("/home/u/.config/iohr/runner/mirrors/o/r.git")],
            write_also: vec![PathBuf::from("/home/u/.config/iohr/runner/target/o/r")],
        }
    }

    #[test]
    fn the_macos_profile_denies_writes_then_allows_only_the_turns_folders() {
        let p = macos_profile(&paths());
        let deny = p.find("(deny file-write*)").unwrap();
        let allow = p.find("(allow file-write*").unwrap();
        assert!(deny < allow, "{p}");
        for w in [
            "(subpath \"/home/u/.config/iohr/runner/coordinate/checkouts/job_1\")",
            "(subpath \"/private/tmp\")",
            "(subpath \"/var/folders/xy/T\")",
            "(subpath \"/home/u/.claude\")",
            "(regex #\"^/home/u/\\.claude\\.json\")",
            "(subpath \"/home/u/Library/Caches\")",
        ] {
            assert!(p.contains(w), "{w} missing from\n{p}");
        }
        // The home folder as a whole is never writable.
        assert!(!p.contains("(subpath \"/home/u\")"), "{p}");
        // The checkout's .git is denied after the allow.
        let git = p.find("(deny file-write* (literal \"/home/u/.config/iohr/runner/coordinate/checkouts/job_1/.git\")").unwrap();
        assert!(git > allow, "{p}");
    }

    #[test]
    fn the_macos_profile_hides_secrets_and_the_config_folder_but_not_the_workdir() {
        let p = macos_profile(&paths());
        assert!(p.contains("(subpath \"/home/u/.ssh\")"), "{p}");
        assert!(p.contains("(subpath \"/home/u/.aws\")"), "{p}");
        assert!(!p.contains("Library/Keychains"), "{p}");
        let hide = p.find("(deny file-read-data").unwrap();
        let config = p.find("(subpath \"/home/u/.config/iohr\")").unwrap();
        let see = p
            .find("(allow file-read-data (subpath \"/home/u/.config/iohr/runner/coordinate\")")
            .unwrap();
        assert!(hide < config && config < see, "{p}");
        // The checkout's mirror is read again, and its shared build directory is read and
        // written; nothing else of the configuration folder is.
        let seen = &p[see..p[see..].find('\n').unwrap() + see];
        assert!(
            seen.contains("runner/mirrors/o/r.git") && seen.contains("runner/target/o/r"),
            "{p}"
        );
        let writes = &p
            [p.find("(allow file-write*").unwrap()..p.find("(deny file-write* (literal").unwrap()];
        assert!(
            writes.contains("runner/target/o/r") && !writes.contains("mirrors"),
            "{p}"
        );
    }

    #[test]
    fn the_macos_profile_limits_the_network_to_https_dns_and_loopback() {
        let p = macos_profile(&paths());
        let deny = p.find("(deny network-outbound)").unwrap();
        let allow = p.find("(allow network-outbound").unwrap();
        assert!(deny < allow, "{p}");
        let net = &p[allow..];
        for r in [
            "(remote tcp \"*:443\")",
            "(remote udp \"*:53\")",
            "(remote ip \"localhost:*\")",
        ] {
            assert!(net.contains(r), "{r} missing from\n{p}");
        }
        assert!(!net.contains("*:*"), "{p}");
    }

    #[test]
    fn a_claude_config_dir_replaces_the_home_folder_claude_files() {
        let mut ps = paths();
        ps.claude_dir = Some(PathBuf::from("/data/claude"));
        let p = macos_profile(&ps);
        assert!(p.contains("(subpath \"/data/claude\")"), "{p}");
        assert!(!p.contains(".claude.json"), "{p}");
    }

    #[test]
    fn quotes_and_backslashes_in_a_path_cannot_end_the_literal() {
        let mut ps = paths();
        ps.write_root = PathBuf::from("/w/a\"b\\c");
        let p = macos_profile(&ps);
        assert!(p.contains("(subpath \"/w/a\\\"b\\\\c\")"), "{p}");
        let r = prefix_regex(Path::new("/h/.claude.json"));
        assert_eq!(r, "#\"^/h/\\.claude\\.json\"");
    }

    #[test]
    fn bwrap_mounts_the_root_read_only_and_the_turns_folders_writable_after() {
        let a = bwrap_args(&paths());
        let pos = |needle: &[&str]| {
            a.windows(needle.len())
                .position(|w| w.iter().zip(needle).all(|(x, y)| x == y))
                .unwrap_or_else(|| panic!("{needle:?} missing from {a:?}"))
        };
        let root = pos(&["--ro-bind", "/", "/"]);
        let hide = pos(&["--tmpfs", "/home/u/.config/iohr"]);
        let wr = "/home/u/.config/iohr/runner/coordinate/checkouts/job_1";
        let write = pos(&["--bind-try", wr, wr]);
        let git = format!("{wr}/.git");
        let git_ro = pos(&["--ro-bind-try", &git, &git]);
        assert!(root < hide && hide < write && write < git_ro, "{a:?}");
        pos(&["--bind-try", "/home/u/.claude", "/home/u/.claude"]);
        let mirror = "/home/u/.config/iohr/runner/mirrors/o/r.git";
        assert!(hide < pos(&["--ro-bind-try", mirror, mirror]), "{a:?}");
        let target = "/home/u/.config/iohr/runner/target/o/r";
        assert!(hide < pos(&["--bind-try", target, target]), "{a:?}");
        pos(&["--die-with-parent"]);
        assert!(!a.iter().any(|x| x == "--unshare-net"), "{a:?}");
        assert!(
            !a.iter().any(|x| x == "/home/u"),
            "the home folder is never bound writable: {a:?}"
        );
    }

    #[test]
    fn wrap_puts_the_program_and_its_arguments_after_the_sandbox() {
        let args = vec!["-p".to_owned(), "--model".to_owned(), "m".to_owned()];
        let (tool, out) = wrap(
            &Kind::MacOs(PathBuf::from(SANDBOX_EXEC)),
            &paths(),
            Path::new("/usr/local/bin/claude"),
            &args,
        );
        assert_eq!(tool, PathBuf::from(SANDBOX_EXEC));
        assert_eq!(out[0], "-p");
        assert!(out[1].starts_with("(version 1)"));
        assert_eq!(&out[2..], ["/usr/local/bin/claude", "-p", "--model", "m"]);
        let (tool, out) = wrap(
            &Kind::Bwrap(PathBuf::from("/usr/bin/bwrap")),
            &paths(),
            Path::new("/c"),
            &args,
        );
        assert_eq!(tool, PathBuf::from("/usr/bin/bwrap"));
        let dash = out.iter().position(|x| x == "--").unwrap();
        assert_eq!(&out[dash + 1..], ["/c", "-p", "--model", "m"]);
    }

    #[tokio::test]
    async fn no_sandbox_refuses_the_start_unless_it_is_off() {
        let e = at_start(Mode::Auto, None).await.unwrap_err();
        assert!(
            e.contains("sandbox = \"off\"") && e.contains("bwrap"),
            "{e}"
        );
        let (kind, line) = at_start(Mode::Off, None).await.unwrap();
        assert_eq!(kind, Confine::Off);
        assert!(line.starts_with("warning:"), "{line}");
        // A tool that is there and does not work is no sandbox either.
        let e = at_start(
            Mode::Auto,
            Some(Kind::Bwrap(PathBuf::from("/nonexistent/bwrap"))),
        )
        .await
        .unwrap_err();
        assert!(
            e.contains("did not start") && e.contains("sandbox = \"off\""),
            "{e}"
        );
    }

    #[test]
    fn modes_parse_only_auto_and_off() {
        assert_eq!(Mode::parse("auto"), Some(Mode::Auto));
        assert_eq!(Mode::parse(" off "), Some(Mode::Off));
        assert_eq!(Mode::parse("on"), None);
        assert_eq!(Mode::default(), Mode::Auto);
    }

    /// The profile on this Mac for real: the checkout is writable, its `.git` is not, and
    /// a secret folder's file cannot be read. (Every temporary folder is writable under
    /// the profile, so the test's own folder stands in for the checkout.)
    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn the_macos_profile_runs_and_holds_on_this_machine() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let work = root.join("work");
        let home = root.join("home");
        std::fs::create_dir_all(work.join(".git")).unwrap();
        std::fs::create_dir_all(home.join(".ssh")).unwrap();
        std::fs::write(home.join(".ssh/id_test"), "not a key").unwrap();
        let ps = Paths {
            write_root: work.clone(),
            home: home.clone(),
            tmp: None,
            claude_dir: None,
            config_dir: None,
            workdir: work.clone(),
            read_also: Vec::new(),
            write_also: Vec::new(),
        };
        let profile = macos_profile(&ps);
        let run = |program: &str, target: PathBuf| {
            let mut c = tokio::process::Command::new(SANDBOX_EXEC);
            c.args(["-p", &profile, program]).arg(target);
            c.stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            async move { c.status().await.unwrap().success() }
        };
        assert!(run("/usr/bin/touch", work.join("ok")).await);
        assert!(!run("/usr/bin/touch", work.join(".git/hook")).await);
        assert!(!work.join(".git/hook").exists());
        assert!(!run("/bin/cat", home.join(".ssh/id_test")).await);
    }
}
