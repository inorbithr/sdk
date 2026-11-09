//! `iohr runner service`: the runner as a service of the person's own user, started at
//! login and restarted when it stops: a `LaunchAgent` on macOS, a systemd user unit on
//! Linux. Never a system service, never sudo.
//!
//! Everything is decided first, as a list of [`Step`]s from the service file's text
//! ([`render`]), so `--dry-run` prints exactly what an install or uninstall would do and
//! the tests compare that text without touching a real service manager. [`run`] then
//! carries the steps out with an argument vector each, never a shell.

use std::fmt::{self, Write as _};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The `LaunchAgent`'s label, and its file's name, without a profile.
pub(crate) const LABEL: &str = "hr.inorbit.runner";
/// The systemd user unit's name, without a profile or `.service`.
pub(crate) const UNIT: &str = "iohr-runner";

/// Who runs the service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Manager {
    Launchd,
    Systemd,
}

impl Manager {
    /// This machine's, or `None` where the runner is not installed as a service.
    pub(crate) const fn here() -> Option<Self> {
        if cfg!(target_os = "macos") {
            Some(Self::Launchd)
        } else if cfg!(target_os = "linux") {
            Some(Self::Systemd)
        } else {
            None
        }
    }
}

/// What the service runs, and where its file goes.
#[derive(Debug, Clone)]
pub(crate) struct Spec {
    /// This `iohr`, as an absolute path.
    pub(crate) program: PathBuf,
    /// `--profile`, when given.
    pub(crate) profile: Option<String>,
    /// `IOHR_CONFIG_DIR`, when set, so the service reads the same configuration.
    pub(crate) config_dir: Option<PathBuf>,
    pub(crate) home: PathBuf,
    /// `PATH` as it is now: the service finds `claude`, `git` and `gh` where this shell
    /// does.
    pub(crate) path: String,
    /// The user's id (launchd's `gui/<uid>` domain).
    pub(crate) uid: u32,
    /// `XDG_CONFIG_HOME`, when set (systemd reads user units under it).
    pub(crate) xdg_config: Option<PathBuf>,
}

/// One service, rendered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Unit {
    pub(crate) manager: Manager,
    /// The `LaunchAgent`'s label, or the unit's file name.
    pub(crate) name: String,
    pub(crate) file: PathBuf,
    pub(crate) text: String,
    /// Where launchd writes its output; systemd keeps it in the journal.
    pub(crate) log: Option<PathBuf>,
}

fn suffix(profile: Option<&str>) -> String {
    profile.map(|p| format!("-{p}")).unwrap_or_default()
}

fn utf8(p: &Path) -> Result<&str, String> {
    p.to_str().ok_or_else(|| {
        format!(
            "{} is not UTF-8: a service file cannot name it",
            p.display()
        )
    })
}

/// The command line the service starts.
fn args(s: &Spec) -> Result<Vec<String>, String> {
    let mut a = vec![utf8(&s.program)?.to_owned()];
    if let Some(p) = &s.profile {
        a.extend(["--profile".to_owned(), p.clone()]);
    }
    a.extend(["runner".to_owned(), "run".to_owned()]);
    Ok(a)
}

/// The service's environment, in a fixed order.
fn environment(s: &Spec) -> Result<Vec<(&'static str, String)>, String> {
    let mut e = vec![
        ("HOME", utf8(&s.home)?.to_owned()),
        ("PATH", s.path.clone()),
    ];
    if let Some(d) = &s.config_dir {
        e.push(("IOHR_CONFIG_DIR", utf8(d)?.to_owned()));
    }
    Ok(e)
}

/// The directory systemd reads user units from.
fn systemd_dir(s: &Spec) -> PathBuf {
    s.xdg_config
        .clone()
        .unwrap_or_else(|| s.home.join(".config"))
        .join("systemd/user")
}

fn launchd_dir(s: &Spec) -> PathBuf {
    s.home.join("Library/LaunchAgents")
}

