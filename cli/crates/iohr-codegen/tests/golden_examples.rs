//! The examples of the `public` case, every language, byte for byte against
//! `tests/golden/public/expected/examples.json`; `IOHR_UPDATE_GOLDEN=1` rewrites it
//! (review the diff). `compile_examples.rs` proves the snippets build.

#![allow(clippy::unwrap_used, reason = "test helpers")]

mod support;

use iohr_codegen::Language;

#[test]
fn the_public_examples_match_the_golden_file() {
    let api = support::load("public");
    let examples = iohr_codegen::examples::render(&api, &Language::ALL, "iohr test").unwrap();
    let text = examples.to_json();
    let path = support::golden().join("public/expected/examples.json");
    if std::env::var_os("IOHR_UPDATE_GOLDEN").is_some() {
        std::fs::write(&path, &text).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        expected == text,
        "the public examples differ from {}; run with IOHR_UPDATE_GOLDEN=1 and review the diff",
        path.display()
    );
    assert_eq!(examples.operations.len(), api.operations.len());
    for op in examples.operations.values() {
        assert_eq!(op.code.len(), Language::ALL.len(), "{}", op.line);
    }
}
