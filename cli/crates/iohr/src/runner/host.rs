//! What the runner tells the coordinator about its machine on every heartbeat (platform
//! RFC 0197, ADR 0036): `RunnerCapabilities.resources` (cores, memory, disk) and
//! `HostSignals` (load, pressure, disk free). Placement reads the resources, and sheds
//! build turns from a host whose disk is under its pool's floor.
//!
//! Read from the operating system with no new dependency: `/proc` on Linux,
//! `/usr/sbin/sysctl` on macOS (by absolute path), `df` for disks. A value the host does
//! not give is left out (zero for a resource, absent for a signal), never guessed.

use std::path::Path;
use std::time::Duration;

use serde_json::{Value, json};

use super::settings::{self, DiskSpace};

/// One disk the signals name: its mount point and space.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Disk {
    pub(crate) mount: String,
    pub(crate) space: DiskSpace,
}

/// The machine as one heartbeat reports it.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Host {
    pub(crate) cpu_cores: u32,
    pub(crate) memory_bytes: u64,
    /// The disks the runner works on: the root volume, and the build volume when it has
    /// one. The first one's size is `resources.disk_bytes`.
    pub(crate) disks: Vec<Disk>,
    pub(crate) load_1m: Option<f64>,
    /// Linux pressure stall information, `some avg10` in percent.
    pub(crate) psi_cpu: Option<f64>,
    pub(crate) psi_memory: Option<f64>,
    pub(crate) psi_io: Option<f64>,
}

impl Host {
    /// `RunnerCapabilities` as the heartbeat body carries it. No labels are declared yet,
    /// so the runner asks for none.
    pub(crate) fn capabilities_json(&self) -> Value {
        json!({
            "labels": [],
            "resources": {
                "cpu_cores": self.cpu_cores,
                "memory_bytes": self.memory_bytes.to_string(),
                "disk_bytes": self.disks.first().map_or(0, |d| d.space.total).to_string(),
            },
        })
    }

    /// `HostSignals`, or `None` when the host gave no load: the field is not optional in
    /// the contract, and a zero would read as an idle machine. Only values the
    /// coordinator accepts are sent, since one bad value refuses the whole beat.
    pub(crate) fn signals_json(&self) -> Option<Value> {
        let load = self.load_1m.filter(|l| (0.0..=MAX_LOAD).contains(l))?;
        let disks: Vec<Value> = self
            .disks
            .iter()
            .filter(|d| mount_ok(&d.mount) && d.space.free <= d.space.total)
            .take(MAX_READINGS)
            .map(|d| {
                json!({
                    "mount": d.mount,
                    "free_bytes": d.space.free.to_string(),
                    "total_bytes": d.space.total.to_string(),
                })
            })
            .collect();
        let mut s = json!({ "load_1m": load, "disks": disks });
        if let Value::Object(m) = &mut s {
            for (k, v) in [
                ("psi_cpu", self.psi_cpu),
                ("psi_memory", self.psi_memory),
                ("psi_io", self.psi_io),
            ] {
                if let Some(v) = v.filter(|v| (0.0..=100.0).contains(v)) {
                    m.insert(k.into(), json!(v));
                }
            }
        }
        Some(s)
    }
}

/// The coordinator's bounds (`pools_api`): load up to a million, at most 64 readings, a
/// mount of 1 to 128 printable characters.
const MAX_LOAD: f64 = 1_000_000.0;
const MAX_READINGS: usize = 64;
const MAX_MOUNT: usize = 128;

fn mount_ok(m: &str) -> bool {
    !m.is_empty() && m.len() <= MAX_MOUNT && !m.chars().any(char::is_control)
}

/// Reads the machine now. `build_volume` is the build-disk guard's volume, when set.
pub(crate) async fn probe(build_volume: Option<&Path>) -> Host {
    let mut disks = Vec::new();
    for p in std::iter::once(Path::new("/")).chain(build_volume) {
        if let Some((mount, space)) = settings::disk_space(p).await
            && !disks.iter().any(|d: &Disk| d.mount == mount)
        {
            disks.push(Disk { mount, space });
        }
    }
    let cpu_cores = std::thread::available_parallelism()
        .ok()
        .and_then(|n| u32::try_from(n.get()).ok())
        .unwrap_or(0);
    if cfg!(target_os = "linux") {
        Host {
            cpu_cores,
            memory_bytes: read("/proc/meminfo")
                .await
                .as_deref()
                .and_then(meminfo_total)
                .unwrap_or(0),
            disks,
            load_1m: read("/proc/loadavg")
                .await
                .as_deref()
                .and_then(first_number),
            psi_cpu: read("/proc/pressure/cpu")
                .await
                .as_deref()
                .and_then(psi_some_avg10),
            psi_memory: read("/proc/pressure/memory")
                .await
                .as_deref()
                .and_then(psi_some_avg10),
            psi_io: read("/proc/pressure/io")
                .await
                .as_deref()
                .and_then(psi_some_avg10),
        }
    } else {
        Host {
            cpu_cores,
            memory_bytes: sysctl("hw.memsize")
                .await
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(0),
            disks,
            load_1m: sysctl("vm.loadavg").await.as_deref().and_then(first_number),
            ..Host::default()
        }
    }
}

