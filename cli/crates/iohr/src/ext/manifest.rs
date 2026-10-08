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
    "decisions",
    "domains",
    "ext",
    "help",
    "iohr",
    "lab",
    "login",
    "logout",
    "openapi",
    "profile",
    // `iohr rfc` until core ADR 0056 (2026-10-08): kept from extensions so a script that
    // still calls it fails as unknown rather than running someone else's program.
    "rfc",
    "sdk",
    "token",
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
        check_privileges(&self.privileges)
    }
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
