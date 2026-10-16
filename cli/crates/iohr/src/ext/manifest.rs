//! An extension's manifest: the OCI config blob of each platform's artifact.

use serde::{Deserialize, Serialize};

/// The config media type of an extension artifact.
pub const CONFIG_MEDIA_TYPE: &str = "application/vnd.inorbit.iohr.extension.config.v1+json";
/// The media type of the one layer, a gzipped tar that holds the program.
pub const LAYER_MEDIA_TYPE: &str = "application/vnd.inorbit.iohr.extension.layer.v1.tar+gzip";

/// The largest manifest read.
pub const MAX_MANIFEST: usize = 64 * 1024;
const MAX_SCOPES: usize = 64;
const MAX_DESCRIPTION: usize = 500;
/// The most privileges an extension may declare.
pub const MAX_PRIVILEGES: usize = 16;

/// Every Linux capability the kernel defines (`include/uapi/linux/capability.h`, 0 to
/// `CAP_LAST_CAP`, Linux 5.9 and later), each with what it lets a program do, in the
/// words `iohr ext install` shows. An extension may declare only these.
pub const CAPABILITIES: &[(&str, &str)] = &[
    ("CAP_CHOWN", "change the owner of any file"),
    (
        "CAP_DAC_OVERRIDE",
        "read, write and run any file, whatever its permissions",
    ),
    (
        "CAP_DAC_READ_SEARCH",
        "read any file and list any directory",
    ),
    (
        "CAP_FOWNER",
        "act as the owner of any file (permissions, times)",
    ),
    (
        "CAP_FSETID",
        "keep set-user-ID and set-group-ID bits on files it changes",
    ),
    ("CAP_KILL", "send signals to any process"),
    ("CAP_SETGID", "switch to any group"),
    ("CAP_SETUID", "switch to any user, including root"),
    (
        "CAP_SETPCAP",
        "give or drop capabilities of its own processes",
    ),
    (
        "CAP_LINUX_IMMUTABLE",
        "make files immutable or append-only, and undo it",
    ),
    ("CAP_NET_BIND_SERVICE", "listen on ports below 1024"),
    (
        "CAP_NET_BROADCAST",
        "send broadcast and listen to multicast",
    ),
    (
        "CAP_NET_ADMIN",
        "configure the network: interfaces, routes, firewall, traffic control",
    ),
    ("CAP_NET_RAW", "open raw sockets and capture packets"),
    ("CAP_IPC_LOCK", "lock memory so it is never swapped out"),
    (
        "CAP_IPC_OWNER",
        "use any shared memory, semaphore or message queue",
    ),
    ("CAP_SYS_MODULE", "load and unload kernel modules"),
    (
        "CAP_SYS_RAWIO",
        "read and write devices and I/O ports directly",
    ),
    ("CAP_SYS_CHROOT", "change its root directory"),
    (
        "CAP_SYS_PTRACE",
        "inspect and control any process, and read its memory",
    ),
    ("CAP_SYS_PACCT", "turn process accounting on and off"),
    (
        "CAP_SYS_ADMIN",
        "administer the system: mounts, namespaces and much more; close to root",
    ),
    ("CAP_SYS_BOOT", "reboot the machine or load a new kernel"),
    ("CAP_SYS_NICE", "raise the priority of any process"),
    (
        "CAP_SYS_RESOURCE",
        "go past resource limits and disk quotas",
    ),
    ("CAP_SYS_TIME", "set the system clock"),
    ("CAP_SYS_TTY_CONFIG", "reconfigure terminals"),
    ("CAP_MKNOD", "create device files"),
    ("CAP_LEASE", "take leases on any file"),
    ("CAP_AUDIT_WRITE", "write to the kernel audit log"),
    ("CAP_AUDIT_CONTROL", "change kernel auditing and its rules"),
    ("CAP_SETFCAP", "give capabilities to program files"),
    (
        "CAP_MAC_OVERRIDE",
        "bypass mandatory access control (Smack)",
    ),
    (
        "CAP_MAC_ADMIN",
        "change mandatory access control (Smack) settings",
    ),
    ("CAP_SYSLOG", "read and clear the kernel log"),
    ("CAP_WAKE_ALARM", "set alarms that wake the system"),
    ("CAP_BLOCK_SUSPEND", "keep the system from suspending"),
    ("CAP_AUDIT_READ", "read the kernel audit log"),
    (
        "CAP_PERFMON",
        "observe performance and kernel state (perf events, tracing)",
    ),
    ("CAP_BPF", "load eBPF programs into the kernel"),
    ("CAP_CHECKPOINT_RESTORE", "checkpoint and restore processes"),
];

