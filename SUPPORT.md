# Support policy

How long each version of the SDKs receives fixes, and which runtimes they run on. The
same policy applies to all four packages; each is versioned on its own
([ADR 0003](docs/adr/0003-versioning-and-releases.md)).

## Versions

The packages follow [Semantic Versioning](https://semver.org).

### Before 1.0

Only the latest release of each package receives fixes, security fixes included. A 0.x
minor release may change the public API; the changelog says how to migrate.

### From 1.0

| Major version | Features and bug fixes | Security fixes |
|---|---|---|
| Latest major | yes | yes |
| Earlier majors | no | yes, for at least 5 years from the major's first release |

- The 5-year security support period follows the support period the EU Cyber Resilience
  Act expects for software products (Regulation (EU) 2024/2847, Article 13(8)).
- Security fixes for an earlier major ship as patch releases of that major's latest
  minor.
- A major reaches end of life only after its support period, and never with less than
  12 months' notice.

### How end of life is announced

- In the release notes of the latest major and of the affected major.
- In this file, with the end-of-life date.
- In the package metadata where the registry supports it (npm `deprecate`, PyPI
  classifiers, a notice in the crate and module documentation).

## Runtimes

Minimums and test matrices are set in
[ADR 0006](docs/adr/0006-names-and-runtimes.md).

| Language | Minimum | Rule |
|---|---|---|
| Go | 1.26 | The two newest Go releases, as Go itself supports. |
| Rust | MSRV 1.94 | Raised at most once every six months, in a minor release. |
| TypeScript | Node 22.12, ESM only; Bun, Deno, browsers | Node releases in active or maintenance LTS. |
| Python | 3.11 | Python versions that have not reached end of life. |

Raising a minimum runtime is a minor release with a changelog entry, never a patch.

## Getting help

- Bugs and feature requests: [GitHub issues](https://github.com/inorbithr/sdk/issues).
- Vulnerabilities: privately, as described in [SECURITY.md](SECURITY.md).
- The API itself: <https://docs.inorbit.hr>.
