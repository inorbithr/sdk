# @@NAME@@

@@DESCRIPTION@@

An offline eval suite with a pass bar, in @@LANGUAGE_LABEL@@ on its standard library alone. A
fixed dataset, a solver, deterministic scorers and a bar in `thresholds.toml`. It needs no
network and no key, so it runs the same on your laptop and in CI, and a pull request that
drops a result below the bar fails.

The solver here is a labelled **stub** (keyword rules, `stub: true` in the report), not a
model. That is what makes the suite free and repeatable; swapping in a real model is in
[docs/how-to-grade.md](docs/how-to-grade.md), with how to write cases, scorers and a bar you
can defend.

## What is here

@@LAYOUT@@
- `thresholds.toml`: the pass bar.
- `datasets/tickets.jsonl`: twelve labelled cases.
- `docs/how-to-grade.md`: how to grade, and how to change the suite without fooling yourself.
- `.env.example`: the variables the suite reads, names only (none yet).
- `IO.md`, `AGENTS.md`: how to work in this folder, for people and agents.
- `.github/`: the evals workflow (the offline suite on every pull request), CI (test, secret scan, SBOM), the signed and attested release, dependency updates, CODEOWNERS.
- `SECURITY.md`: how to report a vulnerability.
- `inorbit.toml`, `.inorbit/space`: the InOrbit project record and the link to its space.

## Run

```sh
@@RUN@@
```

Add `--json` for the report as data, `--show` to see each failed case in your terminal. Exit
code 0 means the bar is met, 1 that it is missed, 2 that the dataset or the bar is wrong.

## Test

```sh
@@README_TEST@@
```

The test grades the fixture @@OFFLINE_NOTE@@ and expects the bar to be met; then
it lowers one expected answer and expects the suite to fail and name the case. It also checks
that a chatty answer fails the format scorer, that a solver that raises is an error and not a
pass, and that a broken dataset or a scorer with no bar stops the run.

## Release

Push a tag `v*`. `.github/workflows/release.yml` builds, writes a CycloneDX SBOM, signs every
file with cosign (keyless) and attaches a build-provenance attestation. Its steps and how to
verify a download are in the workflow's header.

## Licence

@@LICENSE@@
