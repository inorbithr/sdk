//! Personal use only (RFC 0214): a runner on the person's own Claude plan, rather than
//! an Anthropic API key, runs only after the person has read [`NOTICE`] and accepted
//! [`ACKNOWLEDGEMENT`] on this machine. The record of that (`runner/personal-use.json`:
//! the notice's version and when it was accepted, nothing else) is local and never sent;
//! a changed notice or a year since asks again.

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// The notice's version: raised whenever its text changes, so it is shown again.
pub(crate) const NOTICE_VERSION: u32 = 1;

/// How long an acceptance holds before the notice is shown again.
pub(crate) const VALID_FOR: Duration = Duration::from_hours(24 * 365);

/// What the person reads before a runner runs on their own Claude plan.
pub(crate) const NOTICE: &str = "Personal use only. Running the runner on your own Claude plan is for your own, \
non-commercial projects, on work only you assign. Your use stays under Anthropic's Consumer Terms and \
Usage Policy, and Anthropic may limit or suspend your account. For work for an organisation, a team or \
anyone else, store an Anthropic API key with `iohr runner api-key set`. InOrbit does not handle your \
Claude sign-in or pay for your usage. InOrbit is not affiliated with Anthropic; Claude is a trademark \
of Anthropic.";

/// What the person accepts.
pub(crate) const ACKNOWLEDGEMENT: &str =
    "I will use this mode only for my own personal, non-commercial work.";

/// The record of an acceptance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Accepted {
    pub(crate) notice_version: u32,
    /// Seconds since the Unix epoch.
    pub(crate) accepted_at: u64,
}

impl Accepted {
    /// An acceptance of the current notice, now.
    pub(crate) fn now() -> Self {
        Self {
            notice_version: NOTICE_VERSION,
            accepted_at: secs(SystemTime::now()),
        }
    }

    /// Whether it still holds at `now`: the notice is the current one and it is less
    /// than [`VALID_FOR`] old (an acceptance dated in the future does not hold).
    pub(crate) fn holds(&self, now: SystemTime) -> bool {
        let now = secs(now);
        self.notice_version == NOTICE_VERSION
            && self.accepted_at <= now
            && now - self.accepted_at < VALID_FOR.as_secs()
    }
}

fn secs(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Where the record lives under the config directory.
pub(crate) fn path(config_dir: &Path) -> PathBuf {
    config_dir.join("runner").join("personal-use.json")
}

/// The record, if there is one this user owns and it parses; anything else is none, so
/// the notice is shown again.
pub(crate) fn load(config_dir: &Path) -> Option<Accepted> {
    let mut f = super::private::open_trusted(&path(config_dir), false).ok()??;
    let mut s = String::new();
    f.read_to_string(&mut s).ok()?;
    serde_json::from_str(&s).ok()
}

/// Records `a` (directory 0700, file 0600, through a rename).
///
/// # Errors
/// The file could not be written; the text names the path.
pub(crate) fn save(config_dir: &Path, a: &Accepted) -> Result<(), String> {
    let p = path(config_dir);
    let fail = |e: &std::io::Error| format!("cannot write {}: {}", p.display(), e.kind());
    let dir = p.parent().unwrap_or(config_dir);
    iohr_auth::create_private_dir(dir).map_err(|e| fail(&e))?;
    let tmp = dir.join(".personal-use.json.tmp");
    let body = serde_json::to_vec(a).map_err(|e| e.to_string())?;
    {
        use std::io::Write as _;
        let mut o = std::fs::OpenOptions::new();
        o.write(true).create(true).truncate(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
        let mut f = o.open(&tmp).map_err(|e| fail(&e))?;
        f.write_all(&body).map_err(|e| fail(&e))?;
        f.sync_all().map_err(|e| fail(&e))?;
    }
    std::fs::rename(&tmp, &p).map_err(|e| fail(&e))
}

/// Removes the record; none is fine.
///
/// # Errors
/// It is there and could not be removed.
pub(crate) fn remove(config_dir: &Path) -> Result<(), String> {
    match std::fs::remove_file(path(config_dir)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!(
            "cannot remove {}: {}",
            path(config_dir).display(),
            e.kind()
        )),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]
    use super::*;

    #[test]
    fn an_acceptance_holds_for_a_year_of_this_notice_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(dir.path()), None);
        let a = Accepted::now();
        save(dir.path(), &a).unwrap();
        assert_eq!(load(dir.path()), Some(a));
        let at = UNIX_EPOCH + Duration::from_secs(a.accepted_at);
        assert!(a.holds(at));
        assert!(a.holds(at + VALID_FOR - Duration::from_secs(1)));
        assert!(!a.holds(at + VALID_FOR));
        assert!(!a.holds(at - Duration::from_secs(1)), "dated in the future");
        let old = Accepted {
            notice_version: NOTICE_VERSION - 1,
            ..a
        };
        assert!(!old.holds(at), "an older notice asks again");
        std::fs::write(path(dir.path()), "not json").unwrap();
        assert_eq!(load(dir.path()), None);
        remove(dir.path()).unwrap();
        remove(dir.path()).unwrap();
        assert_eq!(load(dir.path()), None);
    }

    #[test]
    fn the_notice_says_personal_use_and_points_organisations_to_an_api_key() {
        assert!(NOTICE.starts_with("Personal use only."));
        assert!(NOTICE.contains("iohr runner api-key set"));
        for promo in ["Max limits", "cheaper", "save money", "Pro or Max plan"] {
            assert!(!NOTICE.contains(promo), "{promo}");
        }
    }
}
