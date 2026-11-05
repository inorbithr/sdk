//! `labs.reattribute-document` (RFC 0065.2, PRD 0184, ADR 0205): moves the proven rows of
//! ONE Decisions document from the person's subject to Io, the way core's
//! `labs:reattribute --document` does, and nothing else. It replaces the by-hand
//! `~/.inorbit/asks/reattribute-apply.sh` for one document.
//!
//! The account is the runner's own (the account its approvals belong to). Steps:
//!
//! 1. The evidence files of `evidence_ts` are checked against `SHA256SUMS` in
//!    `<state>/reattribute-evidence`.
//! 2. A fresh detached worktree of core at `origin/main` ([`CoreWorktree`]); it is
//!    removed on every way out. `mise run labs:reattribute --help` must list `--document`.
//! 3. A dry run; its `proven` rows are summed per kind and must equal the approved counts,
//!    else nothing is applied.
//! 4. `--apply`; its `run lrat_...` line is required and is the only line of the tool's
//!    output a report keeps, with the counts.
//! 5. A second dry run must show no proven row left for the document.
//!
//! The database address never leaves core's task (it reads it from the cluster); the
//! worktree needs no `.env` copied from the checkout, so no secret is copied or printed.

use std::collections::BTreeMap;
use std::io::Read as _;
use std::path::Path;

use serde_json::Value;
use sha2::{Digest, Sha256};

use super::{
    Action, ArgvRunner, CoreWorktree, LABS_REATTRIBUTE, Tool, closed, integer_param, redact_output,
    string_param, strings,
};

/// The largest count a message may name.
pub(crate) const MAX_COUNT: u64 = 1_000_000;

/// The folder under the runner's state folder that holds the evidence of a dry run.
const EVIDENCE: &str = "reattribute-evidence";

/// The three kinds of rows the tool counts.
const KINDS: [&str; 3] = ["document", "version", "comment"];

/// `^ldoc_[0-9A-Z]{26}$`.
pub(crate) fn is_document_id(s: &str) -> bool {
    s.strip_prefix("ldoc_").is_some_and(|rest| {
        rest.len() == 26
            && rest
                .bytes()
                .all(|c| c.is_ascii_digit() || c.is_ascii_uppercase())
    })
}

/// A lowercase UUID: `^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$`.
pub(crate) fn is_uuid(s: &str) -> bool {
    let parts: Vec<&str> = s.split('-').collect();
    parts.len() == 5
        && parts.iter().zip([8, 4, 4, 4, 12]).all(|(p, n)| {
            p.len() == n
                && p.bytes()
                    .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        })
}

/// `^[0-9]{8}T[0-9]{4}Z$`.
pub(crate) fn is_evidence_ts(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 14
        && b[8] == b'T'
        && b[13] == b'Z'
        && b[..8].iter().chain(&b[9..13]).all(u8::is_ascii_digit)
}

/// The catalog's sentence for this action (core's `pinned::effect_of`), or `None` when a
/// param is missing or of the wrong type.
pub(crate) fn effect(params: &Value) -> Option<String> {
    let count = |k: &str| {
        params
            .get(k)
            .and_then(Value::as_u64)
            .filter(|n| *n <= MAX_COUNT)
    };
    Some(format!(
        "Moves the proven rows of document {} (documents {}, versions {}, comments {}) to Io; it can be reverted with labs:reattribute --revert <run>.",
        params.get("document").and_then(Value::as_str)?,
        count("expected_documents")?,
        count("expected_versions")?,
        count("expected_comments")?
    ))
}

/// The params of a signed message, checked against the schema's rules.
///
/// # Errors
/// An unknown or missing param, or one the schema's pattern or range refuses.
pub(crate) fn params(params: &serde_json::Map<String, Value>) -> Result<(), String> {
    closed(
        params,
        &[
            "document",
            "from",
            "evidence_ts",
            "expected_documents",
            "expected_versions",
            "expected_comments",
        ],
        "/params",
    )?;
    string_param(
        params,
        "document",
        is_document_id,
        "a Decisions document id",
    )?;
    string_param(params, "from", is_uuid, "a subject")?;
    string_param(params, "evidence_ts", is_evidence_ts, "an evidence stamp")?;
    for k in [
        "expected_documents",
        "expected_versions",
        "expected_comments",
    ] {
        integer_param(params, k, 0, MAX_COUNT)?;
    }
    Ok(())
}