/// The service for `s`, its file's text included.
pub(crate) fn render(m: Manager, s: &Spec) -> Result<Unit, String> {
    let suffix = suffix(s.profile.as_deref());
    let (args, env) = (args(s)?, environment(s)?);
    Ok(match m {
        Manager::Launchd => {
            let label = format!("{LABEL}{suffix}");
            let log = s
                .home
                .join(format!("Library/Logs/InOrbit/runner{suffix}.log"));
            Unit {
                manager: m,
                file: launchd_dir(s).join(format!("{label}.plist")),
                text: plist(&label, &args, &env, utf8(&log)?),
                name: label,
                log: Some(log),
            }
        }
        Manager::Systemd => {
            let name = format!("{UNIT}{suffix}.service");
            Unit {
                manager: m,
                file: systemd_dir(s).join(&name),
                text: systemd(s.profile.as_deref(), &args, &env),
                name,
                log: None,
            }
        }
    })
}

fn xml(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&apos;"),
            c => o.push(c),
        }
    }
    o
}

fn plist(label: &str, args: &[String], env: &[(&str, String)], log: &str) -> String {
    let mut t = String::from(concat!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
        "<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n",
        "<plist version=\"1.0\">\n<dict>\n",
        "\t<key>Label</key>\n",
    ));
    let _ = writeln!(t, "\t<string>{}</string>", xml(label));
    t.push_str("\t<key>ProgramArguments</key>\n\t<array>\n");
    for a in args {
        let _ = writeln!(t, "\t\t<string>{}</string>", xml(a));
    }
    t.push_str("\t</array>\n\t<key>EnvironmentVariables</key>\n\t<dict>\n");
    for (k, v) in env {
        let _ = writeln!(t, "\t\t<key>{k}</key>\n\t\t<string>{}</string>", xml(v));
    }
    t.push_str("\t</dict>\n\t<key>RunAtLoad</key>\n\t<true/>\n\t<key>KeepAlive</key>\n\t<true/>\n");
    t.push_str("\t<key>ThrottleInterval</key>\n\t<integer>10</integer>\n");
    for k in ["StandardOutPath", "StandardErrorPath"] {
        let _ = writeln!(t, "\t<key>{k}</key>\n\t<string>{}</string>", xml(log));
    }
    t.push_str("</dict>\n</plist>\n");
    t
}

/// A systemd value in double quotes: `\` and `"` escaped, `%` (a specifier) doubled, and
/// `$` doubled where systemd expands variables (`ExecStart=`).
fn quoted(s: &str, expands: bool) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '\\' => o.push_str("\\\\"),
            '"' => o.push_str("\\\""),
            '%' => o.push_str("%%"),
            '$' if expands => o.push_str("$$"),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

fn systemd(profile: Option<&str>, args: &[String], env: &[(&str, String)]) -> String {
    let mut t = String::from("[Unit]\n");
    let _ = writeln!(
        t,
        "Description=InOrbit runner{}",
        profile.map(|p| format!(" ({p})")).unwrap_or_default()
    );
    t.push_str("\n[Service]\n");
    let exec: Vec<String> = args.iter().map(|a| quoted(a, true)).collect();
    let _ = writeln!(t, "ExecStart={}", exec.join(" "));
    for (k, v) in env {
        let _ = writeln!(t, "Environment={}", quoted(&format!("{k}={v}"), false));
    }
    t.push_str("Restart=always\nRestartSec=10\n\n[Install]\nWantedBy=default.target\n");
    t
}

/// One thing an install or uninstall does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Step {
    Dir(PathBuf),
    Write(PathBuf, String),
    Remove(PathBuf),
    Run(Vec<String>),
    /// Waits until launchd has let go of a label it was told to stop.
    Gone(Vec<String>),
}

impl fmt::Display for Step {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Dir(p) => write!(f, "create {}", p.display()),
            Self::Write(p, _) => write!(f, "write {}", p.display()),
            Self::Remove(p) => write!(f, "remove {}", p.display()),
            Self::Run(a) => write!(f, "run {}", a.join(" ")),
            Self::Gone(a) => write!(f, "wait until `{}` finds nothing", a.join(" ")),
        }
    }
}

fn cmd(a: &[&str]) -> Vec<String> {
    a.iter().map(|s| (*s).to_owned()).collect()
}

