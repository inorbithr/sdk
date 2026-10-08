//! The pages the loopback listener answers with.
//!
//! Each page is one self-contained document: the style, the script, the InOrbit mark
//! and Io are inline, and the policy sent with it (`default-src 'none'`) allows only
//! that style and that script, by hash. Nothing is loaded from the network, and no page
//! ever shows the code, the `state` or a token. English only: the command line has no
//! locale detection.

use std::fmt::Write as _;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use sha2::{Digest as _, Sha256};

const STYLE: &str = include_str!("page/style.css");
const SCRIPT: &str = include_str!("page/script.js");
const IO_HELLO: &str = include_str!("page/io-hello.svg");
const IO_LOST: &str = include_str!("page/io-lost.svg");

/// The InOrbit mark (`ui/www/public/brand/mark.svg` in the platform), with the
/// amber dot of `mark-amber.svg`.
const MARK: &str = r#"<svg class="mark" viewBox="0 0 32 32" fill="none" aria-hidden="true"><circle cx="18.8" cy="17.3" r="8.7" stroke="currentColor" stroke-width="2.6"/><rect x="3.7" y="9.8" width="2.8" height="17.5" rx="1.4" fill="currentColor"/><circle class="dot" cx="5.1" cy="6.8" r="1.9"/></svg>"#;

/// An orbit ring with a dot going round it once, then a check (or a cross).
const BADGE_OK: &str = r#"<div class="badge ok"><svg viewBox="0 0 64 64" aria-hidden="true"><circle class="track" cx="32" cy="32" r="26"/><circle class="ring" cx="32" cy="32" r="26" pathLength="100"/><g class="orbiter"><circle cx="32" cy="6" r="4"/></g><path class="check" d="M21 33.5l7.5 7.5L44 25.5" pathLength="100"/></svg></div>"#;
const BADGE_BAD: &str = r#"<div class="badge bad"><svg viewBox="0 0 64 64" aria-hidden="true"><circle class="track" cx="32" cy="32" r="26"/><circle class="ring" cx="32" cy="32" r="26" pathLength="100"/><g class="orbiter"><circle cx="32" cy="6" r="4"/></g><path class="check" d="M24 24l16 16M40 24L24 40" pathLength="100"/></svg></div>"#;

const EXTERNAL: &str =
    r#"<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M7 17 17 7M9 7h8v8"/></svg>"#;

/// What the listener tells the browser.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Page<'a> {
    /// The tokens are in hand.
    SignedIn {
        /// The profile being signed in, when the caller named it.
        profile: Option<&'a str>,
        /// The account from the access token.
        account: Option<&'a str>,
        /// The account's plan from the access token.
        plan: Option<&'a str>,
    },
    /// The person declined at the sign-in service.
    Denied,
    /// The browser came back from another sign-in.
    StateMismatch,
    /// Anything else; the reason is a fixed sentence, never text from the request.
    Failed(&'static str),
    /// Any path but `/callback`.
    NotHere,
}

/// A page and the content security policy that lets exactly its style and script run.
pub(crate) struct Rendered {
    pub(crate) body: String,
    pub(crate) csp: String,
}

impl Page<'_> {
    pub(crate) fn render(&self) -> Rendered {
        let body = match *self {
            Self::SignedIn {
                profile,
                account,
                plan,
            } => signed_in(profile, account, plan),
            Self::Denied => problem(
                "Sign-in declined",
                "Sign-in was declined",
                "The sign-in service says access was not allowed, so the terminal was not signed in.",
                "If that was a mistake, close this tab and start again.",
            ),
            Self::StateMismatch => problem(
                "Sign-in did not match",
                "This sign-in did not match",
                "The browser came back from a different sign-in than the one your terminal started, so it was refused to keep your account safe.",
                "This happens with an old tab or a second login. Close this tab and start again.",
            ),
            Self::Failed(reason) => problem(
                "Sign-in did not finish",
                "Sign-in did not finish",
                reason,
                "The terminal says more. Close this tab and start again.",
            ),
            Self::NotHere => {
                return Rendered {
                    body: "<!doctype html><html lang=\"en\"><meta charset=\"utf-8\"><title>iohr</title><p>Nothing here.</p></html>".into(),
                    csp: "default-src 'none'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'".into(),
                };
            }
        };
        Rendered { body, csp: csp() }
    }
}

/// `default-src 'none'`, with the one style and the one script allowed by hash.
fn csp() -> String {
    format!(
        "default-src 'none'; style-src '{}'; script-src '{}'; img-src 'none'; \
         base-uri 'none'; form-action 'none'; frame-ancestors 'none'",
        hash(STYLE),
        hash(SCRIPT)
    )
}

fn hash(text: &str) -> String {
    format!(
        "sha256-{}",
        STANDARD.encode(Sha256::digest(text.as_bytes()))
    )
}

fn head(title: &str) -> String {
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
         <meta name=\"referrer\" content=\"no-referrer\">\
         <meta name=\"color-scheme\" content=\"dark light\">\
         <title>{title} · iohr</title><style>{STYLE}</style></head><body>\
         <main class=\"card\"><div class=\"top\"><div class=\"brand\">{MARK}<span>InOrbit</span></div>\
         <span class=\"tag\">iohr login</span></div>"
    )
}

fn tail() -> String {
    format!(
        "</main><p class=\"foot\">Served by <code>iohr</code> on this computer. Nothing on this page came from the network.</p>\
         <p id=\"status\" class=\"sr\" role=\"status\" aria-live=\"polite\"></p>\
         <script>{SCRIPT}</script></body></html>"
    )
}

