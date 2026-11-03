use std::path::Path;

use serde_json::json;

use super::super::testing::{Fake, fake};
use super::super::{Action, Output, Tool, carry_out_on, effect_of, lookup_on, validate};
use super::*;

const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";
const SOURCE: &str = "[[check]]\nid = \"a\"\n\n[[check]]\nid = \"b\"\n";

fn action(block: &str) -> Action {
    Action::InstallChecks {
        block: block.to_owned(),
        commit: COMMIT.to_owned(),
        service: DEFAULT_SERVICE.to_owned(),
    }
}

fn ok(stdout: &str) -> Output {
    Output::new(true, stdout, "")
}

fn status(connected: bool, checks: usize) -> Output {
    ok(&json!({"connected": connected, "policy": {"checks": checks}}).to_string())
}

/// A host whose scripted answers are the git reads of a good install, then `rest`.
fn host(dir: &Path, source: &str, rest: Vec<Output>) -> Fake {
    let f = fake("", false);
    {
        let mut s = f.script.lock().unwrap();
        s.push_back(ok("")); // fetch
        s.push_back(ok("")); // merge-base
        s.push_back(ok(&format!("{}\n", source.len()))); // cat-file -s
        s.push_back(ok(source)); // show
        s.extend(rest);
    }
    Fake {
        checks_file: Some(dir.join("checks.toml")),
        ..f
    }
}

fn names(dir: &Path) -> Vec<String> {
    let mut n: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    n.sort();
    n
}

const OLD: &str = "# top\n\n# BEGIN gates\n[[refuse]]\nid = \"g\"\n# END gates\n\n# BEGIN contract (from core x at 00)\n[[check]]\nid = \"old\"\n# END contract\n\n# tail\n";

#[test]
fn a_block_is_replaced_between_its_markers_and_nothing_else_moves() {
    let got = put_block(OLD, "contract", "p.toml", COMMIT, SOURCE).unwrap();
    assert_eq!(
        got,
        format!(
            "# top\n\n# BEGIN gates\n[[refuse]]\nid = \"g\"\n# END gates\n\n# BEGIN contract (from core p.toml at {COMMIT})\n{SOURCE}# END contract\n\n# tail\n"
        )
    );
    // The same install twice is the same file.
    assert_eq!(
        put_block(&got, "contract", "p.toml", COMMIT, SOURCE).unwrap(),
        got
    );
}

#[test]
fn a_missing_block_is_appended_after_a_blank_line() {
    let got = put_block("[[check]]\nid = \"x\"", "contract", "p", COMMIT, SOURCE).unwrap();
    assert!(got.starts_with("[[check]]\nid = \"x\"\n\n# BEGIN contract (from core p at "));
    assert!(got.ends_with("# END contract\n"));
    let empty = put_block("", "gates", "p", COMMIT, SOURCE).unwrap();
    assert!(empty.starts_with("# BEGIN gates (from core p at "));
}

#[test]
fn a_block_of_another_name_is_not_this_block() {
    let cur = "# BEGIN contract-extra\n[[check]]\n# END contract-extra\n";
    let got = put_block(cur, "contract", "p", COMMIT, SOURCE).unwrap();
    assert!(got.starts_with(cur));
    assert_eq!(got.matches("# BEGIN contract").count(), 2);
}

#[test]
fn broken_markers_and_marked_sources_are_refused() {
    let open = "# BEGIN contract\n[[check]]\n";
    assert!(
        put_block(open, "contract", "p", COMMIT, SOURCE)
            .unwrap_err()
            .contains("has no # END contract")
    );
    let twice = "# BEGIN contract\n# END contract\n# BEGIN contract\n# END contract\n";
    assert!(
        put_block(twice, "contract", "p", COMMIT, SOURCE)
            .unwrap_err()
            .contains("more than one")
    );
    assert!(
        put_block("", "contract", "p", COMMIT, "# END gates\n[[check]]\n")
            .unwrap_err()
            .contains("marker lines of its own")
    );
}

#[test]
fn the_declared_count_is_checks_and_refusals() {
    assert_eq!(declared(OLD), 2);
    assert_eq!(declared(SOURCE), 2);
    assert_eq!(declared("[[checkx]]\n[check]\n"), 0);
}

#[test]
fn the_backup_is_named_by_the_utc_time() {
    let at = time::macros::datetime!(2026-10-31 12:05:09 UTC);
    assert_eq!(
        backup_path(Path::new("/h/.config/iohr-agent/checks.toml"), at).unwrap(),
        Path::new("/h/.config/iohr-agent/checks.toml.bak-20261031T120509Z")
    );
}

#[test]
fn every_block_has_a_source_in_core_and_nothing_else_does() {
    for (b, p) in SOURCES {
        assert_eq!(source_of(b), Some(p));
    }
    assert_eq!(source_of("../x"), None);
    assert!(!is_block("other"));
}

