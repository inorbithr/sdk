//! The generated Java surface compiles against `java/` (the runtime), and a call a profile
//! may not make does not. Runs with `IOHR_TEST_COMPILE=java` after `mvn verify` in
//! `java/` (the java CI job does both): it asks Maven for the runtime's classpath and runs
//! `javac` on a throwaway program.

#![allow(clippy::unwrap_used, clippy::print_stderr, reason = "test helpers")]

mod support;

use std::path::Path;
use std::process::{Command, Output};

use iohr_codegen::{Language, Options};

use support::{compile_enabled, repo};

const FINE: &str = r#"package app;

import hr.inorbit.sdk.Client;
import iohr.AccountsGetUsageParams;
import iohr.AcmeCi;
import iohr.CreateEndpointRequest;
import iohr.Personal;
import iohr.RadarListDigestsParams;
import java.util.List;

/** What any program may write with this surface. */
public final class Fine {
    private Fine() {}

    /** Calls a few operations. */
    public static void run() {
        Personal personal = new Personal(Client.builder().token("t").build());
        personal.me();
        personal.radar().listDigests(RadarListDigestsParams.builder().limit(5).build());
        AcmeCi ci = AcmeCi.fromEnv();
        ci.accounts().getMe();
        ci.accounts().getUsage("acc_1", AccountsGetUsageParams.builder().from("2026-09-01").build());
        ci.accounts().getUsage("acc_1");
        String secret = ci.events()
                .createEndpoint(CreateEndpointRequest.builder()
                        .accountId("acc_1")
                        .url("https://example.com/hook")
                        .eventTypes(List.of("radar.digest.published"))
                        .build())
                .value()
                .secret();
        if (secret == null) {
            throw new IllegalStateException();
        }
        ci.radar().listDigestsAsync().join();
    }
}
"#;

/// The personal cut has no usage:read, so it has no `accounts` handle at all.
const WRONG: &str = r#"package app;

import iohr.Personal;

/** A call the profile may not make. */
public final class Wrong {
    private Wrong() {}

    /** Must not compile. */
    public static void run(Personal personal) {
        personal.accounts().getUsage("acc_1");
    }
}
"#;

fn javac(root: &Path, classpath: &str, sources: &[&Path]) -> Output {
    let out = root.join("classes");
    Command::new("javac")
        .args(["--release", "17", "-Xlint:all", "-Werror", "-d"])
        .arg(&out)
        .args(["-cp", classpath])
        .args(sources)
        .current_dir(root)
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
fn the_two_profile_surface_compiles_and_the_wrong_profile_does_not() {
    if !compile_enabled(Language::Java) {
        eprintln!("compile test: set IOHR_TEST_COMPILE=java to run it (CI does); skipping");
        return;
    }
    let project = repo().join("java").canonicalize().unwrap();
    let classes = project.join("target/classes");
    assert!(
        classes.join("hr/inorbit/sdk/Client.class").is_file(),
        "no {}: run `mise run java:check` (mvn verify) first",
        classes.display()
    );
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let deps = root.join("deps.txt");
    let mvn = Command::new("mvn")
        .args(["-B", "-q", "-f"])
        .arg(project.join("pom.xml"))
        .args(["dependency:build-classpath", "-Dmdep.includeScope=compile"])
        .arg(format!("-Dmdep.outputFile={}", deps.display()))
        .output()
        .unwrap();
    assert!(
        mvn.status.success(),
        "mvn could not list the runtime's classpath:\n{}",
        text(&mvn)
    );
    let classpath = format!(
        "{}{}{}",
        classes.display(),
        if cfg!(windows) { ";" } else { ":" },
        std::fs::read_to_string(&deps).unwrap().trim()
    );
    let (_, files) = support::render("two-profiles", Language::Java, &Options::default());
    let surface = root.join("src/iohr");
    files.write(&surface, true).unwrap();
    let app = root.join("src/app");
    std::fs::create_dir_all(&app).unwrap();
    std::fs::write(app.join("Fine.java"), FINE).unwrap();
    std::fs::write(app.join("Wrong.java"), WRONG).unwrap();
    let mut sources: Vec<std::path::PathBuf> = std::fs::read_dir(&surface)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    sources.sort();
    let fine = app.join("Fine.java");
    let mut with_fine: Vec<&Path> = sources.iter().map(std::path::PathBuf::as_path).collect();
    with_fine.push(&fine);
    let ok = javac(root, &classpath, &with_fine);
    assert!(
        ok.status.success(),
        "javac failed on the generated surface:\n{}",
        text(&ok)
    );
    let wrong = app.join("Wrong.java");
    let mut with_wrong: Vec<&Path> = sources.iter().map(std::path::PathBuf::as_path).collect();
    with_wrong.push(&wrong);
    let refused = javac(root, &classpath, &with_wrong);
    let message = text(&refused);
    assert!(
        !refused.status.success() && message.contains("cannot find symbol"),
        "the personal profile's getUsage call must not compile:\n{message}"
    );
}