fn target(u: &Unit, s: &Spec) -> String {
    format!("gui/{}/{}", s.uid, u.name)
}

/// The command that says whether the service is loaded: it succeeds when it is.
pub(crate) fn probe(u: &Unit, s: &Spec) -> Vec<String> {
    match u.manager {
        Manager::Launchd => cmd(&["launchctl", "print", &target(u, s)]),
        Manager::Systemd => cmd(&["systemctl", "--user", "is-enabled", "--quiet", &u.name]),
    }
}

/// Install: write the file and (re)start the service. `loaded` says whether launchd has
/// the label now: it is stopped first, so the new file is the one read.
pub(crate) fn install(u: &Unit, s: &Spec, loaded: bool) -> Vec<Step> {
    let mut steps = Vec::new();
    if let Some(dir) = u.file.parent() {
        steps.push(Step::Dir(dir.to_path_buf()));
    }
    if let Some(dir) = u.log.as_deref().and_then(Path::parent) {
        steps.push(Step::Dir(dir.to_path_buf()));
    }
    steps.push(Step::Write(u.file.clone(), u.text.clone()));
    let file = u.file.display().to_string();
    match u.manager {
        Manager::Launchd => {
            let domain = format!("gui/{}", s.uid);
            if loaded {
                steps.push(Step::Run(cmd(&["launchctl", "bootout", &target(u, s)])));
                steps.push(Step::Gone(probe(u, s)));
            }
            steps.push(Step::Run(cmd(&["launchctl", "bootstrap", &domain, &file])));
        }
        Manager::Systemd => {
            steps.push(Step::Run(cmd(&["systemctl", "--user", "daemon-reload"])));
            steps.push(Step::Run(cmd(&["systemctl", "--user", "enable", &u.name])));
            // Starts it, or restarts one running from an older file.
            steps.push(Step::Run(cmd(&["systemctl", "--user", "restart", &u.name])));
        }
    }
    steps
}

/// Uninstall: stop the service and remove its file; nothing for what is not there.
pub(crate) fn uninstall(u: &Unit, s: &Spec, loaded: bool, present: bool) -> Vec<Step> {
    let mut steps = Vec::new();
    match u.manager {
        Manager::Launchd => {
            if loaded {
                steps.push(Step::Run(cmd(&["launchctl", "bootout", &target(u, s)])));
            }
            if present {
                steps.push(Step::Remove(u.file.clone()));
            }
        }
        Manager::Systemd => {
            if loaded || present {
                steps.push(Step::Run(cmd(&[
                    "systemctl",
                    "--user",
                    "disable",
                    "--now",
                    &u.name,
                ])));
            }
            if present {
                steps.push(Step::Remove(u.file.clone()));
                steps.push(Step::Run(cmd(&["systemctl", "--user", "daemon-reload"])));
            }
        }
    }
    steps
}

/// Whether `argv` succeeds, its output dropped.
pub(crate) fn succeeds(argv: &[String]) -> bool {
    let Some((program, rest)) = argv.split_first() else {
        return false;
    };
    Command::new(program)
        .args(rest)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Carries out `steps` in order, saying each first; stops at the first that fails.
pub(crate) fn run(steps: &[Step], say: impl Fn(&str)) -> Result<(), String> {
    for step in steps {
        say(&step.to_string());
        match step {
            Step::Dir(p) => fs::create_dir_all(p).map_err(|e| format!("{step}: {e}"))?,
            Step::Write(p, text) => fs::write(p, text).map_err(|e| format!("{step}: {e}"))?,
            Step::Remove(p) => match fs::remove_file(p) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                    return Err(format!("{step}: {e}"));
                }
                _ => {}
            },
            Step::Run(argv) => {
                let (program, rest) = argv.split_first().ok_or("an empty command")?;
                let out = Command::new(program)
                    .args(rest)
                    .stdin(Stdio::null())
                    .output()
                    .map_err(|e| format!("{step}: {e}"))?;
                if !out.status.success() {
                    let why = String::from_utf8_lossy(&out.stderr);
                    return Err(format!("{step}: {} ({})", out.status, why.trim()));
                }
            }
            Step::Gone(argv) => {
                let deadline = Instant::now() + Duration::from_secs(15);
                while succeeds(argv) {
                    if Instant::now() > deadline {
                        return Err(format!("{step}: still loaded after 15 s"));
                    }
                    std::thread::sleep(Duration::from_millis(250));
                }
            }
        }
    }
    Ok(())
}