#[tokio::test]
async fn a_good_install_backs_up_replaces_restarts_and_waits_for_the_agent() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("checks.toml"), OLD).unwrap();
    // lint, daemon-reload, restart, then not yet connected, then connected with 3 entries
    // (the old file's gates entry and the two of the source).
    let h = host(
        dir.path(),
        SOURCE,
        vec![ok(""), ok(""), ok(""), status(false, 0), status(true, 3)],
    );
    let said = carry_out_on(&action("contract"), &h, "linux")
        .await
        .unwrap();
    assert!(said.starts_with("contract: 3 checks in checks.toml at 0123456789ab"));
    assert!(said.contains("iohr-agent.service restarted and connected"));
    let calls = h.calls();
    let words: Vec<String> = calls
        .iter()
        .map(|(t, a)| format!("{} {}", t.name(), a[..a.len().min(2)].join(" ")))
        .collect();
    assert_eq!(
        words,
        [
            "git fetch --quiet",
            "git merge-base --is-ancestor",
            "git cat-file -s",
            "git show 0123456789abcdef0123456789abcdef01234567:devops/checks/generated/agent.toml",
            "iohr agent checks",
            "systemctl --user daemon-reload",
            "systemctl --user restart",
            "iohr agent status",
            "iohr agent status",
        ]
    );
    assert_eq!(calls[6].1[2], "iohr-agent.service");
    let now = std::fs::read_to_string(dir.path().join("checks.toml")).unwrap();
    assert!(now.contains(
        "# BEGIN contract (from core devops/checks/generated/agent.toml at 0123456789abcdef"
    ));
    assert!(now.contains("# BEGIN gates"));
    assert!(!now.contains("\"old\""));
    let n = names(dir.path());
    assert_eq!(n.len(), 2, "{n:?}");
    let bak = n.iter().find(|n| n.contains(".bak-")).unwrap();
    assert_eq!(std::fs::read_to_string(dir.path().join(bak)).unwrap(), OLD);
}

#[tokio::test]
async fn the_linted_file_is_the_candidate_beside_the_real_one() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("checks.toml"), OLD).unwrap();
    let h = host(
        dir.path(),
        SOURCE,
        vec![ok(""), ok(""), ok(""), status(true, 3)],
    );
    carry_out_on(&action("contract"), &h, "linux")
        .await
        .unwrap();
    let lint = &h.calls()[4].1;
    assert_eq!(lint[..4], ["agent", "checks", "lint", "--file"]);
    let file = Path::new(&lint[4]);
    assert_eq!(file.parent(), Some(dir.path()));
    assert_ne!(file, dir.path().join("checks.toml"));
}

#[tokio::test]
async fn a_failed_lint_leaves_the_file_alone_and_restarts_nothing() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("checks.toml"), OLD).unwrap();
    let h = host(
        dir.path(),
        SOURCE,
        vec![Output::new(false, "", "line 4: unknown field")],
    );
    let err = carry_out_on(&action("contract"), &h, "linux")
        .await
        .unwrap_err();
    assert!(err.contains("the checks file is unchanged"), "{err}");
    assert!(err.contains("unknown field"));
    assert!(h.calls().iter().all(|(t, _)| *t != Tool::Systemctl));
    assert_eq!(
        std::fs::read_to_string(dir.path().join("checks.toml")).unwrap(),
        OLD
    );
    assert_eq!(
        names(dir.path()),
        ["checks.toml"],
        "no backup, no candidate left"
    );
}

#[tokio::test]
async fn a_commit_that_is_not_on_origin_main_is_refused_before_it_is_read() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("checks.toml"), OLD).unwrap();
    let h = fake("", false);
    h.script
        .lock()
        .unwrap()
        .extend([ok(""), Output::new(false, "", "")]);
    let h = Fake {
        checks_file: Some(dir.path().join("checks.toml")),
        ..h
    };
    let err = carry_out_on(&action("contract"), &h, "linux")
        .await
        .unwrap_err();
    assert!(err.contains("is not on origin/main"), "{err}");
    assert_eq!(h.calls().len(), 2, "fetch and merge-base only");
    assert_eq!(names(dir.path()), ["checks.toml"]);
}

#[tokio::test]
async fn a_source_over_the_cap_or_read_short_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("checks.toml"), OLD).unwrap();
    let f = |size: &str, body: &str| {
        let h = fake("", false);
        h.script
            .lock()
            .unwrap()
            .extend([ok(""), ok(""), ok(size), ok(body)]);
        Fake {
            checks_file: Some(dir.path().join("checks.toml")),
            ..h
        }
    };
    let err = carry_out_on(&action("contract"), &f("99999\n", SOURCE), "linux")
        .await
        .unwrap_err();
    assert!(err.contains("over the 16384"), "{err}");
    let err = carry_out_on(&action("contract"), &f("999\n", SOURCE), "linux")
        .await
        .unwrap_err();
    assert!(err.contains("read short"), "{err}");
    let err = carry_out_on(&action("contract"), &f("4\n", "# a\n"), "linux")
        .await
        .unwrap_err();
    assert!(err.contains("declares no check"), "{err}");
}

