//! The language-neutral context every target renders from, per case: each operation's
//! retry and idempotency-key marks, byte for byte against
//! `tests/golden/<case>/expected/operations.json`; `IOHR_UPDATE_GOLDEN=1` rewrites it
//! (review the diff). The marks are what a runtime's `retry` and `idempotency_key`
//! middlewares read (`docs/config.md` sections 7.4 and 7.5).

#![allow(clippy::unwrap_used, reason = "test helpers")]

mod support;

use iohr_codegen::context::{Op, surface};

fn marks(case: &str) -> String {
    let s = surface(&support::load(case));
    let ops: Vec<&Op> = s
        .handles
        .iter()
        .flat_map(|h| h.ops.iter())
        .chain(s.flat.iter())
        .collect();
    let mut rows: Vec<serde_json::Value> = ops
        .iter()
        .map(|op| {
            serde_json::json!({
                "line": op.line,
                "operation": op.hook_name,
                "retry_safe": op.retry_safe,
                "idempotency_key": op.idempotency_key,
            })
        })
        .collect();
    rows.sort_by(|a, b| a["line"].as_str().cmp(&b["line"].as_str()));
    let mut text = serde_json::to_string_pretty(&rows).unwrap();
    text.push('\n');
    text
}

#[test]
fn every_operations_marks_match_the_golden_files() {
    for case in support::CASES {
        let text = marks(case);
        let path = support::golden()
            .join(case)
            .join("expected/operations.json");
        if std::env::var_os("IOHR_UPDATE_GOLDEN").is_some() {
            std::fs::write(&path, &text).unwrap();
            continue;
        }
        let expected = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(
            expected == text,
            "{case}: the operations' marks differ from {}; run with IOHR_UPDATE_GOLDEN=1 and review the diff",
            path.display()
        );
    }
}

/// The writes that take `Idempotency-Key` are marked, and only they: a `PATCH` without
/// the header (`events.update_endpoint`) is not, and no read is.
#[test]
fn only_operations_that_declare_the_header_are_marked() {
    let s = surface(&support::load("public"));
    let ops: Vec<&Op> = s.handles.iter().flat_map(|h| h.ops.iter()).collect();
    let find = |name: &str| *ops.iter().find(|o| o.hook_name == name).unwrap();
    assert!(find("events.create_endpoint").idempotency_key);
    assert!(
        !find("events.create_endpoint").retry_safe,
        "a key, not the method, makes it safe"
    );
    assert!(!find("events.update_endpoint").idempotency_key);
    for op in &ops {
        if op.method.is_idempotent() {
            assert!(!op.idempotency_key, "{}", op.line);
        }
    }
    let marked = ops.iter().filter(|o| o.idempotency_key).count();
    assert!(marked > 0);
}
