//! How an ask goes to the Roles and how its answers come back (RFC 0213, S2).
//!
//! A `?`/`??` ask that finds a session host for its folder is fanned out: one task per Role in
//! the host's durable queue, tagged with that Role (`role_hint`), under one ask id. The Roles
//! take them at once, each on its own thread, so they answer concurrently. The shell reads the
//! tasks back with the host's `results` request and draws each Role's answer as it lands.
//! Nothing here opens the queue: the shell only speaks the socket ([`super::link`]).
//!
//! A task's `external_id` is `ask:<ask id>:<role>`, so asking again with the same id adds
//! nothing, and the tasks of one ask are found by their prefix. Its payload is a [`Payload`]
//! as JSON; what a Role returns is stored as the task's `result_ref`, a [`Reply`] as JSON
//! (the answer's text is cut at [`TEXT_MAX`]). Both live in the host's folder, which only the
//! owner can open; neither is ever logged.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use super::lifecycle::Paths;
use super::link;
use super::queue::now_ms;

/// The most characters of an answer's text kept in the queue.
pub(crate) const TEXT_MAX: usize = 48 * 1024;

/// The most bytes of a payload line the host accepts (the host reads 64 KiB a line, and the
/// line carries the payload as an escaped string).
const PAYLOAD_MAX: usize = 24 * 1024;

/// What a Role is asked: the question and how to ask it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Payload {
    /// Always `"ask"`: the payload's kind.
    pub(crate) kind: String,
    pub(crate) ask: String,
    pub(crate) question: String,
    /// The folder the question is about.
    pub(crate) cwd: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) job: Option<String>,
}

impl Payload {
    pub(crate) fn new(ask: &str, question: &str, cwd: &str) -> Self {
        Self {
            kind: "ask".to_owned(),
            ask: ask.to_owned(),
            question: question.to_owned(),
            cwd: cwd.to_owned(),
            mode: None,
            model: None,
            agent: None,
            job: None,
        }
    }

    /// The payload of a task, or why it is not an ask.
    pub(crate) fn parse(text: &str) -> Result<Self, String> {
        let p: Self = serde_json::from_str(text).map_err(|e| format!("not an ask task: {e}"))?;
        if p.kind == "ask" {
            Ok(p)
        } else {
            Err(format!("not an ask task: kind {:?}", p.kind))
        }
    }
}

/// What a Role answered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Reply {
    pub(crate) text: String,
    #[serde(default)]
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) duration_ms: u64,
}

impl Reply {
    /// The reply as the task's `result_ref`, its text cut to [`TEXT_MAX`] characters.
    pub(crate) fn stored(&self) -> String {
        let text: String = self.text.chars().take(TEXT_MAX).collect();
        serde_json::json!({
            "text": text,
            "model": self.model,
            "duration_ms": self.duration_ms,
        })
        .to_string()
    }
}

