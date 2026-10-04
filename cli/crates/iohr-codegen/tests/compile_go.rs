//! The generated Go surface builds against `go/` (the runtime), is gofmt-clean, and a
//! call a profile may not make does not build. Runs with `IOHR_TEST_COMPILE=go` and a
//! Go toolchain (the go CI job has both): it writes a throwaway module that replaces
//! the runtime with this repository's `go/`, offline.

#![allow(clippy::unwrap_used, clippy::print_stderr, reason = "test helpers")]

mod support;

use std::path::Path;
use std::process::{Command, Output};

use iohr_codegen::{Language, Options};

use support::{compile_enabled, repo};

fn go(dir: &Path, args: &[&str]) -> Output {
    Command::new("go")
        .args(args)
        .current_dir(dir)
        // Offline and reproducible: the runtime is replaced by a path, nothing is fetched.
        .env("GOFLAGS", "-mod=mod")
        .env("GOWORK", "off")
        .env("GOPROXY", "off")
        .env("GOTOOLCHAIN", "local")
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

#[test]
fn the_two_profile_surface_builds_and_the_wrong_profile_does_not() {
    if !compile_enabled(Language::Go) {
        eprintln!("compile test: set IOHR_TEST_COMPILE=go to run it (CI does); skipping");
        return;
    }
    let runtime = repo().join("go").canonicalize().unwrap();
    let options = Options {
        package: Some("example.com/app/iohr".into()),
        ..Options::default()
    };
    let (_, files) = support::render("two-profiles", Language::Go, &options);
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    files.write(&root.join("iohr"), true).unwrap();
    std::fs::write(
        root.join("go.mod"),
        format!(
            "module example.com/app\n\ngo 1.26\n\nrequire github.com/inorbithr/sdk/go v0.0.0\n\nreplace github.com/inorbithr/sdk/go => {}\n",
            runtime.display()
        ),
    )
    .unwrap();
    std::fs::create_dir_all(root.join("fine")).unwrap();
    std::fs::write(
        root.join("fine/main.go"),
        r#"// Command fine is what any program may write with this surface.
package main

import (
	"context"

	"example.com/app/iohr/acmeci"
	"example.com/app/iohr/models"
	"example.com/app/iohr/personal"
	inorbit "github.com/inorbithr/sdk/go"
)

func main() {
	ctx := context.Background()
	c, err := inorbit.NewClient(inorbit.WithToken("t"))
	if err != nil {
		panic(err)
	}
	p := personal.New(c)
	_, _ = p.Me(ctx)
	limit := int32(5)
	_, _ = p.Radar().ListDigests(ctx, &models.RadarListDigestsParams{Limit: &limit})
	ci := acmeci.New(c)
	_, _ = ci.Accounts().GetMe(ctx)
	from := "2026-09-01"
	_, _ = ci.Accounts().GetUsage(ctx, "acc_1", &models.AccountsGetUsageParams{From: &from})
	r, err := ci.Events().CreateEndpoint(ctx, models.CreateEndpointRequest{URL: inorbit.Ptr("https://example.com/hook")})
	if err == nil {
		_ = r.Value.Secret
	}
}
"#,
    )
    .unwrap();
    let vet = go(root, &["vet", "./..."]);
    assert!(
        vet.status.success(),
        "go vet failed on the generated surface:\n{}",
        text(&vet)
    );
    let fmt = Command::new("gofmt")
        .args(["-l", "iohr"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        fmt.status.success() && fmt.stdout.is_empty(),
        "the generated surface is not gofmt-clean:\n{}",
        text(&fmt)
    );
    std::fs::create_dir_all(root.join("wrong")).unwrap();
    std::fs::write(
        root.join("wrong/main.go"),
        r#"// Command wrong calls what the personal profile's cut does not hold.
package main

import (
	"context"

	"example.com/app/iohr/personal"
	inorbit "github.com/inorbithr/sdk/go"
)

func main() {
	c, _ := inorbit.NewClient(inorbit.WithToken("t"))
	_, _ = personal.New(c).Accounts().GetUsage(context.Background(), "acc_1", nil)
}
"#,
    )
    .unwrap();
    // The personal cut holds no accounts operation, so the handle itself is missing.
    let wrong = go(root, &["vet", "./wrong"]);
    let out = text(&wrong);
    assert!(
        !wrong.status.success() && out.contains("has no field or method"),
        "the personal profile's accounts call must not build:\n{out}"
    );
}
