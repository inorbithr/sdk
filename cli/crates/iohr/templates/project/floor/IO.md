# @@NAME@@: how to work in this project

This file is the one source for people and agents (`AGENTS.md` only points here).

## What it is

A `@@TEMPLATE@@` project from InOrbit's templates, in @@LANGUAGE_LABEL@@.

## Commands

- Test: `@@CI_TEST@@`
- The same test runs in CI (`.github/workflows/ci.yml`); a change is done when it passes there.

## Rules

- No secret, token or customer data in the repository, a log or a test fixture. Variable names
  go in `.env.example`; values stay on your machine or in the CI's secret store.
- Add a dependency only with its lockfile updated in the same change.
- Licence: @@LICENSE@@. Report vulnerabilities as `SECURITY.md` says.
