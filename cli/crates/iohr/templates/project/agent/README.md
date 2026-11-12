# @@NAME@@

@@DESCRIPTION@@

An agent loop in @@LANGUAGE_LABEL@@ with the limits a team asks about first, in code you can read
in one sitting. It uses the standard library alone and holds no API key: the model is a scripted
stub, so a run is repeatable, free and offline.

| Limit | What it does |
| --- | --- |
| Budget | `max_steps` 8, `max_tool_calls` 4, `max_tokens` 4000. A run that would pass one stops. |
| Allow-list | A tool the model asks for that is not on the list is refused; its code never runs. |
| Approval | A tool marked as needing approval runs only when the approval callback answers yes. No answer stops the run. |

Every run ends with a named reason: `done`, `budget_steps`, `budget_tool_calls`,
`budget_tokens` or `awaiting_approval`. The log holds tool names and outcomes, never a tool's
arguments or results, because those can be customer data.

## What is here

@@LAYOUT@@
- `.env.example`: the variables the agent reads, names only (none yet).
- `IO.md`, `AGENTS.md`: how to work in this folder, for people and agents.
- `.github/`: CI (test, secret scan, SBOM), the signed and attested release, dependency updates, CODEOWNERS.
- `SECURITY.md`: how to report a vulnerability.
- `inorbit.toml`, `.inorbit/space`: the InOrbit project record and the link to its space.

## Run

```sh
@@RUN@@ "add 2 and 3"
```

The output starts with `[stub: scripted model, no real model was called]`. That label stays
until you replace the model.

## Use a real model

Write a `Model` with one method, `next(history)`, that calls your provider and returns its text or
a tool call and the tokens it used. Sign in the way your provider's subscription allows and keep
the credential in your secret store, never in this folder (the secret scan in CI fails the build
if one lands here). The loop, the budget, the allow-list and the approval step do not change.

## Test

```sh
@@README_TEST@@
```

The test runs a scripted task within budget, checks that each budget stops a run with its own
reason, that a tool outside the allow-list is refused and never runs, that a tool needing
approval does not run without an answer, and that the log holds no tool argument.

## Release

Push a tag `v*`. `.github/workflows/release.yml` builds, writes a CycloneDX SBOM, signs every
file with cosign (keyless) and attaches a build-provenance attestation. Its steps and how to
verify a download are in the workflow's header.

## Licence

@@LICENSE@@