/// The params, checked again before anything runs.
///
/// # Errors
/// What [`params`] refuses.
pub(crate) fn check(
    document: &str,
    from: &str,
    evidence_ts: &str,
    expected: &Counts,
) -> Result<(), String> {
    if !is_document_id(document) {
        return Err("document: not a Decisions document id".to_owned());
    }
    if !is_uuid(from) {
        return Err("from: not a subject".to_owned());
    }
    if !is_evidence_ts(evidence_ts) {
        return Err("evidence_ts: not an evidence stamp".to_owned());
    }
    if [expected.documents, expected.versions, expected.comments]
        .iter()
        .any(|n| *n > MAX_COUNT)
    {
        return Err(format!("expected counts: not from 0 to {MAX_COUNT}"));
    }
    Ok(())
}

/// Rows per kind.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Counts {
    pub(crate) documents: u64,
    pub(crate) versions: u64,
    pub(crate) comments: u64,
}

impl Counts {
    fn show(self) -> String {
        format!(
            "documents {}, versions {}, comments {}",
            self.documents, self.versions, self.comments
        )
    }

    fn total(self) -> u64 {
        self.documents + self.versions + self.comments
    }
}

/// The `proven` rows of a dry run's stdout, summed per kind: lines
/// `<kind>\t<space>\t<class>\t<n>`; every other line is ignored.
///
/// # Errors
/// A row whose count is not a number.
pub(crate) fn proven(stdout: &str) -> Result<Counts, String> {
    let mut sums: BTreeMap<&str, u64> = BTreeMap::new();
    for line in stdout.lines() {
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() != 4 || !KINDS.contains(&f[0]) || f[2] != "proven" {
            continue;
        }
        let n: u64 = f[3]
            .trim()
            .parse()
            .map_err(|_| "the dry run printed a count that is not a number".to_owned())?;
        *sums.entry(f[0]).or_default() += n;
    }
    let get = |k: &str| sums.get(k).copied().unwrap_or(0);
    Ok(Counts {
        documents: get("document"),
        versions: get("version"),
        comments: get("comment"),
    })
}

/// The `run lrat_...: documents=N versions=N comments=N` line of an apply, with its
/// counts.
///
/// # Errors
/// There is no such line.
pub(crate) fn applied(stdout: &str) -> Result<(String, Counts), String> {
    stdout
        .lines()
        .find_map(|line| parse_applied(line.trim()))
        .ok_or_else(|| "apply printed no `run lrat_` line".to_owned())
}

fn parse_applied(line: &str) -> Option<(String, Counts)> {
    let (id, tail) = line.strip_prefix("run lrat_")?.split_once(": ")?;
    if id.is_empty() || !id.bytes().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    let mut parts = tail.split(' ').map(|p| {
        let (k, v) = p.split_once('=')?;
        Some((k, v.parse::<u64>().ok()?))
    });
    let (docs, vers, coms) = (parts.next()??, parts.next()??, parts.next()??);
    if parts.next().is_some() || (docs.0, vers.0, coms.0) != ("documents", "versions", "comments") {
        return None;
    }
    Some((
        format!("run lrat_{id}"),
        Counts {
            documents: docs.1,
            versions: vers.1,
            comments: coms.1,
        },
    ))
}

/// Checks the `-<ts>.jsonl` lines of `SHA256SUMS` in `dir` (both the audit and the MCP
/// files must be listed and match).
///
/// # Errors
/// A missing sums file or evidence file, a file listed twice or not at all, or a digest
/// that differs.
fn verify_evidence(dir: &Path, ts: &str) -> Result<(), String> {
    let sums = std::fs::read_to_string(dir.join("SHA256SUMS"))
        .map_err(|_| "evidence: SHA256SUMS cannot be read".to_owned())?;
    let suffix = format!("-{ts}.jsonl");
    let mut seen = Vec::new();
    for line in sums.lines().filter(|l| l.contains(&suffix)) {
        let (digest, name) = line
            .split_once(' ')
            .map(|(d, n)| (d, n.trim_start_matches([' ', '*'])))
            .ok_or_else(|| "evidence: a SHA256SUMS line is malformed".to_owned())?;
        if name.contains('/') || name.starts_with('.') || !name.ends_with(&suffix) {
            return Err("evidence: a SHA256SUMS name is not a plain file name".to_owned());
        }
        let mut file = std::fs::File::open(dir.join(name))
            .map_err(|_| format!("evidence: {name} cannot be read"))?;
        let mut h = Sha256::new();
        let mut buf = vec![0_u8; 64 * 1024];
        loop {
            let n = file
                .read(&mut buf)
                .map_err(|_| format!("evidence: {name} cannot be read"))?;
            if n == 0 {
                break;
            }
            h.update(&buf[..n]);
        }
        let got = super::super::claude::hex(&h.finalize());
        if !got.eq_ignore_ascii_case(digest) {
            return Err(format!("evidence {ts} does not match SHA256SUMS ({name})"));
        }
        seen.push(name.to_owned());
    }
    for want in [
        format!("labs-writes-{ts}.jsonl"),
        format!("mcp-decisions-{ts}.jsonl"),
    ] {
        if !seen.contains(&want) {
            return Err(format!("evidence: SHA256SUMS does not list {want}"));
        }
    }
    Ok(())
}

