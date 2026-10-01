# Governance

How decisions are made in this repository and who makes them.

## Maintainers

| Maintainer | Role | For |
|---|---|---|
| [@0x19](https://github.com/0x19) | Lead maintainer, release approver | InOrbit d.o.o. |

The SDKs are owned by InOrbit d.o.o. (Croatia), which also runs the API they call.

## Decisions

- Design decisions are written as ADRs in [docs/adr/](docs/adr/) and change only by a
  new ADR that supersedes the old one.
- Cross-language rules live in [docs/design.md](docs/design.md); security requirements
  in [docs/security/requirements.md](docs/security/requirements.md).
- Anyone can propose a change through an issue or a pull request. The maintainers
  decide, and say why in the issue or PR.

## How changes are reviewed today

There is one maintainer. A rule that every change needs a second person's approval cannot
be met yet, so the repository relies on controls that do not depend on one person's
attention:

- Every change reaches `main` through a pull request; direct pushes, force pushes and
  branch deletion are blocked by rulesets.
- The required check `ci-ok` must pass: formatting, linting, type checks, tests,
  conformance cases, workflow linting (actionlint, zizmor) and repository invariants.
- CodeQL, dependency review and OpenSSF Scorecard run on changes and on a schedule.
- An AI reviewer (the `claude-review` workflow) reviews each pull request when it opens,
  and the maintainer works through the pull request template's checklist.
- Releases are cut by release-please from merged commits only; only the release app can
  create release tags, and nobody can move or delete one.
- Publishing uses OIDC trusted publishing from CI, behind environments that need the
  maintainer's approval; npm releases also wait for approval with two-factor
  authentication on npmjs.com.
- Build provenance is recorded for released packages
  ([docs/security/verifying-releases.md](docs/security/verifying-releases.md)).

These are compensating controls, not a substitute for a second reviewer. We plan to add
an independent reviewer with approval rights, then require one approval on every pull
request. This file changes when that happens.

## Access continuity

If the lead maintainer is unavailable:

- Owners of the `inorbithr` GitHub organisation can grant access, approve releases and
  change settings. Today the lead maintainer is the only owner; adding a second owner
  with sealed recovery access is planned alongside the second reviewer.
- Releases need no personal credentials: the registries trust this repository's release
  workflow through OIDC. Nothing secret has to be handed over to keep releasing.
- The registry accounts are protected by two-factor authentication. npm (organisation
  `inorbithr`), PyPI and JSR (scope `@inorbithr`) are held for InOrbit; the crates.io
  crate is owned by the lead maintainer's account until a GitHub team is added as a
  co-owner.

## Code of conduct

Participation follows the [Contributor Covenant](CODE_OF_CONDUCT.md).
