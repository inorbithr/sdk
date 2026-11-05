use super::super::testing::{Fake, fake};
use super::super::{Action, Output, Tool, WORKTREE_DIR, carry_out_on, lookup_on};
use super::*;
use serde_json::json;
use std::fmt::Write as _;

const DOC: &str = "ldoc_01M4P30371W9A3V5S3YCAHRTAN";
const FROM: &str = "2561c570-bde2-4998-82b7-f4cde4ece7c6";
const ACCOUNT: &str = "0a1b2c3d-0000-4000-8000-000000000001";
const TS: &str = "20261101T0038Z";

fn ok(stdout: &str) -> Output {
    Output::new(true, stdout, "")
}

fn counts(d: u64, v: u64, c: u64) -> Counts {
    Counts {
        documents: d,
        versions: v,
        comments: c,
    }
}

fn act() -> Action {
    Action::ReattributeDocument {
        document: DOC.to_owned(),
        from: FROM.to_owned(),
        evidence_ts: TS.to_owned(),
        expected: counts(1, 2, 3),
    }
}

/// A dry run's stdout: the proven rows of the three kinds, and audited rows that never count.
fn dry(d: u64, v: u64, c: u64) -> Output {
    ok(&format!(
        "evidence: proven={} audited=9 skipped_lines=0\n\
         document\tspace_a\tproven\t{d}\n\
         document\tspace_a\taudited\t7\n\
         version\tspace_a\tproven\t{v}\n\
         comment\tspace_a\tproven\t{c}\n\
         comment\tspace_a\tunlogged\t4\n",
        d + v + c
    ))
}

const HELP: &str = "Usage: labs:reattribute\n  --account <id>\n  --document <ldoc_id>\n";
const APPLIED: &str = "run lrat_01M6NWQSN3GGDDS70D547AH0BG: documents=1 versions=2 comments=3\n";

struct Host {
    fake: Fake,
    _dir: tempfile::TempDir,
}

/// A host whose state folder holds good evidence; `mid` is what the calls between the
/// worktree being added and removed answer, in order: trust, help, dry run, apply, check.
fn host(mid: Vec<Output>) -> Host {
    let dir = tempfile::tempdir().unwrap();
    let e = dir.path().join(EVIDENCE);
    std::fs::create_dir_all(&e).unwrap();
    let mut sums = String::new();
    for name in [
        format!("labs-writes-{TS}.jsonl"),
        format!("mcp-decisions-{TS}.jsonl"),
        "labs-writes-20260101T0000Z.jsonl".to_owned(),
    ] {
        let body = format!("{{\"n\": \"{name}\"}}\n");
        std::fs::write(e.join(&name), &body).unwrap();
        let digest = super::super::super::claude::hex(&Sha256::digest(body.as_bytes()));
        let _ = writeln!(sums, "{digest}  {name}");
    }
    std::fs::write(e.join("SHA256SUMS"), sums).unwrap();
    let mut f = fake("", false);
    f.state = Some(dir.path().to_owned());
    f.account = Some(ACCOUNT.to_owned());
    {
        let mut s = f.script.lock().unwrap();
        // prune, fetch, add
        s.extend([ok(""), ok(""), ok("")]);
        s.extend(mid);
    }
    Host { fake: f, _dir: dir }
}

fn good() -> Vec<Output> {
    vec![ok(""), ok(HELP), dry(1, 2, 3), ok(APPLIED), dry(0, 0, 0)]
}

fn argvs(f: &Fake) -> Vec<String> {
    f.calls()
        .iter()
        .map(|(t, a)| format!("{} {}", t.name(), a.join(" ")))
        .collect()
}

fn last_git(f: &Fake) -> Vec<String> {
    f.calls()
        .iter()
        .rev()
        .find(|(t, a)| {
            *t == Tool::Git && a.first().is_some_and(|x| x == "worktree") && a[1] == "remove"
        })
        .map(|(_, a)| a.clone())
        .unwrap_or_default()
}

async fn run(h: &Host) -> Result<String, String> {
    carry_out_on(&act(), &h.fake, "linux").await
}