/// Top-level commands of `iohr` itself, which no extension may take.
pub const RESERVED: &[&str] = &[
    "accounts",
    "api",
    "auth",
    "browser",
    "completion",
    "config",
    "connections",
    "connectors",
    "coordinator",
    "decisions",
    "domains",
    "ext",
    "help",
    "iohr",
    "lab",
    "login",
    "logout",
    "openapi",
    "packages",
    "profile",
    "project",
    // `iohr rfc` until core ADR 0056 (2026-10-08): kept from extensions so a script that
    // still calls it fails as unknown rather than running someone else's program.
    "rfc",
    "sdk",
    "token",
    "tunnel",
    "upgrade",
    "whoami",
];

/// What an extension says about itself: who it is, what to run, which scopes it may ask
/// for, which privileges its system service holds. Read from the registry before
/// anything is installed, and shown to the person.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Manifest {
    /// The command it adds: `iohr <name> ...`.
    pub name: String,
    /// Its version, `MAJOR.MINOR.PATCH` with an optional pre-release.
    pub version: String,
    /// The program's path inside the layer.
    pub entrypoint: String,
    /// The API scopes it may ask `iohr` for; a token request for any other is refused.
    #[serde(default)]
    pub scopes: Vec<String>,
    /// One line about what it does.
    #[serde(default)]
    pub description: String,
    /// The Linux capabilities its system service holds (RFC 0061), such as `CAP_BPF`.
    /// `iohr` grants none of them: it runs as the person. The field tells the person
    /// what the service holds, and installing asks them to confirm it (SR-32). Absent
    /// means none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub privileges: Vec<String>,
    /// It runs as a system service on Linux that the program sets up itself with
    /// `sudo <program> service install --agent-user <user> [--interface <name>]`.
    /// `iohr ext install` offers that step after installing; `iohr ext service` runs it
    /// later. Absent means it has no system service of its own.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub service: bool,
    /// How a runtime runs it: a service, a timer or once (PRD 0084). Absent means a
    /// service restarted on failure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runs: Option<Runs>,
    /// What it may reach, read, write, start and send up, besides `scopes` and
    /// `privileges`, which stay top-level fields of the config.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permissions: Option<Permissions>,
    /// What it needs of the machine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resources: Option<Resources>,
    /// How a runtime knows it is well: a heartbeat file and an optional check.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health: Option<Health>,
    /// The secrets it is given, by reference: never a value.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secrets: Vec<SecretRef>,
    /// Where it may be installed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub targets: Option<Targets>,
    /// How a new version replaces this one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upgrade: Option<Upgrade>,
}

/// How an extension runs.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Runs {
    /// `service` (always up), `timer` (every `every`) or `oneshot`.
    #[serde(rename = "as", default)]
    pub run_as: RunAs,
    /// For a timer: `5m`, `1h`, or a calendar time such as `weekdays 17:00`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub every: String,
    /// When a service is started again.
    #[serde(default)]
    pub restart: Restart,
    /// How long it has to stop after being asked, such as `30s`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub shutdown_grace: String,
}

/// `runs.as`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunAs {
    /// Always up.
    #[default]
    Service,
    /// Started every `every`.
    Timer,
    /// Run once.
    Oneshot,
}

/// `runs.restart`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Restart {
    /// Whenever it stops.
    Always,
    /// When it exits with an error.
    #[default]
    OnFailure,
    /// Never.
    Never,
}

/// What an extension may do. `scopes` and `privileges` are read here in the source
/// manifest (`extension.toml`) and written to the config's top level, where they have
/// always been.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Permissions {
    /// API scopes over the token channel.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scopes: Vec<String>,
    /// Linux capabilities of its system service.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub privileges: Vec<String>,
    /// The hosts it may reach, `host:port`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub network: Vec<String>,
    /// The paths it may read and write; `$STATE` and `$CACHE` are its own.
    #[serde(default, skip_serializing_if = "Filesystem::is_empty")]
    pub filesystem: Filesystem,
    /// The programs it may start, by name.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exec: Vec<String>,
    /// What it may send up (ADR 0063).
    #[serde(default, skip_serializing_if = "Data::is_empty")]
    pub data: Data,
}

impl Permissions {
    fn is_empty(&self) -> bool {
        self == &Self::default()
    }
}

/// `permissions.filesystem`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Filesystem {
    /// Paths it may read.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub read: Vec<String>,
    /// Paths it may write.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub write: Vec<String>,
}

impl Filesystem {
    fn is_empty(&self) -> bool {
        self.read.is_empty() && self.write.is_empty()
    }
}

/// `permissions.data`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Data {
    /// The kinds it may send up, each in [`DATA_LEAVES`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub leaves: Vec<String>,
}

impl Data {
    fn is_empty(&self) -> bool {
        self.leaves.is_empty()
    }
}

/// What an extension needs of the machine.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Resources {
    /// CPUs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu: Option<u32>,
    /// Memory, such as `512MiB`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub memory: String,
    /// Disk for `$CACHE`, such as `1GiB`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub disk: String,
    /// Scheduling niceness, -20 to 19.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nice: Option<i8>,
}

