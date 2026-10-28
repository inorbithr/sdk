//! The runner's settings (`docs/config.md` section 4.4): the `[runner]` table of
//! `config.toml`, a profile's `[profiles.<name>.runner]` over it, and the command-line
//! flags over both. `iohr runner run` resolves them at start and again on SIGHUP, and
//! sends capacity, version and kinds on every heartbeat (platform RFC 0120), so the slots
//! a runner offers change without registering again.
//!
//! The build-disk guard lives here too: while the build volume has less than
//! [`BUILD_FLOOR_BYTES`] free, the runner declares itself without `build`, so the service
//! offers it no new build turn. Turns already running are never touched.

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use iohr_auth::RunnerConfig;

/// The most turns one runner holds at once; the service clamps to the same.
pub(crate) const MAX_CAPACITY: u32 = 32;
/// The turn classes a runner may declare (the service's `coordinate` and `build`).
pub(crate) const CLASSES: [&str; 2] = ["coordinate", "build"];
/// The class the build-disk guard drops.
pub(crate) const BUILD: &str = "build";
/// Declared alone when the guard leaves no class: a kind no turn has, so the runner is
/// offered nothing new. An empty list would not do: the service reads it as "not
/// declared" and keeps the kinds of the beat before.
pub(crate) const NOTHING: &str = "none";
/// The volume build turns use on a build server.
pub(crate) const BUILD_VOLUME: &str = "/mnt/build";
/// Under this much free space on [`BUILD_VOLUME`], no new build turn: 100 GB.
pub(crate) const BUILD_FLOOR_BYTES: u64 = 100 * 1_000_000_000;

/// Where a setting's value came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Source {
    Flag,
    /// `[profiles.<name>.runner]`.
    Profile(String),
    /// `[runner]`.
    File,
    Default,
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Flag => f.write_str("flag"),
            Self::Profile(p) => write!(f, "[profiles.{p}.runner]"),
            Self::File => f.write_str("[runner]"),
            Self::Default => f.write_str("default"),
        }
    }
}

/// One setting in effect and its source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Setting<T> {
    pub(crate) value: T,
    pub(crate) source: Source,
}

/// The flags of `iohr runner register` and `run` that a setting may come from.
#[derive(Debug, Clone, Default)]
pub(crate) struct Flags {
    pub(crate) capacity: Option<u32>,
    pub(crate) kinds: Option<Vec<String>>,
    /// `--always-on` given (it can only say yes).
    pub(crate) always_on: bool,
    pub(crate) name: Option<String>,
}

/// The settings in effect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Effective {
    /// `None` when set nowhere: the runner keeps the capacity it registered with and
    /// sends none, so the service leaves it alone.
    pub(crate) capacity: Option<Setting<u32>>,
    pub(crate) kinds: Setting<Vec<String>>,
    pub(crate) always_on: Setting<bool>,
    /// `None` when set nowhere: register uses the host name, run the key this machine holds.
    pub(crate) name: Option<Setting<String>>,
    /// `None` when set nowhere: with no kube context either, approved host actions are
    /// refused (every `kubectl` call is pinned to the configured cluster).
    pub(crate) kube_context: Option<Setting<String>>,
    /// The kubeconfig file the pinned `kubectl` reads (absolute path), if any.
    pub(crate) kubeconfig: Option<Setting<String>>,
}

/// Resolves each key: flag, then the profile's table, then `[runner]`, then the default.
///
/// # Errors
///
/// A message naming the key and where the bad value came from: a capacity outside 1 to
/// 32, an empty kinds list, or a kind other than `coordinate` and `build`.
pub(crate) fn resolve(
    file: &RunnerConfig,
    profile: Option<(&str, &RunnerConfig)>,
    flags: &Flags,
) -> Result<Effective, String> {
    fn pick<T: Clone>(
        flag: Option<T>,
        profile: Option<(&str, &RunnerConfig)>,
        file: &RunnerConfig,
        get: impl Fn(&RunnerConfig) -> Option<T>,
    ) -> Option<Setting<T>> {
        if let Some(value) = flag {
            return Some(Setting {
                value,
                source: Source::Flag,
            });
        }
        if let Some((name, p)) = profile
            && let Some(value) = get(p)
        {
            return Some(Setting {
                value,
                source: Source::Profile(name.to_owned()),
            });
        }
        get(file).map(|value| Setting {
            value,
            source: Source::File,
        })
    }
    let capacity = pick(flags.capacity, profile, file, |c| c.capacity);
    if let Some(c) = &capacity
        && !(1..=MAX_CAPACITY).contains(&c.value)
    {
        return Err(format!(
            "runner capacity {} ({}) is outside 1 to {MAX_CAPACITY}",
            c.value, c.source
        ));
    }
    let kinds = match pick(flags.kinds.clone(), profile, file, |c| c.kinds.clone()) {
        Some(k) => Setting {
            value: kinds_of(&k.value).map_err(|e| format!("runner kinds ({}): {e}", k.source))?,
            source: k.source,
        },
        None => Setting {
            value: CLASSES.iter().map(|c| (*c).to_owned()).collect(),
            source: Source::Default,
        },
    };
    let always_on = pick(flags.always_on.then_some(true), profile, file, |c| {
        c.always_on
    })
    .unwrap_or(Setting {
        value: false,
        source: Source::Default,
    });
    let name = pick(flags.name.clone(), profile, file, |c| c.name.clone());
    if let Some(n) = &name
        && n.value.trim().is_empty()
    {
        return Err(format!("runner name ({}) is empty", n.source));
    }
    let kube_context = pick(None, profile, file, |c| c.kube_context.clone());
    let kubeconfig = pick(None, profile, file, |c| c.kubeconfig.clone());
    Ok(Effective {
        capacity,
        kinds,
        always_on,
        name,
        kube_context,
        kubeconfig,
    })
}

