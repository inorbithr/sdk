//! The generated TypeScript surface type-checks against `typescript/` (the runtime), and
//! a call a profile may not make is a type error. Runs with
//! `IOHR_TEST_COMPILE=typescript` after `pnpm install` in `typescript/` (the ts CI job
//! does both): it writes a throwaway project and runs that package's `tsc`.

#![allow(clippy::unwrap_used, clippy::print_stderr, reason = "test helpers")]

mod support;

use std::process::Command;

use iohr_codegen::{Language, Options};

use support::{compile_enabled, repo};

#[test]
fn the_two_profile_surface_type_checks_and_the_wrong_profile_does_not() {
    if !compile_enabled(Language::TypeScript) {
        eprintln!("compile test: set IOHR_TEST_COMPILE=typescript to run it (CI does); skipping");
        return;
    }
    let package = repo().join("typescript").canonicalize().unwrap();
    let tsc = package.join("node_modules/.bin/tsc");
    assert!(
        tsc.is_file(),
        "no {}: run `pnpm install` in typescript/ first",
        tsc.display()
    );
    let (_, files) = support::render("two-profiles", Language::TypeScript, &Options::default());
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    files.write(&root.join("iohr"), true).unwrap();
    std::fs::write(
        root.join("tsconfig.json"),
        format!(
            r#"{{
  "compilerOptions": {{
    "target": "ES2023", "lib": ["ES2023", "DOM"], "module": "NodeNext", "moduleResolution": "NodeNext",
    "strict": true, "noUncheckedIndexedAccess": true, "exactOptionalPropertyTypes": true,
    "verbatimModuleSyntax": true, "noEmit": true, "skipLibCheck": true, "types": [],
    "paths": {{ "@inorbithr/sdk": [{:?}] }}
  }},
  "include": ["*.ts", "iohr/*.ts"]
}}"#,
            package.join("src/runtime.ts").display().to_string()
        ),
    )
    .unwrap();
    std::fs::write(
        root.join("package.json"),
        r#"{ "type": "module", "private": true }"#,
    )
    .unwrap();
    std::fs::write(
        root.join("program.ts"),
        r#"import { Client } from "@inorbithr/sdk";
import { AcmeCi, type CreateEndpointRequest, Personal } from "./iohr/index.js";

/** What any program may write with this surface. */
export async function fine(): Promise<void> {
  const personal = new Personal(new Client({ token: "t" }));
  await personal.me();
  await personal.radar.listDigests({ limit: 5 });
  const ci = AcmeCi.fromEnv();
  await ci.accounts.getMe();
  await ci.accounts.getUsage("acc_1", { from: "2026-09-01" });
}

/** A write, with the body the caller built. */
export async function create(ci: AcmeCi, body: CreateEndpointRequest): Promise<void> {
  const { value } = await ci.events.createEndpoint(body);
  void value.secret;
}

/** A stream: the events, typed, in a for-await loop. */
export async function stream(ci: AcmeCi, signal: AbortSignal): Promise<void> {
  for await (const event of ci.events.streamEvents({ types: "key.created" }, { signal })) {
    void event.type;
  }
}

/** The personal cut has no usage:read: the call must be a type error. */
export async function wrong(personal: Personal): Promise<void> {
  // @ts-expect-error the personal profile may not call getUsage
  await personal.accounts.getUsage("acc_1");
  // @ts-expect-error the personal profile holds no events:read, so no stream
  personal.events.streamEvents();
}
"#,
    )
    .unwrap();
    let output = Command::new(&tsc)
        .args(["-p", "tsconfig.json"])
        .current_dir(root)
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    // A call the profile could make would leave the @ts-expect-error unused, which tsc
    // reports as an error, so success proves both halves.
    assert!(
        output.status.success(),
        "tsc failed on the generated surface:\n{text}"
    );
}