/// How a runtime knows an extension is well.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Health {
    /// The heartbeat file, such as `$STATE/heartbeat.json`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub heartbeat: String,
    /// How often it is written, such as `60s`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub heartbeat_every: String,
    /// When a heartbeat this old means stale, such as `5m`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub stale_after: String,
    /// A command that exits 0 and prints JSON with `ok` when it is well.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check: Option<Check>,
}

/// `health.check`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Check {
    /// The program and its arguments; never a shell.
    pub exec: Vec<String>,
    /// How often, such as `5m`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub every: String,
}

/// A secret an extension is given: a name the target resolves, never a value.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecretRef {
    /// Its name in the manifest.
    pub name: String,
    /// What the target resolves: a Keychain item, a systemd credential, an agent's
    /// secret reference.
    #[serde(rename = "ref")]
    pub reference: String,
    /// The environment variable the program receives it in.
    pub env: String,
}

/// Where an extension may be installed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Targets {
    /// `os/arch` pairs, such as `darwin/arm64`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub platforms: Vec<String>,
    /// `desktop` (this Mac), `host` (a machine over SSH), `agent` (a customer agent).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kinds: Vec<String>,
    /// Programs the target must have.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub needs: Vec<String>,
}

/// How a new version replaces the running one.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Upgrade {
    /// `replace`, or `drain` (wait for the current job).
    #[serde(default)]
    pub strategy: Strategy,
    /// Artifacts kept for rollback.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep_previous: Option<u8>,
    /// Run once before the new version starts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub migrate: Option<Exec>,
    /// Run on rollback.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rollback: Option<Exec>,
}

/// `upgrade.strategy`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Strategy {
    /// Stop the old one, start the new one.
    #[default]
    Replace,
    /// Let the current job finish first.
    Drain,
}

/// A program and its arguments; never a shell.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Exec {
    /// The program and its arguments.
    pub exec: Vec<String>,
}

/// The kinds an extension may send up and what each carries, every one in ADR 0063's
/// non-sensitive classes (identifiers we issued, measurements, hashes, declared names,
/// derived evidence). The source of truth is the platform's
/// `configs/agents/data-classes.toml` (core); this list mirrors it and a kind not in
/// it fails the lint and the manifest. Add one here only when that file has it.
pub const DATA_LEAVES: &[(&str, &str)] = &[
    (
        "ids",
        "identifiers InOrbit issued (account, agent, monitor, check and extension ids and versions)",
    ),
    (
        "measurements",
        "measurements (status codes, timings, sizes, counts, verdicts)",
    ),
    ("hashes", "hashes (payload and policy hashes, chain heads)"),
    (
        "declared.names",
        "names you chose to declare (check names, environment name)",
    ),
    (
        "evidence.claims",
        "derived evidence (claims in the closed vocabulary, never quoted content)",
    ),
    ("check.result", "check results (status, timings, ids)"),
    ("monitor.result", "monitor results (status, timings, ids)"),
    (
        "extension.health",
        "its health (state, heartbeat time, version)",
    ),
];

/// The extension kinds of RFC 0073 and the targets each may be installed on.
pub const TARGET_KINDS: &[(&str, &[&str])] = &[
    ("program", &["desktop", "host", "agent"]),
    ("agent-plugin", &["agent"]),
    ("web", &[]),
    ("connector", &[]),
    ("assistant-plugin", &[]),
];

/// What a data kind carries, in plain words; `None` for one outside ADR 0063's
/// non-sensitive list.
#[must_use]
pub fn data_leaf(kind: &str) -> Option<&'static str> {
    DATA_LEAVES
        .iter()
        .find(|(k, _)| *k == kind)
        .map(|(_, w)| *w)
}

/// Why a manifest or a name was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ManifestError(pub String);

impl Manifest {
    /// Reads and checks a manifest.
    ///
    /// # Errors
    ///
    /// [`ManifestError`] when the bytes are too large, not the JSON shape, or any field
    /// breaks its rule.
    pub fn parse(bytes: &[u8]) -> Result<Self, ManifestError> {
        if bytes.len() > MAX_MANIFEST {
            return Err(ManifestError(format!(
                "the extension manifest is larger than {} KiB",
                MAX_MANIFEST / 1024
            )));
        }
        let m: Self = serde_json::from_slice(bytes)
            .map_err(|e| ManifestError(format!("the extension manifest is not valid: {e}")))?;
        m.check()?;
        Ok(m)
    }

    fn check(&self) -> Result<(), ManifestError> {
        check_name(&self.name)?;
        check_version(&self.version)?;
        check_entrypoint(&self.entrypoint)?;
        if self.scopes.len() > MAX_SCOPES {
            return Err(ManifestError(format!(
                "an extension declares at most {MAX_SCOPES} scopes"
            )));
        }
        for s in &self.scopes {
            check_scope(s)?;
        }
        if self.description.chars().count() > MAX_DESCRIPTION
            || self.description.chars().any(char::is_control)
        {
            return Err(ManifestError(format!(
                "an extension's description is one line of at most {MAX_DESCRIPTION} characters"
            )));
        }
        check_privileges(&self.privileges)?;
        if let Some(p) = &self.permissions
            && (!p.scopes.is_empty() || !p.privileges.is_empty())
        {
            return Err(ManifestError(
                "an artifact's config keeps scopes and privileges at its top level".into(),
            ));
        }
        self.check_runtime()
    }

