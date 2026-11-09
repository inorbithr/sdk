# Extensions

How `iohr` finds, installs, verifies, pins and runs extensions, how an extension gets its
tokens, and how to build one of your own. Back to the [iohr overview](../README.md).

An extension is a separate program that `iohr` installs from an OCI registry, verifies,
pins and runs. The first is the InOrbit agent:

```sh
iohr ext search agent                    # the catalogue: what you may install
iohr ext show                            # no name: the whole catalogue, coming ones marked
iohr ext show inorbit/agent              # publisher, signer, scopes, privileges, versions
iohr ext install inorbit/agent           # or inorbit/agent@0.1.0, or the short form: agent
iohr agent status                        # runs the installed program
```

Why extensions work this way: [ADR 0012](../../docs/adr/0012-extensions.md). How to check
one by hand: [verifying extensions](../../docs/security/verifying-extensions.md).

## The commands

| Command | What it does |
|---|---|
| `iohr ext search [QUERY] [--kind K] [--all \| --page-size N]` | The catalogue: name, kind, publisher, latest version and visibility |
| `iohr ext show [PUBLISHER/NAME]` | With no name, the same catalogue with listings not yet released marked coming; with a name, the description, publisher, signer, scopes, privileges in plain words, evidence, versions and the install line |
| `iohr ext install NAME\|PUBLISHER/NAME [--lock FILE] [--no-service] [--yes]` | Install one, from the registry (`NAME`) or through the catalogue (`PUBLISHER/NAME`) |
| `iohr ext service NAME` | Set up the system service of an extension that has one, later |
| `iohr ext list`, `verify`, `upgrade`, `remove`, `sync` | What is installed and its privileges; check it; upgrade it; remove it; install exactly what a lock file names |
| `iohr ext packages` | Every package you have in one table: published by your account (public or private), private ones shared with it, unpublished ones in progress on this machine, and what is installed |
| `iohr ext init`, `dev`, `lint` | Build one ([below](#building-one)) |
| `iohr <name> ...` | Run an installed extension |

## The catalogue

`iohr ext search` and `iohr ext show` read the platform's extensions catalogue over the API
host, with the `extensions:read` scope or as a signed-in person. They never contact the
registry. A private listing the account may not see is "not found", like one that does not
exist.

`iohr ext install PUBLISHER/NAME` asks the catalogue which version to install (the newest
release, or `@VERSION`), its digest and its signer, then fetches that digest from the
registry and runs every check in the next section. The artifact must then be the one
listed: the same version, signed by exactly the signer the catalogue names, which must be
the listing's signing identity, with the same scopes and privileges. Any difference fails
the install and nothing is written. So does an installed extension of the same name from
another signer (`iohr ext remove` it first). The catalogue never replaces the trust root:
an artifact the root refuses is refused whatever the catalogue says.

The short form (`iohr ext install agent`), `upgrade` and `sync` read only the registry.
Another publisher's extension is not in InOrbit's registry, so it needs `ext.registry` set
to the mirror it is published in ([a mirror](#a-mirror)).

## What is checked, and what is pinned

Before anything is used, `iohr` checks every digest, a signature and SLSA provenance from
InOrbit's release workflow (Sigstore, offline, against the root built into `iohr`), and the
manifest's rules. No flag skips this. Install prints who signed it and the API scopes it
may ask for.

What is installed is pinned in `iohr-ext.lock`. With `--lock iohr-ext.lock` the same file
goes into your repository, and `iohr ext sync` installs exactly that elsewhere. Nothing
updates by itself: `iohr ext upgrade` does, when you run it.

`iohr ext verify` checks what is installed.
`iohr ext verify ARTIFACT|sha256:DIGEST --bundle FILE --publisher-key PEM` checks one
artifact that is not, with the install's trust check, offline and without a sign-in. It
prints the signer and exits 1 when it is not trusted (`--json`: `ok`, `digest`, `signer`,
`provenance`, `error`).

## Privileges

An extension that controls a system service, such as eBPF traffic capture, declares the
Linux capabilities that service holds in its manifest (`privileges`). `iohr` grants none of
them: it runs as you, and the service gets them from its own package or unit. Install shows
each one in plain words and asks you to type `yes`. In a pipeline, pass `--yes`; without a
terminal and without `--yes`, nothing is installed.

```text
capture 0.1.0 declares privileges: its system service holds these Linux capabilities.
  CAP_BPF        load eBPF programs into the kernel
  CAP_PERFMON    observe performance and kernel state (perf events, tracing)
  CAP_NET_ADMIN  configure the network: interfaces, routes, firewall, traffic control
iohr runs as you and grants none of them; the service gets them from its own package or unit.
Type yes to confirm these privileges and install it:
```

`iohr ext upgrade` names the privileges a new release adds and asks again. `iohr ext list`
and `verify` show them (`--json` too), and `iohr-ext.lock` records what was confirmed:
`iohr ext sync` refuses an extension whose privileges grew beyond its lock entry
([SR-32](../../docs/security/requirements.md)).

An extension with a system service of its own (`inorbit/capture`) offers to set it up with
sudo after installing. It lists the network interfaces it detects and suggests one
(`--interface`, repeatable, or `--all-interfaces`); `--no-service` skips the offer, and
`iohr ext service NAME` does it later.

## Tokens for a running extension

A running extension never sees your refresh token or the credential store. It asks `iohr`
for an access token over a private socket (`IOHR_EXT_TOKEN_SOCKET`, mode 0600), and only
for the scopes its manifest declares; `IOHR_EXT_API` is the API's address.

- **Signed in as a person**, each token is narrowed to the scopes asked for: `iohr` makes
  an account key with exactly those scopes, exchanges it for a 15-minute token and revokes
  the key at once. This needs an admin or owner of the account; anyone else is refused with
  that reason.
- **A key or token profile** hands out its own token when it holds every scope asked for,
  and is refused when it does not.

Getting a token is bounded at 30 seconds; after that the program gets a `503` that says
why.

## The offer in `iohr project init`

`iohr project init` looks at the template and the `--describe` text, finds the catalogue
listings that fit (a rule table in
[`commands/init_extensions.rs`](../crates/iohr/src/commands/init_extensions.rs)) and, at a
terminal, asks whether to install each one (default no) through the same path as
`iohr ext install`. With `--yes`, `--json` or no terminal it prints one
`iohr ext install ...` line and installs nothing.

A listing marked coming is never offered: init says, as a plain line, that it is not
released yet and that there is nothing to install. An extension that is already installed
is not offered. The site crawler is not a listing of its own: it ships inside
`inorbit/agent` as `iohr agent atlas crawl <URL>`, so init offers `inorbit/agent` for it.

## Building one

From an empty folder to a running extension:

```sh
iohr ext init inorbit/hello --lang shell   # or rust, or python
cd hello
iohr ext lint --preview                    # the manifest, checked, in plain words
iohr ext dev                               # run it: live log, token channel, heartbeat
iohr ext dev --test                        # its self-test, same channel and folders
```

`iohr ext init PUBLISHER/NAME --lang rust|python|shell [--dir DIR]` scaffolds a folder: a
manifest with every field, an entry point on the token channel, a self-test, a README and
a signing release workflow. These three commands work offline, except the token channel's
sign-in. The whole loop: [developing extensions](../../docs/extensions/developing.md).

### The manifest and `iohr ext lint`

`extension.toml` is the source manifest: the artifact's config (name, version, entrypoint,
scopes, privileges) plus how it runs, what it may reach, read, write, start and send up,
its resources, health, secrets by reference, targets and upgrade. Unknown keys are
refused.

`iohr ext lint [PATH] [--preview] [--json]` exits 1 on:

- an unknown key;
- a value that looks like a secret;
- a `permissions.data.leaves` kind outside the non-sensitive list (the error lists the
  kinds allowed);
- a `targets.kinds` the kind cannot be installed on;
- a name holding `tbd`.

`--preview` prints what the extension may do as an installer will show it, and `--json`
the same, structured, with the artifact's `config`.

A program may name `cluster` in `targets.kinds` (it needs `delivery = "program"` and a
`linux/...` platform), serve a panel (`[panel]`: `title`, `entry`) and read `$REPOS`, the
repository folders the user adds in its host.

### `iohr ext dev`

`iohr ext dev [PATH] [--test]` lints first, then runs the program with the token channel
for exactly the manifest's scopes. Every request is logged, a refused one in red, and the
output is timestamped.

- `STATE` and `CACHE` are `~/.inorbit/ext-dev/NAME/state` and `cache`.
- `REPOS` is the roots in `~/.inorbit/ext-dev/NAME/config.json` (`{"repos": ["/abs/path"]}`)
  joined with `:`, and empty without that file.
- Heartbeats are marked `"dev": true`.
- Installed extensions and their lock are never read or written.
- The child gets no `IOHR_TOKEN*`, `INORBIT_TOKEN*` or `INORBIT_KEY_*`, and no folder
  holding an `iohr` stays on its `PATH`, so the channel is its only credential.
- A secret `{ name, ref, env }` comes from `IOHR_EXT_SECRET_<NAME>` or the macOS Keychain
  (service `hr.inorbit.iohr.ext`, account the `ref`).

`--test` runs `tests/` (`*.sh`, `python3 -m unittest`, or `cargo test`) the same way, with
a fresh `HOME` and `TMPDIR`, and fails when anything was written outside `STATE` and
`CACHE`.

## A mirror

To install from a company's mirror:

```sh
iohr config set ext.registry registry.acme.hr/inorbit/iohr-ext
iohr config set ext.trusted_keys mirror.pub      # when the mirror re-signs
```

A mirror that needs a login reads `IOHR_EXT_REGISTRY_AUTH=user:password` from the
environment. `iohr config get` and `unset` read and clear the same two keys.