/// An ask id is letters and digits, and short.
pub(crate) fn valid_ask(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// The start of every task's `external_id` in an ask.
pub(crate) fn prefix(ask: &str) -> String {
    format!("ask:{ask}:")
}

/// A new ask id: this moment, this process and a count, so two asks never share one.
pub(crate) fn new_ask_id() -> String {
    static COUNT: AtomicU64 = AtomicU64::new(0);
    format!(
        "{:x}{:x}{:x}",
        now_ms(),
        std::process::id(),
        COUNT.fetch_add(1, Ordering::Relaxed)
    )
}

/// The running host for the folder a question is asked in: the nearest folder above it (or
/// itself) that a host runs for.
pub(crate) fn find_host(cwd: &Path, data: &Path) -> Option<Paths> {
    cwd.ancestors()
        .map(|dir| Paths::for_project(data, dir))
        .find(|paths| {
            // A host that is stopping answers `status` but takes no more work.
            link::ask(paths, "status")
                .is_ok_and(|a| a.starts_with("ok ") && !a.ends_with(" stopping"))
        })
}

/// The Roles the host runs; `None` when it does not answer or does not know the request (an
/// older host), in which case the ask goes the way it always did.
pub(crate) fn hosted_roles(paths: &Paths) -> Option<Vec<String>> {
    let answer = link::ask(paths, "roles").ok()?;
    let list = answer.strip_prefix("ok ")?;
    let names: Vec<String> = list
        .split(',')
        .filter(|n| !n.is_empty())
        .map(str::to_owned)
        .collect();
    (!names.is_empty()).then_some(names)
}

/// Puts one task per Role in the host's queue. Every task is its Role's; a Role that is not
/// alive leaves its task to the others (the queue's own rule). An error says why nothing, or
/// not everything, was queued; what was queued is answered all the same.
pub(crate) fn fan_out(paths: &Paths, payload: &Payload, roles: &[String]) -> Result<(), String> {
    let body = serde_json::to_string(payload).map_err(|e| e.to_string())?;
    if body.len() > PAYLOAD_MAX {
        return Err("the question is too long to hand to the Roles".to_owned());
    }
    for role in roles {
        let request = serde_json::json!({
            "external_id": format!("{}{role}", prefix(&payload.ask)),
            "role_hint": role,
            "payload": body,
        });
        let answer = link::ask(paths, &format!("enqueue {request}")).map_err(|e| e.to_string())?;
        if !answer.starts_with("queued ") {
            return Err(format!("the host answered: {answer}"));
        }
    }
    Ok(())
}

/// Where one Role's share of an ask stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// Queued or being answered.
    Working,
    Answered(Reply),
    /// It ended without an answer, with the reason.
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Landed {
    pub(crate) role: String,
    pub(crate) outcome: Outcome,
}

/// Reads the ask's tasks back from the host, one per Role, in the order they were queued.
pub(crate) fn poll(paths: &Paths, ask: &str) -> Result<Vec<Landed>, String> {
    let line = link::ask(paths, &format!("results {ask}")).map_err(|e| e.to_string())?;
    landed(&line)
}

fn landed(line: &str) -> Result<Vec<Landed>, String> {
    #[derive(Deserialize)]
    struct Row {
        role: String,
        state: String,
        result: Option<String>,
        reason: Option<String>,
    }
    let rows: Vec<Row> =
        serde_json::from_str(line).map_err(|_| format!("the host answered: {line}"))?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let outcome = match r.state.as_str() {
                "done" => match r.result.as_deref().map(serde_json::from_str::<Reply>) {
                    Some(Ok(reply)) => Outcome::Answered(reply),
                    _ => Outcome::Failed("the answer could not be read".to_owned()),
                },
                "failed" => Outcome::Failed(r.reason.unwrap_or_else(|| "it failed".to_owned())),
                _ => Outcome::Working,
            };
            Landed {
                role: r.role,
                outcome,
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_ask_id_is_short_letters_and_digits_and_never_repeats() {
        let (a, b) = (new_ask_id(), new_ask_id());
        assert_ne!(a, b);
        assert!(valid_ask(&a) && valid_ask(&b), "{a} {b}");
        assert!(!valid_ask("") && !valid_ask("a:b") && !valid_ask("a b"));
        assert!(!valid_ask(&"a".repeat(65)));
    }

    #[test]
    fn a_payload_round_trips_and_a_stranger_is_refused() {
        let mut p = Payload::new("ab12", "why does it fail?", "/work");
        p.mode = Some("plan".into());
        let text = serde_json::to_string(&p).unwrap();
        assert_eq!(Payload::parse(&text).unwrap(), p);
        assert!(
            Payload::parse("{\"kind\":\"build\",\"ask\":\"a\",\"question\":\"q\",\"cwd\":\"/\"}")
                .is_err()
        );
        assert!(Payload::parse("p").is_err());
    }

    #[test]
    fn a_stored_reply_is_cut_and_read_back() {
        let reply = Reply {
            text: "x".repeat(TEXT_MAX + 10),
            model: "haiku".into(),
            duration_ms: 7,
        };
        let back: Reply = serde_json::from_str(&reply.stored()).unwrap();
        assert_eq!(back.text.chars().count(), TEXT_MAX);
        assert_eq!((back.model.as_str(), back.duration_ms), ("haiku", 7));
    }

    #[test]
    fn rows_become_working_answered_or_failed() {
        let line = r#"[
            {"role":"a","state":"done","result":"{\"text\":\"hi\",\"model\":\"m\",\"duration_ms\":1}","reason":null},
            {"role":"b","state":"running","result":null,"reason":null},
            {"role":"c","state":"failed","result":null,"reason":"boom"},
            {"role":"d","state":"done","result":"nope","reason":null}
        ]"#;
        let got = landed(line).unwrap();
        assert!(matches!(&got[0].outcome, Outcome::Answered(r) if r.text == "hi"));
        assert_eq!(got[1].outcome, Outcome::Working);
        assert_eq!(got[2].outcome, Outcome::Failed("boom".into()));
        assert!(matches!(&got[3].outcome, Outcome::Failed(_)));
        assert!(landed("error busy").is_err());
    }
}