#[test]
fn params_and_ids_hold_at_their_edges() {
    assert!(is_document_id(DOC));
    assert!(!is_document_id("ldoc_01m4P30371W9A3V5S3YCAHRTAN"));
    assert!(!is_document_id("ldoc_01M4P30371W9A3V5S3YCAHRTA"));
    assert!(is_uuid(FROM));
    assert!(!is_uuid(&FROM.to_uppercase()));
    assert!(!is_uuid("2561c570bde24998-82b7-f4cde4ece7c6"));
    assert!(is_evidence_ts(TS));
    assert!(!is_evidence_ts("20261101T0038"));
    assert!(!is_evidence_ts("2026110aT0038Z"));
    let doc = |p: serde_json::Value| json!({"action": "labs.reattribute-document", "params": p});
    let good = json!({"document": DOC, "from": FROM, "evidence_ts": TS,
        "expected_documents": 1, "expected_versions": 2, "expected_comments": 3});
    assert_eq!(lookup_on(&doc(good.clone()), "linux").unwrap(), act());
    let mut big = good.clone();
    big["expected_comments"] = json!(1_000_001);
    assert!(params(big.as_object().unwrap()).is_err());
    let mut extra = good.clone();
    extra["unproven"] = json!("keep");
    assert!(params(extra.as_object().unwrap()).is_err());
    let mut text = good;
    text["expected_documents"] = json!("1");
    assert!(params(text.as_object().unwrap()).is_err());
}

#[test]
fn only_the_proven_rows_are_counted_per_kind() {
    assert_eq!(proven(&dry(1, 2, 3).stdout).unwrap(), counts(1, 2, 3));
    let two = "document\ts1\tproven\t1\ndocument\ts2\tproven\t2\n";
    assert_eq!(proven(two).unwrap(), counts(3, 0, 0));
    assert!(proven("document\ts\tproven\tmany\n").is_err());
    assert_eq!(proven("").unwrap(), counts(0, 0, 0));
}

#[test]
fn the_apply_line_needs_its_counts() {
    let (line, c) = applied(APPLIED).unwrap();
    assert_eq!(line, "run lrat_01M6NWQSN3GGDDS70D547AH0BG");
    assert_eq!(c, counts(1, 2, 3));
    assert!(applied("run lrat_x: documents=1\n").is_err());
    assert!(applied("done\n").is_err());
}

#[tokio::test]
async fn a_matching_run_moves_the_rows_and_removes_the_worktree() {
    let h = host(good());
    let out = run(&h).await.unwrap();
    assert!(
        out.starts_with("run lrat_01M6NWQSN3GGDDS70D547AH0BG\n"),
        "{out}"
    );
    assert!(out.contains("documents 1, versions 2, comments 3"), "{out}");
    // Nothing of the tool's other output (spaces, rows) is reported.
    assert!(
        !out.contains("space_a") && !out.contains("evidence:"),
        "{out}"
    );
    let a = argvs(&h.fake);
    assert_eq!(a[0], "git worktree prune");
    assert_eq!(a[1], "git fetch origin main");
    assert!(a[2].starts_with("git worktree add --detach "), "{a:?}");
    assert!(a[2].ends_with(" origin/main"), "{a:?}");
    assert_eq!(a[3], "mise trust -q .");
    assert_eq!(a[4], "mise run -q labs:reattribute --help");
    let flags = |n: usize, mode: &str| {
        a[n].contains(&format!("--account {ACCOUNT} --from {FROM} "))
            && a[n].contains("--unproven keep")
            && a[n].contains(&format!("--document {DOC} {mode}"))
            && a[n].contains(&format!("labs-writes-{TS}.jsonl"))
            && a[n].contains(&format!("mcp-decisions-{TS}.jsonl"))
    };
    assert!(flags(5, "--dry-run"), "{a:?}");
    assert!(flags(6, "--apply"), "{a:?}");
    assert!(flags(7, "--dry-run"), "{a:?}");
    assert!(a[8].starts_with("git worktree remove --force "), "{a:?}");
    // Every mise call ran in the one worktree, which is under the state folder.
    let folders = h.fake.folders.lock().unwrap().clone();
    assert_eq!(folders.len(), 5);
    assert!(folders.iter().all(|f| *f == folders[0]));
    assert!(folders[0].starts_with(h.fake.state.as_ref().unwrap().join(WORKTREE_DIR)));
    assert!(a[2].contains(&folders[0].display().to_string()));
}

