//! What one Mac knows about its monitors beyond the files: which are switched off or on here
//! and until when, and how each one ran (RFC 0008, ADR 0009).
//!
//! Two files in a git-ignored state folder, by default `<project>/.inorbit/state/monitors/` beside the
//! project's `.inorbit/monitors/`:
//!
//! - `silences.json`: per monitor name, an override of the file's `enabled`: off (a silence,
//!   with a reason and an optional end) or on (for a monitor whose file says `enabled = false`).
//!   An override past its end counts as gone and is dropped on the next write. The monitor
//!   files are never edited for this, so a silence never shows in git or reaches the agent.
//! - `runs.jsonl`: one line per monitor per run: when, the verdict, the short why, the time and
//!   the status. Never a body, a header value or a secret: the why is the same short line the
//!   report prints (a shape failure names a place and two types, never a value), cut to
//!   [`MAX_WHY`]. Past [`MAX_LOG`] bytes the oldest half is dropped.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::Monitor;
use super::run::{Ran, Verdict};

/// The run log's size before its oldest half is dropped.
pub const MAX_LOG: u64 = 1024 * 1024;
/// The longest why kept in the run log, in characters.
pub const MAX_WHY: usize = 300;
const SILENCES: &str = "silences.json";
const RUNS: &str = "runs.jsonl";

/// Seconds since 1970, UTC.
#[must_use]
pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// The state folder for a monitors folder: `IOHR_MONITORS_STATE` when set, else
/// `.inorbit/state/monitors` under the project that holds it (`<root>/.inorbit/monitors`; the
/// project's `.gitignore` lists `.inorbit/state`), or `state/monitors` beside a folder elsewhere.
#[must_use]
pub fn dir_for(monitors: &Path) -> PathBuf {
    if let Some(d) = std::env::var_os("IOHR_MONITORS_STATE") {
        return PathBuf::from(d);
    }
    let parent = monitors.parent().unwrap_or(monitors);
    parent.join("state").join("monitors")
}

/// A local override of a monitor's `enabled`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Override {
    /// false: silenced here; true: on here though its file says `enabled = false`.
    pub on: bool,
    /// When it ends, in seconds since 1970; never when missing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// When it was set.
    pub since: u64,
}

impl Override {
    fn live(&self, now: u64) -> bool {
        self.until.is_none_or(|u| u > now)
    }
}

/// Whether a monitor runs here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    On,
    /// Its file says `enabled = false` and nothing here turned it on.
    Off,
    /// Switched off here, with the reason and the end.
    Silenced { reason: Option<String>, until: Option<u64> },
}

impl Mode {
    /// `on`, `off` or `silenced`, for JSON.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Mode::On => "on",
            Mode::Off => "off",
            Mode::Silenced { .. } => "silenced",
        }
    }
}

/// One line of the run log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// Seconds since 1970.
    pub at: u64,
    pub name: String,
    /// `pass`, `fail` or `skip`.
    pub verdict: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
    pub ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
}

impl Entry {
    /// The log line for one run at `at`, its why cut to [`MAX_WHY`].
    #[must_use]
    pub fn of(r: &Ran, at: u64) -> Entry {
        let (verdict, why) = match &r.verdict {
            Verdict::Pass => ("pass", None),
            Verdict::Fail(w) => ("fail", Some(w)),
            Verdict::Skip(w) => ("skip", Some(w)),
        };
        Entry { at, name: r.name.clone(), verdict: verdict.into(), why: why.map(|w| w.chars().take(MAX_WHY).collect()), ms: r.ms, status: r.status }
    }
}

/// One monitor's past, newest last.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Past {
    pub runs: Vec<Entry>,
    pub last_pass: Option<u64>,
    pub last_fail: Option<u64>,
    /// How often the verdict went from pass to fail or back.
    pub flaps: usize,
}

impl Past {
    /// The newest run.
    #[must_use]
    pub fn last(&self) -> Option<&Entry> {
        self.runs.last()
    }
}

/// The state folder and what is in it.
#[derive(Debug, Clone)]
pub struct State {
    pub dir: PathBuf,
}

