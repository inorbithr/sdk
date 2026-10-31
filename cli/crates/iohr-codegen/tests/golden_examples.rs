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

/// What every program of a language must hold, in the golden file (which the test above
/// holds to the renderer): the client closed where the runtime has a close, Ctrl-C
/// handled where the language needs it, and an API error reported on standard error
/// with a non-zero exit status.
const PROGRAM: &[(&str, &[&str])] = &[
    (
        "go",
        &[
            "signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)",
            "defer stop()",
            "defer api.Close()",
            "errors.Is(err, context.Canceled)",
            "log.Fatalf(",
        ],
    ),
    (
        "python",
        &["with Public.from_env() as api:", "sys.exit(f\""],
    ),
    ("typescript", &["console.error(", "process.exitCode = 1;"]),
    ("java", &["System.err.println(", "System.exit(1);"]),
    (
        "csharp",
        &[
            "using var api = ",
            "Console.Error.WriteLine(",
            "Environment.ExitCode = 1;",
        ],
    ),
    ("swift", &["FileHandle.standardError.write(", "exit(1)"]),
    (
        "rust",
        &["tokio::signal::ctrl_c()", "eprintln!(", "ExitCode::FAILURE"],
    ),
];

#[test]
fn every_example_closes_its_client_and_exits_non_zero_on_an_api_error() {
    let path = support::golden().join("public/expected/examples.json");
    let doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let mut missing = Vec::new();
    for (id, op) in doc["operations"].as_object().unwrap() {
        for (lang, marks) in PROGRAM {
            let code = op["code"][*lang].as_str().unwrap();
            for mark in *marks {
                if !code.contains(mark) {
                    missing.push(format!("{id} {lang}: {mark}"));
                }
            }
        }
    }
    assert!(
        missing.is_empty(),
        "{} missing:\n{}",
        missing.len(),
        missing.join("\n")
    );
}