#[tokio::test]
async fn counts_that_changed_apply_nothing_and_still_remove_the_worktree() {
    let mut mid = good();
    mid[2] = dry(1, 2, 4);
    let h = host(mid);
    let e = run(&h).await.unwrap_err();
    assert!(e.contains("nothing was applied"), "{e}");
    assert!(
        e.contains("found: documents 1, versions 2, comments 4"),
        "{e}"
    );
    assert!(
        e.contains("approved: documents 1, versions 2, comments 3"),
        "{e}"
    );
    assert!(!argvs(&h.fake).iter().any(|a| a.contains("--apply")));
    assert_ne!(last_git(&h.fake).len(), 0);
}

#[tokio::test]
async fn no_document_flag_refuses_before_any_dry_run() {
    let mut mid = good();
    mid[1] = ok("Usage: labs:reattribute\n  --account <id>\n");
    let h = host(mid);
    let e = run(&h).await.unwrap_err();
    assert!(e.contains("has no --document flag"), "{e}");
    assert!(!argvs(&h.fake).iter().any(|a| a.contains("--dry-run")));
    assert_ne!(last_git(&h.fake).len(), 0);
}

#[tokio::test]
async fn an_apply_without_its_run_line_is_refused_without_the_tool_output() {
    let mut mid = good();
    mid[3] = Output::new(false, "title: Secret plan\n", "Error: database down\nnoise");
    let h = host(mid);
    let e = run(&h).await.unwrap_err();
    assert!(e.contains("no `run lrat_` line"), "{e}");
    assert!(e.contains("Error: database down"), "{e}");
    assert!(!e.contains("Secret plan") && !e.contains("noise"), "{e}");
    assert_ne!(last_git(&h.fake).len(), 0);
}

#[tokio::test]
async fn proven_rows_left_after_the_apply_are_refused_with_the_run_line() {
    let mut mid = good();
    mid[4] = dry(0, 1, 0);
    let h = host(mid);
    let e = run(&h).await.unwrap_err();
    assert!(e.starts_with("run lrat_"), "{e}");
    assert!(e.contains("proven rows are left"), "{e}");
    assert_ne!(last_git(&h.fake).len(), 0);
}

#[tokio::test]
async fn a_failed_fetch_runs_nothing_in_a_worktree_but_still_cleans_up() {
    let h = host(Vec::new());
    {
        let mut s = h.fake.script.lock().unwrap();
        s.clear();
        s.extend([ok(""), Output::new(false, "", "fatal: no route")]);
    }
    let e = run(&h).await.unwrap_err();
    assert!(e.contains("git fetch: failed"), "{e}");
    assert!(h.fake.folders.lock().unwrap().is_empty());
    assert_ne!(last_git(&h.fake).len(), 0);
}

#[tokio::test]
async fn evidence_that_does_not_match_stops_everything_before_a_tool_runs() {
    let h = host(good());
    let file = h
        .fake
        .state
        .as_ref()
        .unwrap()
        .join(EVIDENCE)
        .join(format!("mcp-decisions-{TS}.jsonl"));
    std::fs::write(&file, "changed\n").unwrap();
    let e = run(&h).await.unwrap_err();
    assert!(e.contains("does not match SHA256SUMS"), "{e}");
    assert!(h.fake.ran_nothing());
}

#[tokio::test]
async fn evidence_missing_from_the_sums_is_refused() {
    let h = host(good());
    let sums = h
        .fake
        .state
        .as_ref()
        .unwrap()
        .join(EVIDENCE)
        .join("SHA256SUMS");
    let kept = std::fs::read_to_string(&sums)
        .unwrap()
        .lines()
        .filter(|l| !l.contains("mcp-decisions"))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    std::fs::write(&sums, kept).unwrap();
    let e = run(&h).await.unwrap_err();
    assert!(e.contains("does not list mcp-decisions"), "{e}");
    assert!(h.fake.ran_nothing());
}

#[tokio::test]
async fn a_runner_without_an_account_refuses() {
    let mut h = host(good());
    h.fake.account = None;
    assert!(run(&h).await.unwrap_err().contains("account"));
    assert!(h.fake.ran_nothing());
}