/// What a failed call said, reduced to its error lines (never the rest of the tool's
/// output: it can carry addresses).
fn errors_of(what: &str, out: &super::Output) -> String {
    let lines: Vec<String> = [out.stderr.as_str(), out.stdout.as_str()]
        .iter()
        .flat_map(|s| s.lines())
        .filter(|l| l.starts_with("Error") || l.starts_with("error"))
        .take(3)
        .map(redact_output)
        .collect();
    if lines.is_empty() {
        format!("{what}: failed")
    } else {
        format!("{what}: failed\n{}", lines.join("\n"))
    }
}

/// Carries out `labs.reattribute-document`.
///
/// # Errors
/// A refusal (params, evidence, no `--document` flag, counts that differ), a failed call,
/// an apply without a `run lrat_` line, or proven rows left after it.
pub(crate) async fn reattribute(
    action: &Action,
    tools: &impl ArgvRunner,
) -> Result<String, String> {
    let Action::ReattributeDocument {
        document,
        from,
        evidence_ts,
        expected,
    } = action
    else {
        return Err("not labs.reattribute-document".to_owned());
    };
    check(document, from, evidence_ts, expected)?;
    let account = tools.account()?;
    if !is_uuid(&account) {
        return Err("this runner's account id is not an account".to_owned());
    }
    let evidence = tools.state_dir()?.join(EVIDENCE);
    verify_evidence(&evidence, evidence_ts)?;
    let stamp = super::unique_name("reattribute");
    CoreWorktree::within(tools, &stamp, async |dir: &Path| {
        let mise = async |argv: Vec<String>| tools.run_in(Tool::CoreMise, dir, &argv).await;
        // The checkout is trusted by the host's mise, as the by-hand script did.
        mise(strings(["trust", "-q", "."])).await?;
        let help = mise(strings(["run", "-q", LABS_REATTRIBUTE, "--help"])).await?;
        if !help.success || !help.stdout.lines().any(|l| l.trim_start().starts_with("--document")) {
            return Err(format!(
                "refused: core's {LABS_REATTRIBUTE} has no --document flag at origin/main"
            ));
        }
        let labs_log = evidence.join(format!("labs-writes-{evidence_ts}.jsonl"));
        let mcp_log = evidence.join(format!("mcp-decisions-{evidence_ts}.jsonl"));
        let run_argv = |mode: &str| {
            let mut v = strings(["run", "-q", LABS_REATTRIBUTE, "--account"]);
            v.extend([
                account.clone(),
                "--from".to_owned(),
                from.clone(),
                "--labs-log".to_owned(),
                labs_log.display().to_string(),
                "--mcp-log".to_owned(),
                mcp_log.display().to_string(),
                "--unproven".to_owned(),
                "keep".to_owned(),
                "--document".to_owned(),
                document.clone(),
                mode.to_owned(),
            ]);
            v
        };
        let dry = mise(run_argv("--dry-run")).await?;
        if !dry.success {
            return Err(errors_of("labs:reattribute dry run", &dry));
        }
        let found = proven(&dry.stdout)?;
        if found != *expected {
            return Err(format!(
                "refused: the proven rows changed since the ask; nothing was applied\nfound: {}\napproved: {}",
                found.show(),
                expected.show()
            ));
        }
        let apply = mise(run_argv("--apply")).await?;
        let (line, moved) = match applied(&apply.stdout) {
            Ok(r) if apply.success => r,
            Ok((line, _)) => return Err(format!("{}\n{line}", errors_of("apply", &apply))),
            Err(e) => return Err(format!("{e}\n{}", errors_of("apply", &apply))),
        };
        let post = mise(run_argv("--dry-run")).await?;
        if !post.success {
            return Err(format!(
                "{line}\n{}",
                errors_of("the check after the apply", &post)
            ));
        }
        let left = proven(&post.stdout)?;
        if left.total() != 0 {
            return Err(format!(
                "{line}\nproven rows are left after the apply: {}",
                left.show()
            ));
        }
        if moved != *expected {
            return Err(format!(
                "{line}\nthe run moved {}, not the approved {}",
                moved.show(),
                expected.show()
            ));
        }
        Ok(format!(
            "{line}\nmoved {}; no proven row left; revert with labs:reattribute --revert <run>",
            moved.show()
        ))
    })
    .await
}

#[cfg(test)]
mod tests;