fn copy_chip(command: &str) -> String {
    let c = escape(command);
    format!(
        "<li><button type=\"button\" class=\"chip\" data-copy=\"{c}\" aria-label=\"Copy {c}\">\
         <span class=\"pr\" aria-hidden=\"true\">$</span><code>{c}</code><span class=\"hint\" aria-hidden=\"true\">copy</span></button></li>"
    )
}

fn link_chip(label: &str, href: &str) -> String {
    format!(
        "<li><a class=\"chip\" href=\"{href}\" target=\"_blank\" rel=\"noopener noreferrer\">{label}{EXTERNAL}<span class=\"sr\"> (opens in a new tab)</span></a></li>"
    )
}

fn signed_in(profile: Option<&str>, account: Option<&str>, plan: Option<&str>) -> String {
    let mut s = head("Signed in");
    let _ = write!(
        s,
        "<div class=\"hero\"><div class=\"stage\">{IO_HELLO}</div>{BADGE_OK}</div>\
         <h1>You\u{2019}re signed in</h1>\
         <p class=\"lead\">The command line has its session and keeps it in your system\u{2019}s credential store.</p>"
    );
    let facts: Vec<(&str, &str)> = [("Profile", profile), ("Account", account), ("Plan", plan)]
        .into_iter()
        .filter_map(|(k, v)| v.filter(|v| !v.is_empty()).map(|v| (k, v)))
        .collect();
    if !facts.is_empty() {
        s.push_str("<dl class=\"facts\">");
        for (k, v) in facts {
            let _ = write!(s, "<div><dt>{k}</dt><dd>{}</dd></div>", escape(v));
        }
        s.push_str("</dl>");
    }
    s.push_str(
        "<p class=\"close\">You can close this tab. Your terminal is ready.</p>\
         <p class=\"count\" id=\"count\" hidden>Trying to close this tab in <span id=\"n\">10</span>\u{a0}s.\
         <button type=\"button\" id=\"stay\">Keep it open</button></p>\
         <h2>Next steps</h2><ul class=\"chips\">",
    );
    s.push_str(&copy_chip("iohr whoami"));
    s.push_str(&copy_chip("iohr ext install agent"));
    s.push_str(&link_chip("Docs", "https://docs.inorbit.hr"));
    s.push_str(&link_chip("Console", "https://console.inorbit.hr"));
    s.push_str("</ul>");
    s.push_str(&tail());
    s
}

fn problem(title: &str, heading: &str, reason: &str, next: &str) -> String {
    let mut s = head(title);
    let _ = write!(
        s,
        "<div class=\"hero\"><div class=\"stage\">{IO_LOST}</div>{BADGE_BAD}</div>\
         <h1>{}</h1><p class=\"lead\">Your terminal is not signed in.</p>\
         <div class=\"why\"><p>{}</p><p>{}</p></div>\
         <h2>What to do</h2><ul class=\"chips\">",
        escape(heading),
        escape(reason),
        escape(next)
    );
    s.push_str(&copy_chip("iohr login"));
    s.push_str(&link_chip("Sign-in help", "https://docs.inorbit.hr"));
    s.push_str("</ul>");
    s.push_str(&tail());
    s
}

/// Escapes text for HTML content and double-quoted attributes.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{Page, escape};

    fn all() -> Vec<Page<'static>> {
        vec![
            Page::SignedIn {
                profile: Some("default"),
                account: Some("acc_1"),
                plan: Some("pro"),
            },
            Page::SignedIn {
                profile: None,
                account: None,
                plan: None,
            },
            Page::Denied,
            Page::StateMismatch,
            Page::Failed("The sign-in service refused the request."),
            Page::NotHere,
        ]
    }

    #[test]
    fn the_pages_load_nothing() {
        for page in all() {
            let r = page.render();
            assert!(!r.body.contains("src="), "{page:?}");
            assert!(!r.body.contains("url(http"), "{page:?}");
            assert!(!r.body.contains("@import"), "{page:?}");
            assert!(r.csp.starts_with("default-src 'none'"), "{page:?}");
            assert!(!r.csp.contains("unsafe"), "{page:?}");
            // Links are navigations only, to InOrbit's own sites.
            for href in r.body.split("href=\"").skip(1) {
                assert!(
                    href.starts_with("https://docs.inorbit.hr")
                        || href.starts_with("https://console.inorbit.hr"),
                    "{href}"
                );
            }
        }
    }

    #[test]
    fn the_policy_names_exactly_the_inline_style_and_script() {
        let r = Page::Denied.render();
        assert_eq!(r.body.matches("<style>").count(), 1);
        assert_eq!(r.body.matches("<script>").count(), 1);
        assert!(r.csp.contains(&super::hash(super::STYLE)));
        assert!(r.csp.contains(&super::hash(super::SCRIPT)));
        // No inline handlers or style attributes, which a hash would not allow.
        assert!(!r.body.contains(" style=") && !r.body.contains(" onclick="));
    }

    #[test]
    fn the_success_page_shows_what_it_knows_escaped() {
        let r = Page::SignedIn {
            profile: Some("work"),
            account: Some("<b>acc</b>"),
            plan: None,
        }
        .render();
        assert!(r.body.contains("You\u{2019}re signed in"));
        assert!(r.body.contains("<dd>work</dd>"));
        assert!(r.body.contains("&lt;b&gt;acc&lt;/b&gt;"));
        assert!(!r.body.contains("<dt>Plan</dt>"));
        assert!(r.body.contains("iohr whoami") && r.body.contains("iohr ext install agent"));
    }

    #[test]
    fn escapes() {
        assert_eq!(
            escape(r#"<a href="x">&'"#),
            "&lt;a href=&quot;x&quot;&gt;&amp;&#39;"
        );
    }
}
