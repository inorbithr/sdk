# @@NAME@@

@@DESCRIPTION@@

A command line in @@LANGUAGE_LABEL@@ with what a tool needs on its first day: text and JSON
output, exit codes a script can rely on, golden-file tests and a version that says which commit
it was built from. It uses the standard library alone, so there is nothing to install.

```sh
@@RUN@@
@@NAME@@ greet Ada --json        # {"message":"Hello, Ada"}
@@NAME@@ version                 # @@NAME@@ 0.1.0 (commit 0123456789ab)
```

| Exit code | Means |
| --- | --- |
| 0 | it worked |
| 1 | it ran and failed (for example a name that is empty or too long) |
| 2 | it was used wrongly (an unknown command, too many arguments); the error is on stderr |

Output a script reads goes to stdout and is stable: `--json` keys are sorted and every output
has a golden file in `golden/`. Errors go to stderr and never echo a hostile argument raw.

## What is here

@@LAYOUT@@
- `golden/`: the expected output of every command, in text and JSON; the tests compare against it.
- `.env.example`: the variables the program reads, names only (none yet).
- `IO.md`, `AGENTS.md`: how to work in this folder, for people and agents.
- `.github/`: CI (test, secret scan, SBOM), the signed and attested release, dependency updates, CODEOWNERS.
- `SECURITY.md`: how to report a vulnerability.
- `inorbit.toml`, `.inorbit/space`: the InOrbit project record and the link to its space.

## Test

```sh
@@README_TEST@@
```

The test compares each command's output with its golden file in text and JSON, checks the exit
codes and checks that `version` carries a commit. Change a golden file only when the output is
meant to change, in the same commit.

## Install

```sh
@@INSTALL@@
```

## Release

Push a tag `v*`. `.github/workflows/release.yml` builds, writes a CycloneDX SBOM, signs every
file with cosign (keyless) and attaches a build-provenance attestation. Its steps and how to
verify a download are in the workflow's header.

## Licence

@@LICENSE@@