    /// The rules of the fields a runtime reads (PRD 0084).
    fn check_runtime(&self) -> Result<(), ManifestError> {
        if let Some(r) = &self.runs {
            match r.run_as {
                RunAs::Timer if r.every.trim().is_empty() => {
                    return Err(ManifestError(
                        "runs.every says how often a timer runs, such as 5m".into(),
                    ));
                }
                RunAs::Service | RunAs::Oneshot if !r.every.is_empty() => {
                    return Err(ManifestError(
                        "runs.every is for runs.as = \"timer\" only".into(),
                    ));
                }
                _ => {}
            }
            check_line("runs.every", &r.every)?;
            check_duration("runs.shutdown_grace", &r.shutdown_grace)?;
        }
        if let Some(p) = &self.permissions {
            for n in &p.network {
                check_host_port(n)?;
            }
            for f in p.filesystem.read.iter().chain(&p.filesystem.write) {
                check_line("permissions.filesystem", f)?;
            }
            for e in &p.exec {
                check_program_name("permissions.exec", e)?;
            }
            for l in &p.data.leaves {
                if data_leaf(l).is_none() {
                    return Err(ManifestError(format!(
                        "`{}` is not a non-sensitive data kind (ADR 0063); one of: {}",
                        printable(l),
                        DATA_LEAVES
                            .iter()
                            .map(|(k, _)| *k)
                            .collect::<Vec<_>>()
                            .join(", ")
                    )));
                }
            }
        }
        if let Some(r) = &self.resources {
            if r.nice.is_some_and(|n| !(-20..=19).contains(&n)) {
                return Err(ManifestError("resources.nice is -20 to 19".into()));
            }
            check_size("resources.memory", &r.memory)?;
            check_size("resources.disk", &r.disk)?;
        }
        if let Some(h) = &self.health {
            check_line("health.heartbeat", &h.heartbeat)?;
            check_duration("health.heartbeat_every", &h.heartbeat_every)?;
            check_duration("health.stale_after", &h.stale_after)?;
            if let Some(c) = &h.check {
                check_exec("health.check.exec", &c.exec)?;
                check_duration("health.check.every", &c.every)?;
            }
        }
        check_secrets(&self.secrets)?;
        if let Some(t) = &self.targets {
            for p in &t.platforms {
                check_platform(p)?;
            }
            for k in &t.kinds {
                if !TARGET_KINDS.iter().any(|(_, ts)| ts.contains(&k.as_str())) {
                    return Err(ManifestError(format!(
                        "`{}` is not a target: desktop, host or agent",
                        printable(k)
                    )));
                }
            }
            for n in &t.needs {
                check_program_name("targets.needs", n)?;
            }
        }
        if let Some(u) = &self.upgrade {
            if let Some(m) = &u.migrate {
                check_exec("upgrade.migrate.exec", &m.exec)?;
            }
            if let Some(r) = &u.rollback {
                check_exec("upgrade.rollback.exec", &r.exec)?;
            }
        }
        Ok(())
    }
}

/// One line of at most 256 characters without control characters.
fn check_line(field: &str, v: &str) -> Result<(), ManifestError> {
    if v.chars().count() > 256 || v.chars().any(char::is_control) {
        return Err(ManifestError(format!(
            "{field} is one line of at most 256 characters"
        )));
    }
    Ok(())
}

/// A duration such as `30s`, `5m`, `1h` or `1d`; empty is unset.
#[must_use]
pub fn parse_duration(v: &str) -> Option<std::time::Duration> {
    let (n, unit) = v.split_at(v.find(|c: char| !c.is_ascii_digit())?);
    let n: u64 = n.parse().ok()?;
    let secs = match unit {
        "s" => n,
        "m" => n.checked_mul(60)?,
        "h" => n.checked_mul(3600)?,
        "d" => n.checked_mul(86_400)?,
        _ => return None,
    };
    Some(std::time::Duration::from_secs(secs))
}

fn check_duration(field: &str, v: &str) -> Result<(), ManifestError> {
    if v.is_empty() || parse_duration(v).is_some() {
        Ok(())
    } else {
        Err(ManifestError(format!(
            "{field} `{}` is not a duration such as 30s, 5m or 1h",
            printable(v)
        )))
    }
}

fn check_size(field: &str, v: &str) -> Result<(), ManifestError> {
    let ok = v.is_empty()
        || ["KiB", "MiB", "GiB", "TiB"].iter().any(|u| {
            v.strip_suffix(u).is_some_and(|n| {
                !n.is_empty() && n.len() <= 6 && n.bytes().all(|b| b.is_ascii_digit())
            })
        });
    if ok {
        Ok(())
    } else {
        Err(ManifestError(format!(
            "{field} `{}` is a size such as 512MiB or 10GiB",
            printable(v)
        )))
    }
}