impl State {
    #[must_use]
    pub fn new(dir: PathBuf) -> State {
        State { dir }
    }

    /// The overrides still in force at `now`.
    #[must_use]
    pub fn overrides(&self, now: u64) -> BTreeMap<String, Override> {
        let all: BTreeMap<String, Override> = fs::read(self.dir.join(SILENCES)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        all.into_iter().filter(|(_, o)| o.live(now)).collect()
    }

    /// Whether `m` runs here at `now`, given `overrides`.
    #[must_use]
    pub fn mode(m: &Monitor, overrides: &BTreeMap<String, Override>, now: u64) -> Mode {
        match overrides.get(&m.name).filter(|o| o.live(now)) {
            Some(o) if !o.on => Mode::Silenced { reason: o.reason.clone(), until: o.until },
            Some(_) => Mode::On,
            None if m.enabled => Mode::On,
            None => Mode::Off,
        }
    }

    /// Switches `m` off here until `until` (never when `None`).
    pub fn disable(&self, m: &Monitor, until: Option<u64>, reason: Option<String>) -> std::io::Result<()> {
        let now = now();
        self.write_overrides(&m.name, Some(Override { on: false, until, reason, since: now }), now)
    }

    /// Switches `m` on here: drops its silence, and for a monitor whose file says
    /// `enabled = false`, keeps an override that turns it on.
    pub fn enable(&self, m: &Monitor) -> std::io::Result<()> {
        let now = now();
        let o = (!m.enabled).then_some(Override { on: true, until: None, reason: None, since: now });
        self.write_overrides(&m.name, o, now)
    }

    /// Drops any override for `name`, for a monitor that was deleted.
    pub fn forget(&self, name: &str) -> std::io::Result<()> {
        if !self.overrides(now()).contains_key(name) {
            return Ok(());
        }
        self.write_overrides(name, None, now())
    }

    fn write_overrides(&self, name: &str, o: Option<Override>, now: u64) -> std::io::Result<()> {
        let mut all = self.overrides(now);
        match o {
            Some(o) => all.insert(name.to_owned(), o),
            None => all.remove(name),
        };
        let text = serde_json::to_vec_pretty(&all).map_err(std::io::Error::other)?;
        replace(&self.dir.join(SILENCES), &text)
    }

    /// Appends one line per run at `at`; past [`MAX_LOG`] the oldest half is dropped.
    pub fn record(&self, ran: &[Ran], at: u64) -> std::io::Result<()> {
        if ran.is_empty() {
            return Ok(());
        }
        fs::create_dir_all(&self.dir)?;
        let path = self.dir.join(RUNS);
        let mut lines = String::new();
        for r in ran {
            lines.push_str(&serde_json::to_string(&Entry::of(r, at)).map_err(std::io::Error::other)?);
            lines.push('\n');
        }
        fs::OpenOptions::new().create(true).append(true).open(&path)?.write_all(lines.as_bytes())?;
        if fs::metadata(&path)?.len() > MAX_LOG {
            let text = fs::read_to_string(&path)?;
            let all: Vec<&str> = text.lines().collect();
            let keep = all[all.len() / 2..].join("\n") + "\n";
            replace(&path, keep.as_bytes())?;
        }
        Ok(())
    }

    /// Every logged run, oldest first; lines that do not parse are left out.
    #[must_use]
    pub fn runs(&self) -> Vec<Entry> {
        fs::read_to_string(self.dir.join(RUNS)).map(|t| t.lines().filter_map(|l| serde_json::from_str(l).ok()).collect()).unwrap_or_default()
    }

    /// Each monitor's past, by name.
    #[must_use]
    pub fn pasts(&self) -> BTreeMap<String, Past> {
        let mut out: BTreeMap<String, Past> = BTreeMap::new();
        for e in self.runs() {
            let p = out.entry(e.name.clone()).or_default();
            match e.verdict.as_str() {
                "pass" => p.last_pass = Some(e.at),
                "fail" => p.last_fail = Some(e.at),
                _ => {}
            }
            let judged = |v: &str| v == "pass" || v == "fail";
            if judged(&e.verdict) && p.runs.iter().rev().find(|r| judged(&r.verdict)).is_some_and(|r| r.verdict != e.verdict) {
                p.flaps += 1;
            }
            p.runs.push(e);
        }
        out
    }
}

/// Writes `bytes` to `path` through a file beside it, so a reader never sees half a file.
fn replace(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(".{}.{}", path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default(), std::process::id()));
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use super::check_files;

