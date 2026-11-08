# Security policy

This policy covers everything in this repository: the InOrbit SDKs and the `iohr`
command line, including its runner (`iohr runner`). For how they are built to be safe,
see [docs/security/](docs/security/).

## Reporting a vulnerability

Report privately through GitHub:
[Security > Report a vulnerability](https://github.com/inorbithr/sdk/security/advisories/new).
Do not open a public issue, pull request or discussion about a vulnerability.

Include, as far as you can:

- the package, version and runtime;
- what an attacker can do, and under which conditions;
- the smallest reproduction you have;
- whether you know of it being exploited.

Vulnerabilities in the InOrbit API itself (`api.inorbit.hr`, `auth.inorbit.hr`) go
through the same form; we route them.

## What happens next

| Step | Target |
|---|---|
| Acknowledge your report | within 2 business days |
| Triage: confirm, rate severity (CVSS v4), agree a timeline with you | within 5 business days |
| Fix released, Critical | within 7 days of triage |
| Fix released, High | within 30 days |
| Fix released, Medium | within 90 days |
| Fix released, Low | in the next regular release |

The targets are for a confirmed vulnerability in a supported version
([SUPPORT.md](SUPPORT.md)). If a target cannot be met, we tell you why and what the new
date is.

## Coordinated disclosure

- We ask for up to 90 days from your report to publish a fix before details become
  public. We publish sooner when a fix is out, and earlier still if the issue is being
  exploited and users need to act.
- We publish a GitHub Security Advisory for every confirmed vulnerability and request a
  CVE through GitHub, which is a CVE Numbering Authority. Advisories flow into the
  GitHub Advisory Database and OSV, where dependency scanners pick them up.
- We credit you in the advisory unless you ask us not to.

## Safe harbour

We will not pursue legal action against research that:

- stays within this policy and reports privately;
- avoids privacy violations, data destruction and service disruption;
- uses only accounts and API keys you own, and never another customer's data;
- stops and reports as soon as you reach data that is not yours.

## EU Cyber Resilience Act

InOrbit d.o.o. treats itself as the manufacturer of these SDKs under Regulation (EU)
2024/2847. For an actively exploited vulnerability or a severe incident affecting the
security of an SDK, InOrbit reports through ENISA's Single Reporting Platform within the
Article 14 deadlines: an early warning within 24 hours of becoming aware, a notification
within 72 hours, and a final report within 14 days after a fix is available (one month
for a severe incident). Affected users are informed of the issue and of the measures
they can take, through the advisory and the release notes.

## Supported versions

See [SUPPORT.md](SUPPORT.md). Before 1.0, only the latest release of each package
receives fixes.

## What we do on our side

- Releases are published from CI with OIDC trusted publishing; no long-lived registry
  tokens exist. npm and PyPI carry provenance attestations; npm versions wait for a
  maintainer's approval with two-factor authentication before they go live.
- Every GitHub Action is pinned to a full commit SHA and must be on an allowlist;
  workflows are checked by zizmor and CodeQL on every change.
- No telemetry (see [What leaves your machine](#what-leaves-your-machine)). The SDKs never
  log request or response bodies by default, and never print key secrets or access
  tokens; each language tests this
  ([docs/security/requirements.md](docs/security/requirements.md)).
- How to check that a package you installed is the one we built:
  [docs/security/verifying-releases.md](docs/security/verifying-releases.md).

## What leaves your machine

This is the one place the rule is written down; the other documents point here.

Nothing in this repository sends telemetry: no usage pings, no crash reports, no
analytics. The SDKs send only the requests your code makes, to the API and its token
endpoint. `iohr` does the same, with three disclosed exceptions:

- `iohr ext install`, `upgrade` and `sync` contact the extension registry, and only on
  those commands (SR-25 in
  [docs/security/requirements.md](docs/security/requirements.md));
- a running `iohr runner` sends a heartbeat to the API (below);
- a running `iohr runner` checks for and installs new `iohr` releases, unless you turn
  that off (below).

## The runner

`iohr runner run` turns a machine you own into a worker for InOrbit's coordinator. It runs
as you, on your machine, with your own `claude` sign-in. It is supported on macOS and
Linux. **It is not supported on Windows yet**: `iohr runner run` refuses to start there.

### Trust model

- The coordinator schedules turns: it offers this runner a job lease, and the runner
  carries the turn out with `claude`. The runner asks for work only as the runner it
  registered, in the account it registered in, and signs every turn report with its key.
- The lease names the tools a turn may use. A local ceiling, set on the machine, that a
  lease cannot widen is being added and is not in a release yet; until it ships, run the
  runner under an OS user that holds only what its turns need.
- The runner's Ed25519 key, and the key that pins approvals for host actions, are kept
  in the credential store your profile uses (the macOS Keychain on a Mac), never in a
  plain file. Host actions run only from a fixed catalogue, after an approved and signed
  decision (SR-34). GitHub access is a short-lived token per lease, held in memory only
  (SR-33).

### What it sends

- On `iohr runner register`: the runner's name (by default this machine's host name; set
  your own with `--name`), OS, CPU architecture, `iohr` version, the job kinds and
  capacity you chose, whether it is always on, which providers are signed in, a salted
  SHA-256 of the provider identity (never the identity itself), the person it is held
  for if you name one, and its public key with a signed challenge.
- On every heartbeat (every 20 seconds by default, `--interval` changes it): its id,
  version, capacity, kinds and provider sign-in state, and the machine's CPU core count,
  memory and disk size, the 1-minute load average, free and total space per mounted disk
  (at most 64, each with its mount point), and on Linux the pressure (PSI) for CPU,
  memory and I/O. Nothing else about the machine: no process list, no environment.
- Turn reports: the result of each turn the coordinator gave it.

### The hosts it reaches

- `api.inorbit.hr` (or the API base your profile sets), for leases, heartbeats and reports;
- `auth.inorbit.hr`, to refresh your sign-in;
- `github.com` and `api.github.com`, for the repository a lease names (with the lease's
  token) and for release checks;
- Anthropic, only through your own `claude` command line and its own sign-in; the runner
  never calls a model itself.

`console.inorbit.hr` is where a person approves a request in the browser; the runner does
not connect to it.

### Automatic upgrades

A runner keeps itself on the latest `iohr` release; this is on by default
(`auto_upgrade = true`). The first check is about a minute after start, then every
`upgrade_interval_minutes` (60 by default, 5 to 10080). Before installing, it lets
running turns finish. A new release is installed only if every check passes
(`cli/crates/iohr/src/commands/upgrade.rs`, `cli/crates/iohr/src/ext/trust.rs`):

- the archive's SHA-256 matches the release's `SHA256SUMS`;
- its Sigstore bundle was signed by this repository's release workflow
  (`inorbithr/sdk`, `.github/workflows/release.yml`, a branch ref, issuer
  `https://token.actions.githubusercontent.com`);
- its SLSA provenance names this repository's `main` branch.

A Homebrew install upgrades through `brew`; an APT install is left to your package
manager. To turn it off, set `auto_upgrade = false` under `[runner]` (or
`[profiles.<name>.runner]`) in your configuration, or start with
`iohr runner run --no-auto-upgrade`. A runner started by `iohr session` never upgrades
itself. See the runner tables in [docs/config.md](docs/config.md).