fn check_host_port(v: &str) -> Result<(), ManifestError> {
    let ok = v.rsplit_once(':').is_some_and(|(host, port)| {
        !host.is_empty()
            && host.len() <= 253
            && host
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'*'))
            && port.parse::<u16>().is_ok_and(|p| p > 0)
    });
    if ok {
        Ok(())
    } else {
        Err(ManifestError(format!(
            "permissions.network `{}` is a host and port, such as api.github.com:443",
            printable(v)
        )))
    }
}

fn check_program_name(field: &str, v: &str) -> Result<(), ManifestError> {
    let ok = (1..=64).contains(&v.len())
        && v.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b'+'));
    if ok {
        Ok(())
    } else {
        Err(ManifestError(format!(
            "{field} `{}` is a program's name, such as git, never a path or a command line",
            printable(v)
        )))
    }
}

fn check_exec(field: &str, exec: &[String]) -> Result<(), ManifestError> {
    if exec.is_empty() || exec.len() > 32 {
        return Err(ManifestError(format!(
            "{field} is a program and its arguments, 1 to 32 items"
        )));
    }
    for a in exec {
        check_line(field, a)?;
    }
    Ok(())
}

fn check_platform(v: &str) -> Result<(), ManifestError> {
    const OS: &[&str] = &["darwin", "linux", "windows"];
    const ARCH: &[&str] = &["amd64", "arm64"];
    match v.split_once('/') {
        Some((o, a)) if OS.contains(&o) && ARCH.contains(&a) => Ok(()),
        _ => Err(ManifestError(format!(
            "targets.platforms `{}` is one of darwin, linux or windows with amd64 or arm64, such as linux/amd64",
            printable(v)
        ))),
    }
}

/// An environment variable's name: upper-case letters, digits and `_`, not a digit
/// first.
fn env_name_ok(v: &str) -> bool {
    (1..=64).contains(&v.len())
        && v.bytes()
            .next()
            .is_some_and(|b| b.is_ascii_uppercase() || b == b'_')
        && v.bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
}

fn check_secrets(secrets: &[SecretRef]) -> Result<(), ManifestError> {
    let name_ok = |v: &str| {
        (1..=64).contains(&v.len())
            && v.bytes().next().is_some_and(|b| b.is_ascii_lowercase())
            && v.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-'))
    };
    let ref_ok = |v: &str| {
        (1..=128).contains(&v.len())
            && v.bytes().next().is_some_and(|b| b.is_ascii_alphanumeric())
            && v.bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b'/'))
    };
    for (i, s) in secrets.iter().enumerate() {
        if !name_ok(&s.name) {
            return Err(ManifestError(format!(
                "secrets.name `{}` has lower-case letters, digits, '_' and '-'",
                printable(&s.name)
            )));
        }
        if !ref_ok(&s.reference) {
            return Err(ManifestError(format!(
                "secrets.ref of {} is a name the target resolves, such as github-token; never a value",
                s.name
            )));
        }
        if !env_name_ok(&s.env) {
            return Err(ManifestError(format!(
                "secrets.env of {} is an environment variable such as GITHUB_TOKEN",
                s.name
            )));
        }
        if s.env.starts_with("IOHR_") || s.env.starts_with("INORBIT_") {
            return Err(ManifestError(format!(
                "secrets.env of {} may not start with IOHR_ or INORBIT_: those are iohr's",
                s.name
            )));
        }
        if secrets[..i]
            .iter()
            .any(|o| o.name == s.name || o.env == s.env)
        {
            return Err(ManifestError(format!(
                "secret {} or its variable {} is declared twice",
                s.name, s.env
            )));
        }
    }
    Ok(())
}

/// Checks a list of privileges: at most [`MAX_PRIVILEGES`], each a capability the
/// kernel defines ([`CAPABILITIES`]), none twice.
///
/// # Errors
///
/// [`ManifestError`] naming the rule.
pub fn check_privileges(privileges: &[String]) -> Result<(), ManifestError> {
    if privileges.len() > MAX_PRIVILEGES {
        return Err(ManifestError(format!(
            "an extension declares at most {MAX_PRIVILEGES} privileges"
        )));
    }
    for (i, p) in privileges.iter().enumerate() {
        if privilege(p).is_none() {
            return Err(ManifestError(format!(
                "`{}` is not a Linux capability such as CAP_NET_ADMIN",
                printable(p)
            )));
        }
        if privileges[..i].contains(p) {
            return Err(ManifestError(format!("{p} is declared twice")));
        }
    }
    Ok(())
}

/// What a capability lets a program do, in plain words; `None` for a name the kernel
/// does not define.
#[must_use]
pub fn privilege(name: &str) -> Option<&'static str> {
    CAPABILITIES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, words)| *words)
}

