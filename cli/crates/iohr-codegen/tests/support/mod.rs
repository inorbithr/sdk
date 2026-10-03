//! What every language's golden and compile tests share: loading a case's documents,
//! rendering it, comparing with `tests/golden/<case>/expected/<lang>/`, and the switch
//! that turns on a language's compile test.
//!
//! A new language adds `tests/golden_<lang>.rs` calling [`check`] for each case (plus
//! its own assertions) and `tests/compile_<lang>.rs` gated by [`compile_enabled`].

#![allow(
    clippy::unwrap_used,
    dead_code,
    reason = "test helpers; each test file uses a subset"
)]

use std::path::{Path, PathBuf};

use iohr_codegen::{Files, Language, Options};
use iohr_openapi::Api;

/// The cases: `public`, `two-profiles`, `write-scopes`.
pub(crate) const CASES: [&str; 3] = ["public", "two-profiles", "write-scopes"];

/// `tests/golden`.
#[must_use]
pub(crate) fn golden() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

/// The repository's root, for a compile test that needs a runtime by path.
#[must_use]
pub(crate) fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

/// A case's documents, one per `input/<profile>.json`, as one API.
#[must_use]
pub(crate) fn load(case: &str) -> Api {
    let dir = golden().join(case).join("input");
    let mut entries: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    entries.sort();
    let docs = entries.into_iter().map(|path| {
        let profile = path.file_stem().unwrap().to_str().unwrap().to_owned();
        let doc = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        (profile, doc)
    });
    Api::from_documents(docs).unwrap()
}

/// Renders a case as `lang` with `options`.
#[must_use]
pub(crate) fn render(case: &str, lang: Language, options: &Options) -> (Api, Files) {
    let api = load(case);
    let files = iohr_codegen::render(lang, &api, options).unwrap();
    (api, files)
}

/// Compares a case's rendering with `expected/<lang>/`; `IOHR_UPDATE_GOLDEN=1`
/// rewrites the expected files instead (review the diff).
pub(crate) fn check(case: &str, lang: Language, options: &Options) {
    let (_, files) = render(case, lang, options);
    let expected = golden().join(case).join("expected").join(lang.as_str());
    if std::env::var_os("IOHR_UPDATE_GOLDEN").is_some() {
        files.write(&expected, true).unwrap();
        return;
    }
    let diff = files.diff(&expected).unwrap();
    assert!(
        diff.is_empty(),
        "{case} ({lang}): the rendered surface differs from tests/golden/{case}/expected/{lang} ({diff:?}); run with IOHR_UPDATE_GOLDEN=1 and review the diff"
    );
}

/// One rendered file as text.
#[must_use]
pub(crate) fn text(files: &Files, name: &str) -> String {
    let bytes = files
        .iter()
        .find(|(p, _)| p.ends_with(name))
        .unwrap_or_else(|| panic!("no {name} among the rendered files"))
        .1;
    String::from_utf8(bytes.to_vec()).unwrap()
}

/// Whether `lang`'s compile test runs: `IOHR_TEST_COMPILE` is `all`, `1`, or a
/// comma-separated list naming the language. A compile test needs the language's
/// toolchain, which the language's CI job has.
#[must_use]
pub(crate) fn compile_enabled(lang: Language) -> bool {
    std::env::var("IOHR_TEST_COMPILE").is_ok_and(|v| {
        v == "1" && lang == Language::Rust
            || v == "all"
            || v.split(',').any(|l| l.trim() == lang.as_str())
    })
}