/// The other runner services installed beside `u` (other profiles'), by their files.
pub(crate) fn others(u: &Unit) -> Vec<PathBuf> {
    let (prefix, ext) = match u.manager {
        Manager::Launchd => (LABEL, ".plist"),
        Manager::Systemd => (UNIT, ".service"),
    };
    let Some(dir) = u.file.parent() else {
        return Vec::new();
    };
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| *p != u.file)
        .filter(|p| {
            p.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                n.strip_suffix(ext).is_some_and(|stem| {
                    stem == prefix
                        || stem
                            .strip_prefix(prefix)
                            .is_some_and(|r| r.starts_with('-'))
                })
            })
        })
        .collect();
    found.sort();
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(profile: Option<&str>) -> Spec {
        Spec {
            program: PathBuf::from("/opt/iohr/bin/iohr"),
            profile: profile.map(str::to_owned),
            config_dir: None,
            home: PathBuf::from("/Users/a"),
            path: "/opt/iohr/bin:/usr/bin:/bin".into(),
            uid: 501,
            xdg_config: None,
        }
    }

    #[test]
    fn the_launch_agent_runs_this_iohr_as_the_runner_at_login() {
        let u = render(Manager::Launchd, &spec(None)).unwrap();
        assert_eq!(u.name, "hr.inorbit.runner");
        assert_eq!(
            u.file,
            Path::new("/Users/a/Library/LaunchAgents/hr.inorbit.runner.plist")
        );
        let want = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>Label</key>
	<string>hr.inorbit.runner</string>
	<key>ProgramArguments</key>
	<array>
		<string>/opt/iohr/bin/iohr</string>
		<string>runner</string>
		<string>run</string>
	</array>
	<key>EnvironmentVariables</key>
	<dict>
		<key>HOME</key>
		<string>/Users/a</string>
		<key>PATH</key>
		<string>/opt/iohr/bin:/usr/bin:/bin</string>
	</dict>
	<key>RunAtLoad</key>
	<true/>
	<key>KeepAlive</key>
	<true/>
	<key>ThrottleInterval</key>
	<integer>10</integer>
	<key>StandardOutPath</key>
	<string>/Users/a/Library/Logs/InOrbit/runner.log</string>
	<key>StandardErrorPath</key>
	<string>/Users/a/Library/Logs/InOrbit/runner.log</string>
</dict>
</plist>
"#;
        assert_eq!(u.text, want);
    }

    #[test]
    fn a_profile_names_its_own_service_and_is_passed_to_the_runner() {
        let mut s = spec(Some("work"));
        s.config_dir = Some(PathBuf::from("/Users/a/cfg & more"));
        let u = render(Manager::Launchd, &s).unwrap();
        assert_eq!(u.name, "hr.inorbit.runner-work");
        assert!(u.file.ends_with("hr.inorbit.runner-work.plist"));
        assert!(u.text.contains(
            "\t\t<string>/opt/iohr/bin/iohr</string>\n\t\t<string>--profile</string>\n\t\t<string>work</string>\n\t\t<string>runner</string>"
        ));
        assert!(
            u.text.contains(
                "<key>IOHR_CONFIG_DIR</key>\n\t\t<string>/Users/a/cfg &amp; more</string>"
            )
        );
        assert!(u.text.contains("Logs/InOrbit/runner-work.log"));
        let u = render(Manager::Systemd, &s).unwrap();
        assert_eq!(u.name, "iohr-runner-work.service");
        assert_eq!(
            u.file,
            Path::new("/Users/a/.config/systemd/user/iohr-runner-work.service")
        );
    }

    #[test]
    fn the_systemd_unit_is_a_user_unit_restarted_always_and_quoted() {
        let mut s = spec(None);
        s.home = PathBuf::from("/home/a");
        s.program = PathBuf::from("/home/a/bin/i$o%hr \"x\"");
        s.xdg_config = Some(PathBuf::from("/home/a/xdg"));
        let u = render(Manager::Systemd, &s).unwrap();
        assert_eq!(
            u.file,
            Path::new("/home/a/xdg/systemd/user/iohr-runner.service")
        );
        assert_eq!(u.log, None);
        let want = r#"[Unit]
Description=InOrbit runner

[Service]
ExecStart="/home/a/bin/i$$o%%hr \"x\"" "runner" "run"
Environment="HOME=/home/a"
Environment="PATH=/opt/iohr/bin:/usr/bin:/bin"
Restart=always
RestartSec=10

[Install]
WantedBy=default.target
"#;
        assert_eq!(u.text, want);
    }

    fn said(steps: &[Step]) -> Vec<String> {
        steps.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn install_and_uninstall_say_each_step_and_never_sudo() {
        let s = spec(None);
        let u = render(Manager::Launchd, &s).unwrap();
        let plist = "/Users/a/Library/LaunchAgents/hr.inorbit.runner.plist";
        assert_eq!(
            said(&install(&u, &s, false)),
            [
                "create /Users/a/Library/LaunchAgents".to_owned(),
                "create /Users/a/Library/Logs/InOrbit".to_owned(),
                format!("write {plist}"),
                format!("run launchctl bootstrap gui/501 {plist}"),
            ]
        );
        // Loaded already: stopped first, so launchd reads the new file.
        assert_eq!(
            said(&install(&u, &s, true))[3..],
            [
                "run launchctl bootout gui/501/hr.inorbit.runner".to_owned(),
                "wait until `launchctl print gui/501/hr.inorbit.runner` finds nothing".to_owned(),
                format!("run launchctl bootstrap gui/501 {plist}"),
            ]
        );
        assert_eq!(
            said(&uninstall(&u, &s, true, true)),
            [
                "run launchctl bootout gui/501/hr.inorbit.runner".to_owned(),
                format!("remove {plist}"),
            ]
        );
        let none = uninstall(&u, &s, false, false);
        assert!(none.is_empty(), "{:?}", said(&none));
        let u = render(Manager::Systemd, &s).unwrap();
        let unit = "/Users/a/.config/systemd/user/iohr-runner.service";
        assert_eq!(
            said(&install(&u, &s, true))[1..],
            [
                format!("write {unit}"),
                "run systemctl --user daemon-reload".to_owned(),
                "run systemctl --user enable iohr-runner.service".to_owned(),
                "run systemctl --user restart iohr-runner.service".to_owned(),
            ]
        );
        assert_eq!(
            said(&uninstall(&u, &s, false, true)),
            [
                "run systemctl --user disable --now iohr-runner.service".to_owned(),
                format!("remove {unit}"),
                "run systemctl --user daemon-reload".to_owned(),
            ]
        );
        for m in [Manager::Launchd, Manager::Systemd] {
            let u = render(m, &s).unwrap();
            for step in install(&u, &s, true)
                .iter()
                .chain(&uninstall(&u, &s, true, true))
            {
                assert!(!step.to_string().contains("sudo"), "{step}");
            }
        }
    }

    #[test]
    fn the_other_runner_services_are_found_by_their_files() {
        let t = tempfile::tempdir().unwrap();
        let mut s = spec(None);
        s.home = t.path().to_path_buf();
        let u = render(Manager::Launchd, &s).unwrap();
        let dir = u.file.parent().unwrap();
        fs::create_dir_all(dir).unwrap();
        for n in [
            "hr.inorbit.runner.plist",
            "hr.inorbit.runner-work.plist",
            "hr.inorbit.runnerx.plist",
            "hr.inorbit.other.plist",
        ] {
            fs::write(dir.join(n), "").unwrap();
        }
        assert_eq!(others(&u), [dir.join("hr.inorbit.runner-work.plist")]);
        let mut w = s.clone();
        w.profile = Some("work".into());
        let u = render(Manager::Launchd, &w).unwrap();
        assert_eq!(others(&u), [dir.join("hr.inorbit.runner.plist")]);
    }
}