/// Checks an extension name: 1 to 32 lower-case letters, digits and `-`, starting with
/// a letter, and not one of `iohr`'s own commands.
///
/// # Errors
///
/// [`ManifestError`] naming the rule.
pub fn check_name(name: &str) -> Result<(), ManifestError> {
    let ok = (1..=32).contains(&name.len())
        && name.bytes().next().is_some_and(|b| b.is_ascii_lowercase())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !name.ends_with('-');
    if !ok {
        return Err(ManifestError(
            "an extension name has 1 to 32 lower-case letters, digits and '-', and starts with a letter"
                .into(),
        ));
    }
    if RESERVED.contains(&name) {
        return Err(ManifestError(format!(
            "`{name}` is a command of iohr itself and cannot be an extension"
        )));
    }
    Ok(())
}

/// Checks a version: semantic versioning without build metadata, at most 64 characters.
///
/// # Errors
///
/// [`ManifestError`] when it is not one.
pub fn check_version(version: &str) -> Result<semver::Version, ManifestError> {
    let v = (version.len() <= 64)
        .then(|| semver::Version::parse(version).ok())
        .flatten()
        .filter(|v| v.build.is_empty())
        .ok_or_else(|| {
            ManifestError(format!(
                "`{}` is not a version such as 1.2.3",
                printable(version)
            ))
        })?;
    Ok(v)
}

fn check_entrypoint(e: &str) -> Result<(), ManifestError> {
    let parts: Vec<&str> = e.split('/').collect();
    let ok = (1..=256).contains(&e.len())
        && parts.iter().all(|p| {
            !p.is_empty()
                && *p != "."
                && *p != ".."
                && p.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        });
    if ok {
        Ok(())
    } else {
        Err(ManifestError(
            "an extension's entrypoint is a relative path inside its layer, without `..`".into(),
        ))
    }
}

/// Checks a scope: `resource:action`, lower-case.
///
/// # Errors
///
/// [`ManifestError`] when it is not one.
pub fn check_scope(s: &str) -> Result<(), ManifestError> {
    let part = |p: &str| {
        !p.is_empty()
            && p.bytes().next().is_some_and(|b| b.is_ascii_lowercase())
            && p.bytes().all(|b| {
                b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-' | b'.')
            })
    };
    match s.split_once(':') {
        Some((r, a)) if s.len() <= 64 && part(r) && part(a) => Ok(()),
        _ => Err(ManifestError(format!(
            "`{}` is not a scope such as agents:write",
            printable(s)
        ))),
    }
}

/// The source manifest's file name in an extension's folder.
pub const SOURCE_FILE: &str = "extension.toml";

/// An extension's source manifest, `extension.toml` in its folder (PRD 0084): the
/// artifact's config plus what only building it needs. Every key is known: an unknown
/// one is refused, so a typo never silently drops a permission.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct SourceManifest {
    /// `publisher/name`, as the catalogue lists it.
    pub id: String,
    /// Its version.
    pub version: String,
    /// The RFC 0073 kind; this slice builds `program`.
    #[serde(default = "program")]
    pub kind: String,
    /// How it arrives (RFC 0073.1); `program` for a program.
    #[serde(default = "program")]
    pub delivery: String,
    /// The program's path inside the artifact.
    pub entrypoint: String,
    /// Arguments it is started with.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    /// One line about what it does.
    #[serde(default)]
    pub description: String,
    /// It sets up a system service of its own (see [`Manifest::service`]).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub service: bool,
    /// The API versions it works with, such as `>=2026.10 <2027.01`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub api: String,
    /// See [`Manifest::runs`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runs: Option<Runs>,
    /// See [`Permissions`]; `scopes` and `privileges` live here in the source.
    #[serde(default)]
    pub permissions: Permissions,
    /// See [`Manifest::resources`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resources: Option<Resources>,
    /// See [`Manifest::health`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health: Option<Health>,
    /// See [`Manifest::secrets`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secrets: Vec<SecretRef>,
    /// See [`Manifest::targets`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub targets: Option<Targets>,
    /// See [`Manifest::upgrade`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upgrade: Option<Upgrade>,
    /// How `iohr ext dev` runs it while it is being built.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dev: Option<Dev>,
}

fn program() -> String {
    "program".into()
}

/// `[dev]`: how `iohr ext dev` runs the folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dev {
    /// The language the folder is written in.
    pub lang: Lang,
}

/// The languages `iohr ext init` scaffolds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    /// A Cargo project on the `inorbithr` crate: `cargo run`.
    Rust,
    /// A Python package under `src/`: `python3 -m`.
    Python,
    /// A POSIX shell script: run as it is.
    Shell,
}

impl Lang {
    /// Its name in the manifest.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::Python => "python",
            Self::Shell => "shell",
        }
    }
}

