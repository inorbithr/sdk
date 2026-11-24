//! A monitor and its failures in plain words: what it checks, what it expects, and for a
//! failure a one-line hint on what to look at (RFC 0008). The words come from the monitor and
//! the short why only, never from a body.

use super::{Expect, Monitor, Shape, Surface, Target};

/// What `m` sends or opens, such as `GET http://127.0.0.1:7171/api/health`,
/// `tcp localhost:9494` or `eye scrum board --json`.
#[must_use]
pub fn target(m: &Monitor) -> String {
    match (&m.target, m.surface) {
        (Target::Url(u), Surface::Http | Surface::Sse) => {
            format!("{} {u}", m.method.as_deref().unwrap_or("GET"))
        }
        (Target::Cli(bin), _) => match m.steps.as_slice() {
            [one] => format!("{bin} {}", one.argv.join(" ")),
            steps => format!(
                "{bin}, {} steps: {}",
                steps.len(),
                steps
                    .iter()
                    .map(|s| s.argv.first().map_or("", String::as_str))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        },
        (t, s) => format!("{} {t}", s.as_str()),
    }
}

/// What a pass needs, such as `200, content-type application/json, under 2000 ms, body
/// matches schemas/api/health/v0.json`.
#[must_use]
pub fn expected(m: &Monitor) -> String {
    if let Some(by) = &m.by {
        return format!("refused by the {by}, never sent");
    }
    let mut parts = Vec::new();
    match m.surface {
        Surface::Tcp | Surface::Tls => parts.push("accepts a connection".to_owned()),
        Surface::Cli => {
            let exits: Vec<String> = m
                .steps
                .iter()
                .map(|s| s.expect.exit.unwrap_or(0).to_string())
                .collect();
            let same = exits.windows(2).all(|w| w[0] == w[1]);
            parts.push(if same {
                format!("exits {}", exits.first().map_or("0", String::as_str))
            } else {
                format!("exits {}", exits.join(", then "))
            });
            let mut shapes: Vec<String> = Vec::new();
            for s in m.steps.iter().filter_map(|s| one_shape(&s.expect)) {
                if !shapes.contains(&s) {
                    shapes.push(s);
                }
            }
            if !shapes.is_empty() {
                parts.push(format!("stdout matches {}", shapes.join(", ")));
            }
        }
        _ => {}
    }
    let x = &m.expect;
    if let Some(s) = x.status {
        parts.push(if m.refuse {
            format!("refused with {s}")
        } else {
            s.to_string()
        });
    } else if matches!(m.surface, Surface::Http | Surface::Sse) {
        parts.push(if m.refuse {
            "refused (4xx)".into()
        } else {
            "2xx or 3xx".into()
        });
    }
    for (k, v) in &x.headers {
        parts.push(format!("{k} {v}"));
    }
    if let Some(l) = &x.location {
        parts.push(format!("location {l}"));
    }
    if let Some(l) = &x.location_prefix {
        parts.push(format!("location starts {l}"));
    }
    if m.surface == Surface::Sse {
        parts.push(format!(
            "{} data event{}",
            m.events,
            if m.events == 1 { "" } else { "s" }
        ));
    }
    if let Some(ms) = x.max_ms {
        parts.push(format!("under {ms} ms"));
    }
    if let Some(d) = x.valid_for_days {
        parts.push(format!("certificate valid {d}+ days"));
    }
    match &x.shape {
        // A cli check's shape is its step's, said above as stdout.
        _ if m.surface == Surface::Cli => {}
        Some(Shape::One(p)) => parts.push(format!("body matches {p}")),
        Some(Shape::Events(e)) => parts.push(format!(
            "events match {}",
            e.iter()
                .map(|(n, p)| format!("{n}: {p}"))
                .collect::<Vec<_>>()
                .join(", ")
        )),
        None => {}
    }
    parts.join(", ")
}

fn one_shape(x: &Expect) -> Option<String> {
    match &x.shape {
        Some(Shape::One(p)) => Some(p.clone()),
        _ => None,
    }
}

/// The whole check in one sentence: `GET <url> answers 200, ...`.
#[must_use]
pub fn checks(m: &Monitor) -> String {
    match m.surface {
        Surface::Cli => format!("runs {} and {}", target(m), expected(m)),
        Surface::Http | Surface::Sse if m.refuse => format!("{} is {}", target(m), expected(m)),
        Surface::Http | Surface::Sse => format!("{} answers {}", target(m), expected(m)),
        _ => format!("{} {}", target(m), expected(m)),
    }
}

/// What to look at for a failure of `m` with this why, or `None` when there is nothing to add.
#[must_use]
pub fn hint(m: &Monitor, why: &str) -> Option<String> {
    let w = why.to_ascii_lowercase();
    let off = format!("`iohr monitors disable {}`", m.name);
    // The same failures read differently per OS: Windows says "actively refused" (os error
    // 10061), "did not properly respond" (10060) and "no such host is known" (11001).
    if w.contains("connection refused") || w.contains("actively refused") {
        return Some(format!(
            "nothing listens on that port; start it, or {off} until it runs"
        ));
    }
    if w.contains("timed out")
        || w.contains("would block")
        || w.contains("temporarily unavailable")
        || w.contains("did not properly respond")
    {
        return Some(match m.surface {
            Surface::Cli => {
                "the command ran past max_ms and was stopped; run it by hand to see where it waits"
                    .into()
            }
            _ => "it accepted but did not answer in time: stuck, overloaded, or behind a firewall"
                .into(),
        });
    }
    if w.contains("failed to lookup")
        || w.contains("nodename nor servname")
        || w.contains("no address")
        || w.contains("no such host")
    {
        return Some("the host name does not resolve; check the target".into());
    }
    if w.starts_with("took ") {
        return Some("it passed but slower than max_ms; find what it waits on, or raise max_ms if the new time is right".into());
    }
    if let Some(rest) = why.strip_prefix("shape ") {
        let schema = match (&m.expect.shape, m.surface) {
            (Some(Shape::One(p)), _) => p.clone(),
            (Some(Shape::Events(e)), _) => e.values().cloned().collect::<Vec<_>>().join(" or "),
            (None, Surface::Cli) => "the step's schema".into(),
            (None, _) => "its schema".into(),
        };
        let schema = if m.surface == Surface::Cli && rest.starts_with("step ") {
            cli_schema(m, rest).unwrap_or(schema)
        } else {
            schema
        };
        let at = rest
            .split_whitespace()
            .find(|p| p.starts_with('/'))
            .unwrap_or("/")
            .trim_end_matches(':');
        if w.contains("found another field") {
            return Some(format!(
                "the answer has a field {schema} does not name at {at}; type it there (and in an example) rather than opening additionalProperties"
            ));
        }
        if w.contains("found none") || w.contains("found nothing") || w.contains("missing") {
            return Some(format!(
                "a field {schema} requires is gone at {at}; fix the handler, or the schema if it was dropped on purpose"
            ));
        }
        return Some(format!(
            "the answer no longer matches {schema} at {at}; fix the handler or update the schema and its examples"
        ));
    }
    if let Some(code) = why
        .strip_prefix("answered ")
        .and_then(|r| r.split(|c: char| !c.is_ascii_digit()).next())
        .and_then(|c| c.parse::<u16>().ok())
    {
        return Some(match code {
            404 => "the route is gone or moved; check the path and the router".into(),
            401 | 403 => "it refused us: a credential or a permission changed".into(),
            500..=599 => "the server failed; read its log around that time".into(),
            200..=399 if m.refuse => "it answered where it should refuse: a guard is off".into(),
            _ => "the status changed; compare with expect.status".into(),
        });
    }
    if w.starts_with("content-type is \"text/html") {
        return Some("an HTML page came back where JSON was wanted; the app answers unknown /api paths with the web app, so the route may not be registered".into());
    }
    if w.contains(" header") && w.starts_with("no ") {
        return Some("the answer lacks a header expect.headers wants".into());
    }
    if w.contains("/exit: expected") {
        return Some(format!(
            "the command exited differently; run it by hand: {}",
            target(m)
        ));
    }
    if w.contains("data events, wanted") {
        return Some(
            "the stream opened but sent fewer events than wanted before the time ran out".into(),
        );
    }
    if w.starts_with("closed before") || w.starts_with("no answer") || w.starts_with("not an http")
    {
        return Some(
            "something listens but does not speak HTTP there, or it crashed mid-answer".into(),
        );
    }
    None
}

/// For a cli shape failure `step N ...`, step N's schema.
fn cli_schema(m: &Monitor, rest: &str) -> Option<String> {
    let n: usize = rest
        .strip_prefix("step ")?
        .split_whitespace()
        .next()?
        .parse()
        .ok()?;
    one_shape(&m.steps.get(n.checked_sub(1)?)?.expect)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn one(text: &str) -> Monitor {
        let f = crate::monitors::File {
            path: PathBuf::from("eye.toml"),
            value: crate::monitors::parse(text).unwrap(),
        };
        let r = crate::monitors::check_files(&[f]);
        assert!(r.problems.is_empty(), "{:?}", r.problems);
        r.monitors.into_iter().next().unwrap()
    }

    #[test]
    fn a_monitor_reads_as_one_sentence_and_its_failures_get_hints() {
        let api = one(
            "[[check]]\nname = \"inorbit\"\ntarget = \"http://127.0.0.1:7171/api/inorbit\"\nevery = \"60s\"\nexpect = { status = 200, max_ms = 2000, headers = { content-type = \"application/json\" }, shape = \"schemas/api/inorbit/v0.json\" }\n",
        );
        assert_eq!(
            checks(&api),
            "GET http://127.0.0.1:7171/api/inorbit answers 200, content-type application/json, under 2000 ms, body matches schemas/api/inorbit/v0.json"
        );
        let h = hint(
            &api,
            "shape 200 /web: expected no other fields, found another field",
        )
        .unwrap();
        assert!(
            h.contains("schemas/api/inorbit/v0.json")
                && h.contains("/web")
                && h.contains("additionalProperties"),
            "{h}"
        );
        assert!(
            hint(&api, "127.0.0.1:7173: Connection refused (os error 61)")
                .unwrap()
                .starts_with("nothing listens on that port")
        );
        for windows in [
            (
                "127.0.0.1:1: No connection could be made because the target machine actively refused it. (os error 10061)",
                "nothing listens on that port",
            ),
            (
                "10.0.0.1:443: A connection attempt failed because the connected party did not properly respond after a period of time, or established connection failed because connected host has failed to respond. (os error 10060)",
                "it accepted but did not answer in time",
            ),
            (
                "nowhere.invalid:443: No such host is known. (os error 11001)",
                "the host name does not resolve",
            ),
        ] {
            assert!(
                hint(&api, windows.0).unwrap().starts_with(windows.1),
                "{}",
                windows.0
            );
        }
        assert!(
            hint(&api, "answered 404, wanted 200")
                .unwrap()
                .contains("route")
        );
        assert!(
            hint(
                &api,
                "content-type is \"text/html; charset=utf-8\", wanted \"application/json\""
            )
            .unwrap()
            .contains("HTML")
        );
        assert_eq!(hint(&api, "something new"), None);

        let tcp = one("[[check]]\nname = \"share\"\ntarget = \"localhost:9494\"\nevery = \"5m\"\n");
        assert_eq!(checks(&tcp), "tcp localhost:9494 accepts a connection");

        let cli = one(
            "[[check]]\nname = \"board\"\ntarget = { cli = \"eye\" }\nargv = [\"scrum\", \"board\", \"--json\"]\nexpect = { shape = \"schemas/cli/scrum-board/v0.json\", max_ms = 5000 }\n",
        );
        assert_eq!(
            checks(&cli),
            "runs eye scrum board --json and exits 0, stdout matches schemas/cli/scrum-board/v0.json, under 5000 ms"
        );
        assert!(
            hint(&cli, "shape step 1 /cards: expected array, found object")
                .unwrap()
                .contains("schemas/cli/scrum-board/v0.json at /cards")
        );
    }
}
