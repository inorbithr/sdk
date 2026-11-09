//! What the crawler says while it runs (RFC 0163): `iohr-agent atlas crawl --progress` writes
//! one JSON line per event on standard error, and this reads them into the numbers init
//! draws a bar from.
//!
//! The contract is the agent's: `{"event":"progress","v":1,"scope":...,"phase":...,"read":
//! ...,"failed":...,"queued":...,"total":...,"cap":...}`, fields only ever added, `total`
//! absent until a sitemap has said how much there is. A line that is not a progress line (the
//! agent's own text summary, a warning) is not one, and a newer agent's extra fields are
//! ignored. A line holds counts and the registrable domain, never a page address, so nothing
//! here can show one.

use serde::Deserialize;

/// The agent's `event` value for a progress line.
const EVENT: &str = "progress";

/// One progress line. Every field has a default, so an agent that leaves one out, or adds
/// one, still reads.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub(super) struct Line {
    event: String,
    /// `pages` while pages are read, `listings` while the page list is written.
    pub phase: String,
    pub read: u64,
    pub failed: u64,
    pub queued: u64,
    /// What the crawl expects to request, at most `cap`; absent until it is known.
    pub total: Option<u64>,
    /// The page budget the crawl runs under.
    pub cap: u64,
}

/// Reads one line of the agent's standard error: `Some` for a progress line, `None` for
/// anything else.
pub(super) fn parse(line: &str) -> Option<Line> {
    let line = line.trim();
    if !line.starts_with('{') {
        return None;
    }
    let l: Line = serde_json::from_str(line).ok()?;
    (l.event == EVENT).then_some(l)
}

impl Line {
    /// Pages the crawl has finished with, read or failed: the bar's position.
    pub(super) const fn done(&self) -> u64 {
        self.read + self.failed
    }

    /// The length of the bar: `total` once known, never below what is done.
    pub(super) fn length(&self) -> Option<u64> {
        self.total.map(|t| t.max(self.done()).max(1))
    }

    /// What the bar or the spinner says. With a total: pages done of it, the failures, and how
    /// many more the page limit allows. Without: a live count and the queue.
    pub(super) fn message(&self, label: &str) -> String {
        if self.phase == "listings" {
            return format!("Reading {label}: writing the page list");
        }
        let failed = if self.failed == 0 {
            String::new()
        } else {
            format!(", {} failed", self.failed)
        };
        match self.length() {
            Some(total) => {
                let left = self.cap.saturating_sub(self.done());
                format!(
                    "Reading {label}: {} of {total} pages{failed}, {left} left to the {}-page limit",
                    self.done(),
                    self.cap
                )
            }
            None => format!(
                "Reading {label}: {} pages read{failed}, {} waiting",
                self.done(),
                self.queued
            ),
        }
    }
}

/// Whether an agent that exited with `code` and said `stderr` did not know `--rate` or
/// `--progress` (an agent from before RFC 0163): clap's usage error, exit 2, naming the flag.
/// The crawl is then run again without them, at that agent's own pace.
pub(super) fn is_old_agent(code: u8, stderr: &str) -> bool {
    code == 2
        && stderr.contains("unexpected argument")
        && (stderr.contains("--rate") || stderr.contains("--progress"))
}

/// Whether an agent that exited with `code` and said `stderr` knows `--rate` but not
/// `--keep-pages`: the same usage error, naming that flag. The crawl is then run again without
/// it, and iohr requests the page text itself, once.
pub(super) fn is_no_keep(code: u8, stderr: &str) -> bool {
    code == 2 && stderr.contains("unexpected argument") && stderr.contains("--keep-pages")
}

#[cfg(test)]
mod tests {
    use super::*;

    const WITH_TOTAL: &str = r#"{"event":"progress","v":1,"scope":"acme.example","phase":"pages","read":12,"failed":1,"queued":40,"total":120,"cap":2000}"#;
    const NO_TOTAL: &str = r#"{"event":"progress","v":1,"scope":"acme.example","phase":"pages","read":3,"failed":0,"queued":9,"cap":2000}"#;

    #[test]
    fn a_progress_line_reads_with_a_total_and_without() {
        let l = parse(WITH_TOTAL).unwrap();
        assert_eq!(
            (l.done(), l.queued, l.total, l.cap),
            (13, 40, Some(120), 2000)
        );
        assert_eq!(l.length(), Some(120));
        let l = parse(NO_TOTAL).unwrap();
        assert_eq!((l.done(), l.total, l.length()), (3, None, None));
    }

    #[test]
    fn what_is_not_a_progress_line_is_not_read() {
        for s in [
            "",
            "read 12 pages in 9 s",
            r#"{"event":"other","v":1}"#,
            r#"{"v":1,"read":3}"#,
            "{not json",
        ] {
            assert_eq!(parse(s), None, "{s}");
        }
    }

    #[test]
    fn a_newer_agents_extra_fields_and_version_are_ignored() {
        let s = r#"{"event":"progress","v":2,"scope":"a","phase":"pages","read":1,"failed":0,"queued":2,"total":5,"cap":9,"eta_s":12,"hosts":["a"]}"#;
        let l = parse(s).unwrap();
        assert_eq!((l.read, l.total), (1, Some(5)));
    }

    #[test]
    fn the_bar_never_ends_before_what_is_done() {
        let mut l = parse(WITH_TOTAL).unwrap();
        l.total = Some(5);
        assert_eq!(l.length(), Some(13));
    }

    #[test]
    fn the_message_counts_up_to_the_total_and_down_to_the_limit() {
        let m = parse(WITH_TOTAL).unwrap().message("acme.example");
        assert_eq!(
            m,
            "Reading acme.example: 13 of 120 pages, 1 failed, 1987 left to the 2000-page limit"
        );
        let m = parse(NO_TOTAL).unwrap().message("acme.example");
        assert_eq!(m, "Reading acme.example: 3 pages read, 9 waiting");
    }

    #[test]
    fn the_listings_phase_says_so() {
        let mut l = parse(WITH_TOTAL).unwrap();
        l.phase = "listings".into();
        assert_eq!(l.message("a"), "Reading a: writing the page list");
    }

    #[test]
    fn an_agent_that_refuses_the_flags_is_an_old_agent() {
        let clap = "error: unexpected argument '--rate' found\n\nUsage: iohr-agent atlas crawl";
        assert!(is_old_agent(2, clap));
        assert!(is_old_agent(
            2,
            "error: unexpected argument '--progress' found"
        ));
        // Another usage error, or another failure, is not.
        assert!(!is_old_agent(2, "error: unexpected argument '--zzz' found"));
        assert!(!is_old_agent(1, clap));
        assert!(!is_old_agent(2, ""));
    }

    #[test]
    fn an_agent_that_refuses_only_keep_pages_is_not_an_old_agent() {
        let clap = "error: unexpected argument '--keep-pages' found";
        assert!(is_no_keep(2, clap));
        assert!(!is_old_agent(2, clap));
        assert!(!is_no_keep(1, clap));
        assert!(!is_no_keep(2, "error: unexpected argument '--rate' found"));
    }
}