impl SourceManifest {
    /// Reads `extension.toml`. Unknown keys are refused.
    ///
    /// # Errors
    ///
    /// [`ManifestError`] when it is not TOML of this shape.
    pub fn parse(text: &str) -> Result<Self, ManifestError> {
        if text.len() > MAX_MANIFEST {
            return Err(ManifestError(format!(
                "{SOURCE_FILE} is larger than {} KiB",
                MAX_MANIFEST / 1024
            )));
        }
        toml::from_str(text).map_err(|e| {
            let at = e
                .span()
                .map(|s| {
                    let line = text[..s.start.min(text.len())].matches('\n').count() + 1;
                    format!(" (line {line})")
                })
                .unwrap_or_default();
            ManifestError(format!(
                "{SOURCE_FILE}{at}: {}",
                e.message().trim_end_matches('\n').replace('\n', " ")
            ))
        })
    }

    /// The publisher and the name of `id`.
    ///
    /// # Errors
    ///
    /// [`ManifestError`] when `id` is not `publisher/name` with a valid name.
    pub fn split_id(&self) -> Result<(&str, &str), ManifestError> {
        split_id(&self.id)
    }

    /// The artifact's config: what the registry, the installer and the runtime read.
    ///
    /// # Errors
    ///
    /// [`ManifestError`] when the result breaks a rule of [`Manifest`].
    pub fn to_manifest(&self) -> Result<Manifest, ManifestError> {
        let (_, name) = self.split_id()?;
        let mut permissions = self.permissions.clone();
        let scopes = std::mem::take(&mut permissions.scopes);
        let privileges = std::mem::take(&mut permissions.privileges);
        let m = Manifest {
            name: name.to_owned(),
            version: self.version.clone(),
            entrypoint: self.entrypoint.clone(),
            scopes,
            description: self.description.clone(),
            privileges,
            service: self.service,
            runs: self.runs.clone(),
            permissions: (!permissions.is_empty()).then_some(permissions),
            resources: self.resources.clone(),
            health: self.health.clone(),
            secrets: self.secrets.clone(),
            targets: self.targets.clone(),
            upgrade: self.upgrade.clone(),
        };
        m.check()?;
        Ok(m)
    }
}

/// Splits `publisher/name`: the publisher 1 to 39 lower-case letters, digits and `-`,
/// the name an extension name ([`check_name`]).
///
/// # Errors
///
/// [`ManifestError`] naming the rule.
pub fn split_id(id: &str) -> Result<(&str, &str), ManifestError> {
    let Some((publisher, name)) = id.split_once('/') else {
        return Err(ManifestError(format!(
            "`{}` is not publisher/name, such as inorbit/hello",
            printable(id)
        )));
    };
    let ok = (1..=39).contains(&publisher.len())
        && publisher
            .bytes()
            .next()
            .is_some_and(|b| b.is_ascii_alphanumeric())
        && publisher
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if !ok {
        return Err(ManifestError(format!(
            "the publisher `{}` has 1 to 39 lower-case letters, digits and '-'",
            printable(publisher)
        )));
    }
    check_name(name)?;
    Ok((publisher, name))
}