/// The kinds, checked and in the order given, once each.
fn kinds_of(kinds: &[String]) -> Result<Vec<String>, String> {
    if kinds.is_empty() {
        return Err("name at least one of coordinate, build".into());
    }
    let mut out: Vec<String> = Vec::new();
    for k in kinds {
        let k = k.trim();
        if !CLASSES.contains(&k) {
            return Err(format!("{k:?} is not a turn class: coordinate or build"));
        }
        if !out.iter().any(|o| o == k) {
            out.push(k.to_owned());
        }
    }
    Ok(out)
}

/// What a heartbeat declares: sent on every beat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Declared {
    /// `None`: the registered capacity stands, and none is sent.
    pub(crate) capacity: Option<u32>,
    pub(crate) version: String,
    pub(crate) kinds: Vec<String>,
}

impl Declared {
    pub(crate) fn of(e: &Effective) -> Self {
        Self {
            capacity: e.capacity.as_ref().map(|c| c.value),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            kinds: e.kinds.value.clone(),
        }
    }

    /// These kinds, without `build` while the build volume is under the floor. An
    /// unknown free space (no such volume, `df` failed) keeps them as they are.
    #[must_use]
    pub(crate) fn guarded(mut self, free: Option<u64>, floor: u64) -> Self {
        if free.is_some_and(|f| f < floor) {
            self = self.without_build();
        }
        self
    }

    /// These kinds, without `build`.
    #[must_use]
    pub(crate) fn without_build(mut self) -> Self {
        self.kinds.retain(|k| k != BUILD);
        if self.kinds.is_empty() {
            self.kinds.push(NOTHING.to_owned());
        }
        self
    }
}

/// The build-disk guard: a volume and the free space it must keep.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BuildFloor {
    pub(crate) volume: PathBuf,
    pub(crate) bytes: u64,
}

impl Default for BuildFloor {
    fn default() -> Self {
        Self {
            volume: PathBuf::from(BUILD_VOLUME),
            bytes: BUILD_FLOOR_BYTES,
        }
    }
}

/// Free bytes on the volume holding `path`, from `df -Pk` (POSIX output, Linux and
/// macOS); `None` when the path does not exist or `df` does not answer in 5 s.
pub(crate) async fn free_bytes(path: &Path) -> Option<u64> {
    if !path.exists() {
        return None;
    }
    let out = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::process::Command::new("df")
            .arg("-Pk")
            .arg(path)
            .kill_on_drop(true)
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    if !out.status.success() {
        return None;
    }
    df_available(&String::from_utf8_lossy(&out.stdout))
}