async fn read(path: &str) -> Option<String> {
    tokio::fs::read_to_string(path).await.ok()
}

/// macOS keeps `sysctl` in `/usr/sbin`, which a launchd agent's `PATH` often leaves
/// out: by bare name the Mac runner sent no memory and no load (2026-10-29).
const SYSCTL: &str = "/usr/sbin/sysctl";

/// `sysctl -n <name>`, given 5 s.
async fn sysctl(name: &str) -> Option<String> {
    let out = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::process::Command::new(SYSCTL)
            .arg("-n")
            .arg(name)
            .kill_on_drop(true)
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// `MemTotal:  65743424 kB` in bytes.
fn meminfo_total(text: &str) -> Option<u64> {
    let line = text.lines().find(|l| l.starts_with("MemTotal:"))?;
    let kib: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    kib.checked_mul(1024)
}

/// The first number in `/proc/loadavg` (`0.52 0.58 0.59 1/389 1234`) or macOS
/// `vm.loadavg` (`{ 1.52 1.61 1.70 }`).
fn first_number(text: &str) -> Option<f64> {
    text.split_whitespace()
        .find_map(|w| w.parse::<f64>().ok())
        .filter(|v| v.is_finite() && *v >= 0.0)
}

/// `some avg10=1.23 avg60=...` from a `/proc/pressure/*` file.
fn psi_some_avg10(text: &str) -> Option<f64> {
    let line = text.lines().find(|l| l.starts_with("some "))?;
    line.split_whitespace()
        .find_map(|w| w.strip_prefix("avg10="))?
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_host_files_parse() {
        assert_eq!(
            meminfo_total("MemTotal:       65743424 kB\nMemFree: 1 kB\n"),
            Some(65_743_424 * 1024)
        );
        assert_eq!(first_number("0.52 0.58 0.59 1/389 1234\n"), Some(0.52));
        assert_eq!(first_number("{ 1.52 1.61 1.70 }\n"), Some(1.52));
        assert_eq!(first_number("{ }"), None);
        assert_eq!(
            psi_some_avg10(
                "some avg10=12.50 avg60=3.00 avg300=1.00 total=9\nfull avg10=1.00 avg60=0 avg300=0 total=1\n"
            ),
            Some(12.5)
        );
        assert_eq!(psi_some_avg10("full avg10=1.00"), None);
    }

    #[test]
    fn resources_and_signals_carry_what_the_host_gave() {
        let h = Host {
            cpu_cores: 48,
            memory_bytes: 64 << 30,
            disks: vec![
                Disk {
                    mount: "/".into(),
                    space: DiskSpace {
                        free: 10,
                        total: 100,
                    },
                },
                Disk {
                    mount: "x".repeat(MAX_MOUNT + 1),
                    space: DiskSpace { free: 1, total: 2 },
                },
            ],
            load_1m: Some(3.5),
            psi_cpu: Some(4.0),
            psi_memory: None,
            psi_io: Some(250.0),
        };
        assert_eq!(
            h.capabilities_json(),
            json!({"labels": [], "resources": {"cpu_cores": 48, "memory_bytes": (64_u64 << 30).to_string(), "disk_bytes": "100"}})
        );
        assert_eq!(
            h.signals_json(),
            Some(json!({
                "load_1m": 3.5,
                "disks": [{"mount": "/", "free_bytes": "10", "total_bytes": "100"}],
                "psi_cpu": 4.0,
            }))
        );
        assert_eq!(Host::default().signals_json(), None, "no load, no signals");
        let wild = Host {
            load_1m: Some(f64::NAN),
            ..h.clone()
        };
        assert_eq!(
            wild.signals_json(),
            None,
            "a load the coordinator refuses is not sent"
        );
    }

    // Memory, disks and load come from /proc, sysctl and df: Windows has none of them
    // and reports cores only.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[tokio::test]
    async fn this_machine_reports_cores_memory_and_root() {
        let h = probe(None).await;
        assert!(h.cpu_cores > 0);
        assert!(h.memory_bytes > 0);
        assert!(h.disks.first().is_some_and(|d| d.space.total > 0));
        assert!(h.load_1m.is_some());
    }

    // A launchd agent's PATH often has no /usr/sbin: by bare name the Mac runner
    // reported 0 bytes of memory and sent no signals.
    #[test]
    fn sysctl_is_named_by_absolute_path() {
        assert!(std::path::Path::new(SYSCTL).is_absolute());
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn sysctl_answers_with_an_empty_path() {
        let out = tokio::process::Command::new(SYSCTL)
            .arg("-n")
            .arg("hw.memsize")
            .env_clear()
            .output()
            .await;
        assert!(out.is_ok_and(|o| o.status.success()));
    }
}