/// A value from the registry, safe to put in a message: no control characters, at most
/// 64 characters.
pub(crate) fn printable(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).take(64).collect()
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory as _;

    use super::{CAPABILITIES, Manifest, RESERVED, check_name, privilege};

    /// Every built-in command, and each of its aliases, is reserved: read from the clap
    /// tree so a new command cannot be shadowed by an extension of the same name.
    #[test]
    fn every_built_in_command_is_reserved() {
        let cli = crate::cli::Cli::command();
        let mut names = vec!["help".to_owned()];
        for sub in cli.get_subcommands() {
            names.push(sub.get_name().to_owned());
            names.extend(sub.get_all_aliases().map(str::to_owned));
        }
        for name in &names {
            assert!(
                RESERVED.contains(&name.as_str()),
                "`iohr {name}` is a built-in command missing from RESERVED"
            );
        }
        let mut sorted = RESERVED.to_vec();
        sorted.sort_unstable();
        assert_eq!(sorted, RESERVED, "keep RESERVED sorted");
    }

    fn manifest(v: &serde_json::Value) -> Result<Manifest, String> {
        Manifest::parse(v.to_string().as_bytes()).map_err(|e| e.0)
    }

    #[test]
    fn reads_the_agent_manifest() {
        let m = manifest(&serde_json::json!({
            "name": "agent", "version": "0.1.0", "entrypoint": "iohr-agent",
            "scopes": ["agents:write", "domains:read"], "description": "The InOrbit agent",
            "later": "an unknown field is ignored"
        }))
        .unwrap();
        assert_eq!(m.scopes, ["agents:write", "domains:read"]);
    }

    #[test]
    fn refuses_what_breaks_a_rule() {
        let base =
            serde_json::json!({"name": "agent", "version": "1.0.0", "entrypoint": "bin/agent"});
        for (field, bad) in [
            ("name", serde_json::json!("login")),
            ("name", serde_json::json!("Agent")),
            ("name", serde_json::json!("a/b")),
            ("version", serde_json::json!("1.0")),
            ("version", serde_json::json!("1.0.0+build")),
            ("entrypoint", serde_json::json!("../agent")),
            ("entrypoint", serde_json::json!("/usr/bin/agent")),
            ("entrypoint", serde_json::json!("bin//agent")),
            ("scopes", serde_json::json!(["Agents:write"])),
            ("scopes", serde_json::json!(["agents"])),
            ("description", serde_json::json!("two\nlines")),
            ("description", serde_json::json!("\u{1b}[31mred")),
        ] {
            let mut v = base.clone();
            v[field] = bad.clone();
            assert!(manifest(&v).is_err(), "{field} = {bad}");
        }
        assert!(manifest(&base).is_ok());
        assert!(Manifest::parse(&vec![b' '; 70 * 1024]).is_err());
    }

    #[test]
    fn a_system_service_is_declared_and_absent_means_none() {
        let base = serde_json::json!({"name": "capture", "version": "1.0.0", "entrypoint": "iohr-capture"});
        let m = manifest(&base).unwrap();
        assert!(!m.service);
        assert!(
            !serde_json::to_string(&m).unwrap().contains("service"),
            "an older manifest keeps its exact shape"
        );
        let mut v = base.clone();
        v["service"] = serde_json::json!(true);
        v["privileges"] = serde_json::json!(["CAP_BPF", "CAP_PERFMON", "CAP_NET_ADMIN"]);
        let m = manifest(&v).unwrap();
        assert!(m.service);
        assert!(
            serde_json::to_string(&m)
                .unwrap()
                .contains("\"service\":true")
        );
        v["service"] = serde_json::json!("yes");
        assert!(manifest(&v).is_err(), "service is a boolean");
    }

    #[test]
    fn privileges_are_known_capabilities_at_most_16_never_twice() {
        let base =
            serde_json::json!({"name": "capture", "version": "1.0.0", "entrypoint": "capture"});
        // Absent is none, and an older manifest keeps its exact shape when written back.
        let m = manifest(&base).unwrap();
        assert_eq!(m.privileges, [] as [String; 0]);
        assert!(
            !serde_json::to_string(&m).unwrap().contains("privileges"),
            "an empty list is not written"
        );

        let mut v = base.clone();
        v["privileges"] = serde_json::json!(["CAP_BPF", "CAP_PERFMON", "CAP_NET_ADMIN"]);
        let m = manifest(&v).unwrap();
        assert_eq!(m.privileges, ["CAP_BPF", "CAP_PERFMON", "CAP_NET_ADMIN"]);
        let all: Vec<&str> = CAPABILITIES.iter().take(16).map(|(n, _)| *n).collect();
        v["privileges"] = serde_json::json!(all);
        assert!(manifest(&v).is_ok());

        for bad in [
            serde_json::json!(["CAP_BPF", "CAP_BPF"]),
            serde_json::json!(["cap_bpf"]),
            serde_json::json!(["BPF"]),
            serde_json::json!(["CAP_EVERYTHING"]),
            serde_json::json!(["CAP_BPF\u{1b}[31m"]),
            serde_json::json!([""]),
            serde_json::json!("CAP_BPF"),
            serde_json::json!(
                CAPABILITIES
                    .iter()
                    .take(17)
                    .map(|(n, _)| *n)
                    .collect::<Vec<_>>()
            ),
        ] {
            let mut v = base.clone();
            v["privileges"] = bad.clone();
            assert!(manifest(&v).is_err(), "privileges = {bad}");
        }
    }

    /// The table is the kernel's list: 41 names (0 to `CAP_LAST_CAP` = 40, Linux 5.9 and
    /// later), each once, each with words to show.
    #[test]
    fn the_capability_table_is_the_kernels() {
        assert_eq!(CAPABILITIES.len(), 41);
        for (i, (name, words)) in CAPABILITIES.iter().enumerate() {
            assert!(name.starts_with("CAP_"), "{name}");
            assert!(!words.is_empty() && !words.ends_with('.'), "{name}");
            assert!(
                CAPABILITIES[..i].iter().all(|(n, _)| n != name),
                "{name} twice"
            );
        }
        assert_eq!(CAPABILITIES[0].0, "CAP_CHOWN");
        assert_eq!(CAPABILITIES[40].0, "CAP_CHECKPOINT_RESTORE");
        assert_eq!(
            privilege("CAP_BPF"),
            Some("load eBPF programs into the kernel")
        );
        assert_eq!(privilege("CAP_ALL"), None);
    }

    #[test]
    fn names() {
        for ok in ["agent", "a", "load-test2"] {
            assert!(check_name(ok).is_ok(), "{ok}");
        }
        for bad in ["", "ext", "sdk", "-a", "a-", "1a", "a_b", &"a".repeat(33)] {
            assert!(check_name(bad).is_err(), "{bad}");
        }
    }
}