    fn monitors(text: &str) -> Vec<Monitor> {
        let f = super::File { path: PathBuf::from("eye.toml"), value: super::parse(text).unwrap() };
        let r = check_files(&[f]);
        assert!(r.problems.is_empty(), "{:?}", r.problems);
        r.monitors
    }

    fn ran(name: &str, v: Verdict) -> Ran {
        Ran { name: name.into(), verdict: v, ms: 3, status: Some(200) }
    }

    #[test]
    fn silences_end_and_enable_turns_on_a_monitor_off_by_default() {
        let t = tempfile::Builder::new().prefix("eye-monitors-state-").tempdir().unwrap();
        let st = State::new(t.path().join("data/monitors"));
        let ms = monitors(
            "[[check]]\nname = \"api\"\ntarget = \"http://127.0.0.1:7171/api/health\"\nevery = \"60s\"\n\n\
             [[check]]\nname = \"follower\"\nenabled = false\ntarget = \"http://127.0.0.1:7173/api/health\"\nevery = \"5m\"\n",
        );
        let (api, follower) = (&ms[0], &ms[1]);
        let now = now();
        assert_eq!(State::mode(api, &st.overrides(now), now), Mode::On);
        assert_eq!(State::mode(follower, &st.overrides(now), now), Mode::Off);

        st.disable(api, Some(now + 3600), Some("deploy".into())).unwrap();
        let o = st.overrides(now);
        assert_eq!(State::mode(api, &o, now), Mode::Silenced { reason: Some("deploy".into()), until: Some(now + 3600) });
        assert_eq!(State::mode(api, &o, now + 3601), Mode::On, "a silence past its end is gone");
        assert!(st.overrides(now + 3601).is_empty());

        st.enable(follower).unwrap();
        st.enable(api).unwrap();
        let o = st.overrides(now);
        assert_eq!(State::mode(api, &o, now), Mode::On);
        assert_eq!(State::mode(follower, &o, now), Mode::On);
        assert_eq!(o.len(), 1, "enable drops a silence and keeps only the on override: {o:?}");
        let left: Vec<_> = fs::read_dir(&st.dir).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(left, ["silences.json"], "no temp file is left");
    }

    #[test]
    fn the_log_keeps_verdicts_without_bodies_counts_flaps_and_drops_its_oldest_half() {
        let t = tempfile::Builder::new().prefix("eye-monitors-log-").tempdir().unwrap();
        let st = State::new(t.path().to_path_buf());
        st.record(&[ran("api", Verdict::Pass), ran("ui", Verdict::Skip("https runs in the agent".into()))], 100).unwrap();
        st.record(&[ran("api", Verdict::Fail("x".repeat(1000)))], 200).unwrap();
        st.record(&[ran("api", Verdict::Pass)], 300).unwrap();
        let p = &st.pasts()["api"];
        assert_eq!((p.runs.len(), p.last_pass, p.last_fail, p.flaps), (3, Some(300), Some(200), 2));
        assert_eq!(p.runs[1].why.as_ref().unwrap().chars().count(), MAX_WHY);
        assert_eq!(st.pasts()["ui"].flaps, 0);

        let big = ran("api", Verdict::Fail("y".repeat(MAX_WHY)));
        let n = usize::try_from(MAX_LOG).unwrap() / 300 + 10;
        for i in 0..n {
            st.record(std::slice::from_ref(&big), 1000 + i as u64).unwrap();
        }
        let len = fs::metadata(st.dir.join(RUNS)).unwrap().len();
        assert!(len <= MAX_LOG, "{len}");
        let runs = st.runs();
        assert_eq!(runs.last().unwrap().at, 1000 + n as u64 - 1, "the newest run is kept");
        assert!(runs.first().unwrap().at > 300, "the oldest runs are gone");
    }
}
