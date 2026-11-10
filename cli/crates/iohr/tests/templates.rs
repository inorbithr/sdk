//! The template set (PRD 0088.1, ADR 0221): the picker, the enterprise floor every template
//! but `blank` carries, the one question with no default (the licence), the `--ci none`
//! opt-out, and for every live variant: init, then that template's own check passes.
//!
//! The binary runs with a private HOME and config folder. A language whose toolchain is not
//! on the PATH is skipped, never faked.

#![allow(
    clippy::unwrap_used,
    reason = "test helpers fail the test by panicking"
)]

mod support;

use std::path::Path;
use std::process::{Command, Output, Stdio};
use support::kept_env;

fn run(home: &Path, cwd: &Path, args: &[&str]) -> Output {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let real = std::env::var_os("HOME").unwrap_or_default();
    Command::new(env!("CARGO_BIN_EXE_iohr"))
        .args(args)
        .current_dir(cwd)
        .env_clear()
        .envs(kept_env())
        .env("PATH", path)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("IOHR_CONFIG_DIR", home.join("config"))
        // The toolchains keep their own homes under the real HOME.
        .env("CARGO_HOME", Path::new(&real).join(".cargo"))
        .env("RUSTUP_HOME", Path::new(&real).join(".rustup"))
        .env("GOPATH", home.join("go"))
        .env("GOCACHE", home.join("gocache"))
        .env("GOFLAGS", "-modcacherw")
        .env("UV_CACHE_DIR", home.join("uv"))
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

fn text(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

fn on_path(program: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|p| {
        std::env::split_paths(&p)
            .any(|d| d.join(program).is_file() || d.join(format!("{program}.exe")).is_file())
    })
}

/// `iohr project init NAME --template T [--lang L] ...extra`, answered and without a plan,
/// into a fresh folder; the folder and the output come back.
fn init(
    template: &str,
    lang: Option<&str>,
    smoke: bool,
    extra: &[&str],
) -> (tempfile::TempDir, std::path::PathBuf, Output) {
    let h = tempfile::tempdir().unwrap();
    let cwd = tempfile::tempdir().unwrap();
    let name = format!("t-{template}");
    let dir = cwd.path().join(&name);
    let mut args = vec!["project", "init", &name, "--template", template];
    // `blank` has no floor, so no licence question: a --set for it is refused.
    if template != "blank" {
        args.extend(["--set", "license=MIT"]);
    }
    args.extend(["--no-plan", "--yes"]);
    if let Some(l) = lang {
        args.extend(["--lang", l]);
    }
    if !smoke {
        args.push("--no-smoke");
    }
    args.extend(extra);
    let out = run(h.path(), cwd.path(), &args);
    // Keep the working folder alive with the home: leak the TempDir of cwd into the home's.
    std::mem::forget(cwd);
    (h, dir, out)
}

/// Whether any file under `dir` (not `.git`) holds `needle`.
fn holds(dir: &Path, needle: &str) -> bool {
    std::fs::read_dir(dir).unwrap().flatten().any(|e| {
        let p = e.path();
        if p.is_dir() {
            e.file_name() != ".git" && holds(&p, needle)
        } else {
            std::fs::read_to_string(&p).is_ok_and(|t| t.contains(needle))
        }
    })
}

fn file(dir: &Path, rel: &str) -> String {
    std::fs::read_to_string(dir.join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

#[test]
fn the_picker_lists_the_approved_set_service_first_and_all_adds_the_hidden() {
    let h = tempfile::tempdir().unwrap();
    let out = run(h.path(), h.path(), &["project", "templates", "--json"]);
    assert!(out.status.success(), "{}", text(&out));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let rows = v.as_array().unwrap();
    let names: Vec<&str> = rows.iter().map(|r| r["name"].as_str().unwrap()).collect();
    assert_eq!(
        names,
        ["blank", "service", "agent", "cli", "extension"],
        "live templates, in the picker's order; coming and hidden ones are not offered"
    );
    let service = rows.iter().find(|r| r["name"] == "service").unwrap();
    assert_eq!(service["default"], true);

    let out = run(
        h.path(),
        h.path(),
        &["project", "template", "list", "--all", "--json"],
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let all: Vec<&str> = v
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["name"].as_str().unwrap())
        .collect();
    for hidden in ["demo", "coordinated", "mcp-server", "evals"] {
        assert!(all.contains(&hidden), "--all lists {hidden}: {all:?}");
    }
    for floor in ["floor", "base"] {
        assert!(
            !all.contains(&floor),
            "{floor} is a base, not a template: {all:?}"
        );
    }
}

#[test]
fn a_template_that_is_coming_refuses_and_says_so() {
    for coming in ["mcp-server", "evals"] {
        let (_h, dir, out) = init(coming, None, false, &[]);
        assert!(!out.status.success(), "{coming}: {}", text(&out));
        assert!(text(&out).contains("coming"), "{coming}: {}", text(&out));
        assert!(!dir.exists(), "{coming}: nothing is written");
    }
}

#[test]
fn a_missing_licence_is_refused_and_the_question_is_named() {
    let h = tempfile::tempdir().unwrap();
    let cwd = tempfile::tempdir().unwrap();
    let out = run(
        h.path(),
        cwd.path(),
        &[
            "project",
            "init",
            "nolic",
            "--template",
            "service",
            "--lang",
            "go",
            "--no-plan",
            "--no-smoke",
            "--yes",
        ],
    );
    assert!(!out.status.success(), "{}", text(&out));
    assert!(text(&out).contains("license"), "{}", text(&out));
    assert!(!cwd.path().join("nolic").exists());

    let out = run(
        h.path(),
        cwd.path(),
        &[
            "project",
            "init",
            "badlic",
            "--template",
            "service",
            "--lang",
            "go",
            "--no-plan",
            "--no-smoke",
            "--yes",
            "--set",
            "license=free for all",
        ],
    );
    assert!(!out.status.success(), "an SPDX id only: {}", text(&out));
}

/// The floor every template but `blank` writes, on a service in Go (no toolchain needed:
/// the smoke is off).
#[test]
fn the_enterprise_floor_is_written_on_every_template_but_blank() {
    for (template, lang) in [
        ("service", "go"),
        ("service", "python"),
        ("service", "rust"),
        ("cli", "go"),
        ("cli", "rust"),
        ("agent", "python"),
        ("extension", "python"),
        ("extension", "typescript"),
    ] {
        let (_h, dir, out) = init(template, Some(lang), false, &[]);
        assert!(out.status.success(), "{template}/{lang}: {}", text(&out));
        let at = format!("{template}/{lang}");

        let ci = file(&dir, ".github/workflows/ci.yml");
        assert!(ci.contains("permissions:\n  contents: read"), "{at}: {ci}");
        // Every third-party action is pinned to a full commit SHA.
        for line in ci.lines().filter(|l| l.contains("uses:")) {
            let pin = line.split('@').nth(1).unwrap_or_default();
            let sha: String = pin.chars().take_while(char::is_ascii_hexdigit).collect();
            assert_eq!(sha.len(), 40, "{at}: pinned by SHA: {line}");
        }
        assert!(
            ci.to_lowercase().contains("cyclonedx") || ci.contains("sbom"),
            "{at}: SBOM in CI"
        );
        assert!(
            ci.contains("secret") || ci.contains("gitleaks"),
            "{at}: a secret scan in CI"
        );

        let release = file(&dir, ".github/workflows/release.yml");
        assert!(release.contains("cosign"), "{at}: keyless signing");
        assert!(release.contains("attest"), "{at}: provenance");
        assert!(dir.join(".github/dependabot.yml").is_file(), "{at}");
        assert!(dir.join(".github/CODEOWNERS").is_file(), "{at}");

        assert!(file(&dir, "SECURITY.md").contains("Reporting"), "{at}");
        assert!(file(&dir, "AGENTS.md").contains("IO.md"), "{at}");
        assert!(
            holds(&dir, "MIT"),
            "{at}: the asked licence is written into the project"
        );

        // `.env.example` holds names only: no line has a value after `=` that looks like one.
        let env = file(&dir, ".env.example");
        for line in env
            .lines()
            .filter(|l| !l.starts_with('#') && l.contains('='))
        {
            let value = line.split_once('=').unwrap().1.trim();
            assert!(
                value.is_empty(),
                "{at}: .env.example holds a name only: {line}"
            );
        }
        assert!(
            file(&dir, ".gitignore").contains(".env"),
            "{at}: .env is ignored"
        );
        assert!(dir.join("inorbit.toml").is_file(), "{at}");

        // The workflows are valid (a stray quote in a generated step is caught here).
        if on_path("actionlint") {
            let lint = Command::new("actionlint")
                .current_dir(&dir)
                .output()
                .unwrap();
            assert!(lint.status.success(), "{at}: {}", text(&lint));
        }
    }
}

#[test]
fn ci_none_drops_the_ci_files_and_keeps_the_rest_of_the_floor() {
    let (_h, dir, out) = init("service", Some("go"), false, &["--ci", "none"]);
    assert!(out.status.success(), "{}", text(&out));
    for gone in [
        ".github/workflows/ci.yml",
        ".github/workflows/release.yml",
        ".github/dependabot.yml",
    ] {
        assert!(!dir.join(gone).exists(), "{gone}");
    }
    for kept in [
        "SECURITY.md",
        "AGENTS.md",
        ".env.example",
        ".github/CODEOWNERS",
    ] {
        assert!(dir.join(kept).is_file(), "{kept}");
    }
}

#[test]
fn blank_keeps_a_small_floor_and_no_ci_unless_asked() {
    let (_h, dir, out) = init("blank", Some("python"), false, &[]);
    assert!(out.status.success(), "{}", text(&out));
    for kept in [".gitignore", "SECURITY.md", ".github/CODEOWNERS"] {
        assert!(dir.join(kept).is_file(), "{kept}");
    }
    assert!(
        !dir.join(".github/workflows").exists(),
        "no CI on blank by default"
    );
    assert!(!dir.join(".github/dependabot.yml").exists());

    let (_h, dir, out) = init("blank", Some("python"), false, &["--ci", "github"]);
    assert!(out.status.success(), "{}", text(&out));
    assert!(dir.join(".github/workflows/ci.yml").is_file());
}

/// Init, then the template's own check: the smoke test runs inside init (`done:` line), and
/// for the languages whose toolchain is here, the project's own test command passes again
/// when run by hand.
#[test]
fn every_live_variant_inits_and_its_own_check_passes_when_the_toolchain_is_here() {
    let table: &[(&str, &str, &[&str])] = &[
        ("service", "go", &["go"]),
        ("service", "python", &["python3"]),
        ("service", "typescript", &["node"]),
        ("service", "rust", &["cargo"]),
        ("cli", "go", &["go"]),
        ("cli", "python", &["python3"]),
        ("cli", "rust", &["cargo"]),
        ("agent", "python", &["uv"]),
        ("agent", "typescript", &["node"]),
        ("extension", "python", &["python3"]),
        ("extension", "typescript", &["node"]),
        ("extension", "go", &["go"]),
        ("extension", "rust", &["cargo"]),
    ];
    let mut ran = 0;
    for (template, lang, needs) in table {
        if !needs.iter().all(|p| on_path(p)) {
            continue;
        }
        let (_h, dir, out) = init(template, Some(lang), true, &[]);
        assert!(out.status.success(), "{template}/{lang}: {}", text(&out));
        assert!(
            text(&out).contains("done:"),
            "{template}/{lang} smoke: {}",
            text(&out)
        );
        assert!(dir.join("README.md").is_file(), "{template}/{lang}");
        ran += 1;
    }
    eprintln!("variants checked: {ran} of {}", table.len());
}