/// The available bytes in `df -Pk` output: the fourth column of the last line, in KiB.
fn df_available(text: &str) -> Option<u64> {
    let line = text.lines().filter(|l| !l.trim().is_empty()).nth(1)?;
    let kib: u64 = line.split_whitespace().nth(3)?.parse().ok()?;
    kib.checked_mul(1024)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(capacity: Option<u32>, kinds: Option<&[&str]>) -> RunnerConfig {
        let mut c = RunnerConfig::default();
        c.capacity = capacity;
        c.kinds = kinds.map(|k| k.iter().map(|s| (*s).to_owned()).collect());
        c
    }

    #[test]
    fn a_flag_beats_the_profile_which_beats_the_file_which_beats_the_default() {
        let file = table(Some(4), Some(&["coordinate"]));
        let work = table(Some(8), None);
        let e = resolve(&file, Some(("work", &work)), &Flags::default()).unwrap();
        assert_eq!(
            e.capacity,
            Some(Setting {
                value: 8,
                source: Source::Profile("work".into())
            })
        );
        assert_eq!(e.kinds.value, ["coordinate"]);
        assert_eq!(e.kinds.source, Source::File);
        assert_eq!(e.always_on.source, Source::Default);
        assert_eq!(e.name, None);

        let flags = Flags {
            capacity: Some(2),
            always_on: true,
            ..Flags::default()
        };
        let e = resolve(&file, Some(("work", &work)), &flags).unwrap();
        assert_eq!(e.capacity.unwrap().source, Source::Flag);
        assert_eq!(
            e.always_on,
            Setting {
                value: true,
                source: Source::Flag
            }
        );

        let e = resolve(&RunnerConfig::default(), None, &Flags::default()).unwrap();
        assert_eq!(e.capacity, None, "unset: the registered capacity stands");
        assert_eq!(e.kinds.value, ["coordinate", "build"]);
        assert_eq!(e.kinds.source, Source::Default);
        assert_eq!(Declared::of(&e).capacity, None);
    }

    #[test]
    fn a_bad_value_is_refused_naming_its_source() {
        let e = resolve(&table(Some(33), None), None, &Flags::default()).unwrap_err();
        assert!(e.contains("33") && e.contains("[runner]"), "{e}");
        let e = resolve(&table(Some(0), None), None, &Flags::default()).unwrap_err();
        assert!(e.contains("outside 1 to 32"), "{e}");
        let e = resolve(&table(None, Some(&["bild"])), None, &Flags::default()).unwrap_err();
        assert!(e.contains("bild"), "{e}");
        let e = resolve(&table(None, Some(&[])), None, &Flags::default()).unwrap_err();
        assert!(e.contains("at least one"), "{e}");
        let flags = Flags {
            kinds: Some(vec!["build".into(), "build".into(), " coordinate".into()]),
            ..Flags::default()
        };
        let e = resolve(&RunnerConfig::default(), None, &flags).unwrap();
        assert_eq!(e.kinds.value, ["build", "coordinate"]);
    }

    #[test]
    fn under_the_build_floor_the_runner_declares_no_build() {
        let d = Declared {
            capacity: Some(4),
            version: "v".into(),
            kinds: vec!["coordinate".into(), "build".into()],
        };
        let floor = BUILD_FLOOR_BYTES;
        assert_eq!(
            d.clone().guarded(Some(floor - 1), floor).kinds,
            ["coordinate"]
        );
        assert_eq!(
            d.clone().guarded(Some(floor), floor).kinds,
            ["coordinate", "build"]
        );
        assert_eq!(
            d.clone().guarded(None, floor).kinds,
            ["coordinate", "build"]
        );
        assert_eq!(
            d.guarded(Some(0), floor).capacity,
            Some(4),
            "capacity is untouched"
        );
        // A build-only runner never sends an empty list (read as "not declared").
        let only = Declared {
            capacity: None,
            version: "v".into(),
            kinds: vec!["build".into()],
        };
        assert_eq!(only.guarded(Some(0), floor).kinds, [NOTHING]);
    }

    #[test]
    fn reads_the_available_column_of_df() {
        let linux = "Filesystem     1024-blocks      Used Available Capacity Mounted on\n\
                     /dev/nvme1n1   3844640564 2950015300 699253584      81% /mnt/build\n";
        assert_eq!(df_available(linux), Some(699_253_584 * 1024));
        let mac = "Filesystem 1024-blocks Used Available Capacity  Mounted on\n\
                   /dev/disk3s1s1 971350180 10652320 72290648    13%    /\n";
        assert_eq!(df_available(mac), Some(72_290_648 * 1024));
        assert_eq!(df_available("Filesystem\n"), None);
        assert_eq!(df_available(""), None);
    }

    #[test]
    fn the_kube_target_comes_from_the_profile_then_the_file_and_is_unset_by_default() {
        let none = resolve(&RunnerConfig::default(), None, &Flags::default()).unwrap();
        assert_eq!((none.kube_context, none.kubeconfig), (None, None));
        let mut file = RunnerConfig::default();
        file.kube_context = Some("prod".into());
        file.kubeconfig = Some("/etc/k/config".into());
        let mut work = RunnerConfig::default();
        work.kube_context = Some("work".into());
        let e = resolve(&file, Some(("work", &work)), &Flags::default()).unwrap();
        assert_eq!(
            e.kube_context.unwrap().source,
            Source::Profile("work".into())
        );
        assert_eq!(e.kubeconfig.unwrap().source, Source::File);
    }
}
