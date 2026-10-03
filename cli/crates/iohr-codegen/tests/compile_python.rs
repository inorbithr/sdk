//! The generated Python surface type-checks against `python/` (the runtime) under pyright
//! and mypy in strict mode, and a call a profile may not make is an error. Runs with
//! `IOHR_TEST_COMPILE=python` after `uv sync` in `python/` (the py CI job does both): it
//! writes a throwaway project and runs that environment's checkers on it.

#![allow(clippy::unwrap_used, clippy::print_stderr, reason = "test helpers")]

mod support;

use std::path::Path;
use std::process::{Command, Output};

use iohr_codegen::{Language, Options};

use support::{compile_enabled, repo};

const FINE: &str = r#""""What any program may write with this surface."""

from inorbithr import Client

from iohr import AcmeCi, AsyncPersonal, CreateEndpointRequest, Personal


def fine() -> None:
    """Calls each profile may make."""
    personal = Personal(Client(token="t"))
    personal.me()
    personal.radar.list_digests(limit=5)
    ci = AcmeCi.from_env()
    ci.accounts.get_me()
    usage = ci.accounts.get_usage("acc_1", from_="2026-09-01").value
    print(usage)


def create(ci: AcmeCi, body: CreateEndpointRequest) -> None:
    """A write, with the body the caller built."""
    created = ci.events.create_endpoint(body).value
    print(created.secret)


async def later(personal: AsyncPersonal) -> None:
    """The asyncio form awaits."""
    me = (await personal.me()).value
    print(me)
"#;

const WRONG: &str = r#""""The personal cut has no usage:read: the call must be a type error."""

from iohr import Personal


def wrong(personal: Personal) -> None:
    """A call the profile may not make."""
    personal.accounts.get_usage("acc_1")
"#;

fn run(tool: &Path, args: &[&str], dir: &Path) -> Output {
    Command::new(tool)
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap()
}

fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn the_two_profile_surface_type_checks_and_the_wrong_profile_does_not() {
    if !compile_enabled(Language::Python) {
        eprintln!("compile test: set IOHR_TEST_COMPILE=python to run it (CI does); skipping");
        return;
    }
    let package = repo().join("python").canonicalize().unwrap();
    let bin = package.join(if cfg!(windows) {
        ".venv/Scripts"
    } else {
        ".venv/bin"
    });
    let (pyright, mypy) = (bin.join("pyright"), bin.join("mypy"));
    assert!(
        pyright.exists() && mypy.exists(),
        "no checkers in {}: run `uv sync` in python/ first",
        bin.display()
    );
    let (_, files) = support::render("two-profiles", Language::Python, &Options::default());
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    files.write(&root.join("iohr"), true).unwrap();
    std::fs::write(
        root.join("pyrightconfig.json"),
        format!(
            r#"{{ "typeCheckingMode": "strict", "pythonVersion": "3.11", "venvPath": {:?}, "venv": ".venv" }}"#,
            package.display().to_string()
        ),
    )
    .unwrap();
    let mypy_args = ["--strict", "--python-version", "3.11"];

    std::fs::write(root.join("fine.py"), FINE).unwrap();
    let out = run(&pyright, &["fine.py", "iohr"], root);
    assert!(
        out.status.success(),
        "pyright failed on the generated surface:\n{}",
        text(&out)
    );
    let out = run(
        &mypy,
        &[&mypy_args[..], &["fine.py", "iohr"]].concat(),
        root,
    );
    assert!(
        out.status.success(),
        "mypy failed on the generated surface:\n{}",
        text(&out)
    );

    std::fs::remove_file(root.join("fine.py")).unwrap();
    std::fs::write(root.join("wrong.py"), WRONG).unwrap();
    let out = run(&pyright, &["wrong.py"], root);
    let said = text(&out);
    assert!(
        !out.status.success() && said.contains("wrong.py:8"),
        "pyright accepted a call the personal profile may not make:\n{said}"
    );
    let out = run(&mypy, &[&mypy_args[..], &["wrong.py"]].concat(), root);
    let said = text(&out);
    assert!(
        !out.status.success() && said.contains("wrong.py:8"),
        "mypy accepted a call the personal profile may not make:\n{said}"
    );
}