#[tokio::test]
async fn a_file_over_the_platform_limit_is_refused_before_lint() {
    let dir = tempfile::tempdir().unwrap();
    let big = "[[check]]\n".repeat(101);
    std::fs::write(dir.path().join("checks.toml"), "").unwrap();
    let h = host(dir.path(), &big, vec![]);
    let err = carry_out_on(&action("gates"), &h, "linux")
        .await
        .unwrap_err();
    assert!(err.contains("101 checks, over the platform's 100"), "{err}");
    assert_eq!(h.calls().len(), 4, "git reads only");
}

#[tokio::test]
async fn an_agent_that_does_not_settle_gets_the_old_file_back() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("checks.toml"), OLD).unwrap();
    // restart works, but the agent never connects.
    let mut rest = vec![ok(""), ok(""), ok("")];
    rest.extend((0..STATUS_TRIES).map(|_| status(false, 0)));
    // the second restart, and its reload
    rest.extend([ok(""), ok("")]);
    let h = host(dir.path(), SOURCE, rest);
    let err = carry_out_on(&action("contract"), &h, "linux")
        .await
        .unwrap_err();
    assert!(
        err.contains("not connected and holds 0 of 3 checks"),
        "{err}"
    );
    assert!(
        err.contains("the old checks file is back from checks.toml.bak-"),
        "{err}"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("checks.toml")).unwrap(),
        OLD
    );
    let restarts = h
        .calls()
        .iter()
        .filter(|(t, a)| *t == Tool::Systemctl && a[1] == "restart")
        .count();
    assert_eq!(restarts, 2, "once for the new file, once for the old");
}

#[tokio::test]
async fn a_failed_restart_also_gets_the_old_file_back() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("checks.toml"), OLD).unwrap();
    let h = host(
        dir.path(),
        SOURCE,
        vec![
            ok(""),
            ok(""),
            Output::new(false, "", "Unit not found"),
            ok(""),
            ok(""),
        ],
    );
    let err = carry_out_on(&action("contract"), &h, "linux")
        .await
        .unwrap_err();
    assert!(err.contains("systemctl restart: failed"), "{err}");
    assert!(err.contains("Unit not found"));
    assert_eq!(
        std::fs::read_to_string(dir.path().join("checks.toml")).unwrap(),
        OLD
    );
}

#[tokio::test]
async fn only_a_linux_runner_installs_and_it_runs_nothing_elsewhere() {
    let dir = tempfile::tempdir().unwrap();
    let h = host(dir.path(), SOURCE, vec![]);
    let err = carry_out_on(&action("contract"), &h, "macos")
        .await
        .unwrap_err();
    assert!(err.contains("only a Linux runner"), "{err}");
    assert!(h.ran_nothing());
}

fn doc(params: &serde_json::Value) -> serde_json::Value {
    json!({
        "action": "agent.install-checks",
        "runner_id": "rnr_00000000000000000000000001",
        "params": params,
        "effect": effect_of("agent.install-checks", params).unwrap_or_else(|| "Placeholder.".to_owned()),
    })
}

#[test]
fn the_catalog_sentence_names_the_block_the_commit_and_the_service() {
    let p = json!({"block": "gates", "commit": COMMIT});
    assert_eq!(
        effect_of("agent.install-checks", &p).unwrap(),
        format!(
            "Installs the gates checks from core at commit {COMMIT} into the agent's checks file and restarts iohr-agent."
        )
    );
    let p = json!({"block": "gates", "commit": COMMIT, "service": "other-agent"});
    assert!(
        effect_of("agent.install-checks", &p)
            .unwrap()
            .ends_with("restarts other-agent.")
    );
}

#[test]
fn the_roll_task_and_sweep_sentences_are_core_s_words() {
    let p = json!({"rev": COMMIT, "task": "roll:services", "services": "a b"});
    assert_eq!(
        effect_of("roll.task", &p).unwrap(),
        format!("Runs roll task roll:services for a b at commit {COMMIT} on the deploy server.")
    );
    let p = json!({"rev": COMMIT, "task": "roll:check", "services": ""});
    assert!(!effect_of("roll.task", &p).unwrap().contains(" for "));
    assert_eq!(
        effect_of("build.sweep-targets", &json!({"mode": "guard"})).unwrap(),
        "Sweeps the Rust build targets on the runner's host's nearly full volumes."
    );
}

#[test]
fn the_document_is_checked_and_looked_up_with_the_default_service() {
    let good = doc(&json!({"block": "contract", "commit": COMMIT}));
    assert_eq!(validate(&good), Ok(()));
    assert_eq!(lookup_on(&good, "linux").unwrap(), action("contract"));
    assert!(lookup_on(&good, "macos").is_err());
    for bad in [
        json!({"block": "../etc", "commit": COMMIT}),
        json!({"block": "contract", "commit": "abc"}),
        json!({"block": "contract", "commit": COMMIT.to_uppercase()}),
        json!({"block": "contract", "commit": COMMIT, "path": "/x"}),
        json!({"block": "contract", "commit": COMMIT, "service": "a b; rm"}),
        json!({"block": "contract"}),
    ] {
        assert!(validate(&doc(&bad)).is_err(), "{bad}");
    }
}
