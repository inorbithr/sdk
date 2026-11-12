//! `robots.txt` as the site reader needs it: may this path be fetched, and how long to wait
//! between requests (RFC 9309, with the `Crawl-delay` line most sites use).
//!
//! The reader asks only for pages the crawler could not hand over, but it asks as a guest all
//! the same: a path the site disallows is not requested, and a `Crawl-delay` slower than the
//! reader's own rate sets the pace for that host.

use std::time::Duration;

/// The rules that apply to one agent on one host.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Rules {
    /// `(allow, pattern)`, as written.
    lines: Vec<(bool, String)>,
    /// `Crawl-delay`, when the group sets one.
    pub delay: Option<Duration>,
    /// The file could not be read: nothing is allowed.
    shut: bool,
}

impl Rules {
    /// No `robots.txt` (a 4xx answer): everything is allowed.
    #[must_use]
    pub fn open() -> Self {
        Self::default()
    }

    /// A `robots.txt` that could not be read (no answer, a 5xx): nothing is allowed, as the
    /// RFC says of an unreachable file.
    #[must_use]
    pub fn shut() -> Self {
        Self {
            shut: true,
            ..Self::default()
        }
    }

    /// The groups of `text` that name `agent` (a product token such as `iohr`), else the `*`
    /// groups.
    #[must_use]
    pub fn parse(text: &str, agent: &str) -> Self {
        struct Group {
            agents: Vec<String>,
            lines: Vec<(bool, String)>,
            delay: Option<Duration>,
        }
        let mut groups: Vec<Group> = Vec::new();
        let mut in_rules = true;
        for raw in text.lines() {
            let line = raw.split('#').next().unwrap_or_default().trim();
            let Some((key, value)) = line.split_once(':') else {
                continue;
            };
            let (key, value) = (key.trim().to_ascii_lowercase(), value.trim());
            match key.as_str() {
                "user-agent" => {
                    if in_rules || groups.is_empty() {
                        groups.push(Group {
                            agents: Vec::new(),
                            lines: Vec::new(),
                            delay: None,
                        });
                    }
                    in_rules = false;
                    if let Some(g) = groups.last_mut() {
                        g.agents.push(value.to_ascii_lowercase());
                    }
                }
                "allow" | "disallow" => {
                    in_rules = true;
                    if let Some(g) = groups.last_mut()
                        && !value.is_empty()
                    {
                        g.lines.push((key == "allow", value.to_owned()));
                    }
                }
                "crawl-delay" => {
                    in_rules = true;
                    if let Some(g) = groups.last_mut()
                        && let Ok(s) = value.parse::<f64>()
                        && s.is_finite()
                        && s > 0.0
                    {
                        g.delay = Some(Duration::from_secs_f64(s.min(3600.0)));
                    }
                }
                _ => {}
            }
        }
        let agent = agent.to_ascii_lowercase();
        let named = |g: &Group| {
            g.agents
                .iter()
                .any(|a| a != "*" && !a.is_empty() && agent.contains(a.as_str()))
        };
        let star = |g: &Group| g.agents.iter().any(|a| a == "*");
        let chosen: Vec<&Group> = if groups.iter().any(named) {
            groups.iter().filter(|g| named(g)).collect()
        } else {
            groups.iter().filter(|g| star(g)).collect()
        };
        Self {
            lines: chosen.iter().flat_map(|g| g.lines.clone()).collect(),
            delay: chosen.iter().filter_map(|g| g.delay).max(),
            shut: false,
        }
    }

    /// Whether `target` (a path with its query, as requested) may be fetched: the longest
    /// matching rule wins, and `Allow` wins a tie.
    #[must_use]
    pub fn allows(&self, target: &str) -> bool {
        if self.shut {
            return false;
        }
        let mut best: Option<(usize, bool)> = None;
        for (allow, pat) in &self.lines {
            if matches(pat, target) {
                let better = match best {
                    None => true,
                    Some((n, a)) => pat.len() > n || (pat.len() == n && *allow && !a),
                };
                if better {
                    best = Some((pat.len(), *allow));
                }
            }
        }
        best.is_none_or(|(_, allow)| allow)
    }
}

/// `pattern` against the start of `target`: `*` is any run, a trailing `$` anchors the end.
fn matches(pattern: &str, target: &str) -> bool {
    let (pat, anchored) = match pattern.strip_suffix('$') {
        Some(p) => (p, true),
        None => (pattern, false),
    };
    let parts: Vec<&str> = pat.split('*').collect();
    let mut at = 0usize;
    for (i, part) in parts.iter().enumerate() {
        if i == 0 {
            if !target.starts_with(part) {
                return false;
            }
            at = part.len();
        } else if i == parts.len() - 1 && anchored {
            return target.len() >= at + part.len() && target.ends_with(part);
        } else {
            match target[at..].find(part) {
                Some(p) => at += p + part.len(),
                None => return false,
            }
        }
    }
    !anchored || at == target.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "\
# a comment
User-agent: *
Disallow: /private
Allow: /private/open
Disallow: /*.pdf$
Crawl-delay: 2

User-agent: iohr
Disallow: /iohr-only
Crawl-delay: 5
";

    #[test]
    fn a_group_that_names_the_agent_wins_over_the_star() {
        let r = Rules::parse(FILE, "iohr/0.1");
        assert!(!r.allows("/iohr-only/x"));
        assert!(r.allows("/private/x"), "the star group does not apply");
        assert_eq!(r.delay, Some(Duration::from_secs(5)));
    }

    #[test]
    fn the_star_group_applies_to_the_rest_with_the_longest_rule_first() {
        let r = Rules::parse(FILE, "other");
        assert!(!r.allows("/private/x"));
        assert!(r.allows("/private/open/x"), "the longer Allow wins");
        assert!(r.allows("/pricing"));
        assert!(!r.allows("/files/a.pdf"));
        assert!(r.allows("/files/a.pdf?x=1"), "$ anchors the end");
        assert_eq!(r.delay, Some(Duration::from_secs(2)));
    }

    #[test]
    fn a_missing_file_allows_and_an_unreachable_one_does_not() {
        assert!(Rules::open().allows("/anything"));
        assert!(!Rules::shut().allows("/anything"));
        assert!(Rules::parse("", "iohr").allows("/x"));
    }

    #[test]
    fn user_agent_lines_in_a_row_share_their_rules() {
        let r = Rules::parse("User-agent: a\nUser-agent: iohr\nDisallow: /x\n", "iohr");
        assert!(!r.allows("/x"));
        assert!(r.allows("/y"));
        let none = Rules::parse("User-agent: *\nDisallow:\n", "iohr");
        assert!(none.allows("/x"), "an empty Disallow allows everything");
    }

    #[test]
    fn wildcards_match_runs() {
        assert!(matches("/a*/c", "/a/b/c"));
        assert!(matches("/*?session=", "/x?session=1"));
        assert!(!matches("/a*/c$", "/a/b/c/d"));
        assert!(matches("/a*/c$", "/a/b/c"));
    }
}
