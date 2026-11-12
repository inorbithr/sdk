# SDKs and the API

How `iohr` adds the published SDK to your project, shows what an SDK client would use,
generates a surface cut to your account, writes examples for an API reference, and calls
the API directly. Back to the [iohr overview](../README.md).

## Add the SDK to a project

```sh
iohr sdk add                 # the language and package manager from the project's files
iohr sdk add go --version 0.2.1
iohr sdk add --dry-run       # print the command, run nothing
```

The language comes from the nearest project file between the current directory and the
repository root: `Cargo.toml`, `package.json` or `deno.json`, `pyproject.toml`,
`requirements.txt` or a Python lock file, or `go.mod`. When one directory holds several,
name the language (`ts` and `js` stand for `typescript`, `py` for `python`). The package
manager is the one the project already uses:

| Project | Command |
|---|---|
| Rust | `cargo add inorbithr` |
| `pnpm-lock.yaml`, `yarn.lock`, `bun.lock` | `pnpm add`, `yarn add` or `bun add @inorbithr/sdk` |
| `deno.json` without `package.json` | `deno add jsr:@inorbithr/sdk` |
| Other JavaScript and TypeScript | `npm install @inorbithr/sdk` (or the manager the `packageManager` field names) |
| `uv.lock`, `poetry.lock`, `pdm.lock` | `uv add`, `poetry add` or `pdm add inorbithr` |
| Other Python, in an active virtualenv | `<venv>/bin/python -m pip install inorbithr` |
| Go | `go get github.com/inorbithr/sdk/go@latest` |

Lock files are looked for up to the repository root, so a workspace member uses its
workspace's manager. Without a virtualenv and without a uv, poetry or pdm lock, `iohr`
installs nothing into a system Python and prints the uv and venv commands instead.
`--version` installs that release (`inorbithr@0.2.1`, `inorbithr==0.2.1`, `@v0.2.1`);
without it the manager picks the newest and records it as it always does.

Not supported yet: C# and Java. They are not on NuGet and Maven Central yet, so
`iohr sdk add` refuses them with a pointer to the source.

`iohr` prints the command, then runs it as one program with its arguments (no shell, never
sudo) in the project directory. The exit code is the package manager's, or 2 when no
project or manager can be chosen. Only the package manager contacts a registry. After it
succeeds, `iohr` shows a first call in that language and points at `iohr sdk config`. With
`--json`, the manager's output goes to stderr and stdout holds one JSON object: `lang`,
`manager`, `dir`, `command` (the argument vector) and `dry_run`.

## What will my service see?

`iohr sdk config` resolves the SDK configuration the way `load` does in every SDK
([docs/config.md](../../docs/config.md) sections 2 to 5) and prints the same JSON as the
SDKs' `describe()`: the profile and where it was chosen, the config file, every setting
with its value and source, the credential chain with each source tried, the pipeline,
and what was read but ignored (unknown keys, scopes a token does not use).

```sh
INORBIT_TIMEOUT=5s iohr sdk config --profile ci
```

It reads `INORBIT_*`, the standard proxy variables and the config file, contacts no host
and prints no secret: a key secret, a token and a proxy password show as `<redacted>`.

- `--profile NAME` is the client's `profile` option. `IOHR_PROFILE` is the command line's
  switch and is not read here, as the SDKs do not read it.
- `--for TYPE` resolves for a generated profile type instead of the public client
  (`Client<AcmeCi>` is `--for acme-ci`, which reads `INORBIT_ACME_CI_*` and its own table).
- A configuration `load` would refuse exits 1 with every problem on stderr, each with the
  setting and where its value came from.

## An SDK for your account

```sh
iohr login                                   # a person, or `iohr login --with-token --profile ci`
iohr profile account default acme            # a team's slug or id (a person's profile)
iohr sdk generate --lang rust --for default --for ci --out src/iohr
git add src/iohr src/iohr.lock
```

`iohr sdk generate` writes a surface that holds exactly the operations each profile's
credential may call, into your repository, with `iohr.lock` beside the directory. A call a
profile may not make does not compile (in Python, pyright and mypy refuse it).

- `--lang` takes `rust`, `typescript`, `python`, `go`, `java`, `csharp` and `swift`.
- `--for PROFILE` names a profile; repeat it to put several accounts in one surface. A
  person profile is cut to the account it acts for (`iohr profile account`).
- `--from NAME=FILE` reads a document saved with `iohr openapi pull`, for an offline run.
- `--force` replaces a non-empty output directory.
- `--runtime NAME` is the runtime's name in your project. The defaults are the published
  runtimes: `inorbithr` (Rust, Python), `@inorbithr/sdk` (TypeScript),
  `github.com/inorbithr/sdk/go` (Go), `hr.inorbit.sdk` (Java), `InOrbit.Sdk` (C#) and
  `InOrbit` (Swift).
- `--package NAME` is the package or namespace the surface goes into (Go, Java, C#). In Go
  each profile is a package and the import path is read from the enclosing `go.mod`; in C#
  the default namespace is `InOrbit.Generated`.

Why the surface is generated into your repository: [ADR 0011](../../docs/adr/0011-runtime-and-surface.md).

### Checking the cut in CI

```sh
iohr sdk check                # --lock iohr.lock by default
iohr sdk check --files        # also list the generated files that differ from what is on disk
```

`iohr sdk check` fetches every profile's document again and exits 1 with what moved when
the cut has changed, so a plan change or a revoked scope is a failing check, not a surprise
in production. In CI, a token per profile in `IOHR_TOKEN_<PROFILE>` (`IOHR_TOKEN_CI`,
`IOHR_TOKEN_DEFAULT`) stands in for the profile; the bare `IOHR_TOKEN` does not. A person
profile cannot sign in on CI: give it a token there, or leave it out of the lock.

## Examples for a reference

```sh
iohr sdk examples --from openapi.public.json --out examples.json
```

For every operation of the document, one program per language that builds the client,
makes the call with its path and required query parameters filled (the document's
`example`, or a `<name>` placeholder), walks every page of a list, reads a stream in a
loop, and reports the API's error code and request id.

- `--from [NAME=]FILE` is the document: the public one, or one from `iohr openapi pull`.
- `--lang L` picks a language; repeat it for several. Every language when left out.
- `--out FILE` is where to write; the default `-` is standard output.

The output is JSON, `operations.<operationId>.code.<lang>`, with the generator's version,
the document's API version and its cut hash. Each snippet uses the published runtime's
public surface, and the compile tests build every one against each runtime
([`compile_examples.rs`](../crates/iohr-codegen/tests/compile_examples.rs)). How a snippet
builds its client is one template per language
([`examples/client.rs`](../crates/iohr-codegen/src/examples/client.rs)), so a change to the
client configuration changes every example in one place. The API reference on the docs
site shows these beside each operation.

## `iohr api`

```sh
iohr api GET /v1/me
iohr api GET /v1/radar/digests -f page_size=5
iohr api GET /v1/webhooks/endpoints --all     # every page of a list as one answer
```

One call to the API as the active profile.

| Flag | What it does |
|---|---|
| `METHOD` | `GET`, `POST`, `PUT`, `PATCH` or `DELETE` |
| `PATH` | A path on the API, such as `/v1/me`; it must stay on the API host |
| `-f`, `--field KEY=VALUE` | A string field: a query value for `GET` and `DELETE`, a JSON body field otherwise |
| `-F`, `--typed-field KEY=JSON` | A field whose value is read as JSON (numbers, `true`, `false`, `null`, arrays) |
| `--input FILE` | The JSON body from a file, or `-` for standard input; not with `-f` or `-F` |
| `-i`, `--include` | Print the status and the request id before the body |
| `--all` (alias `--paginate`) | `GET` every page of a list: follow `next_page_token` until it is empty and print one answer with every item |
| `--max-pages N` | With `--all`, stop after N pages (1 to 10,000, default 100); the answer then keeps the token to go on with (`-f page_token=...`) |

## `iohr openapi pull`

```sh
iohr openapi pull                 # writes openapi.json
iohr openapi pull -o -            # to standard output
```

Saves the OpenAPI document this credential sees: the operations it may call. `-o FILE`
names the file (default `openapi.json`). `iohr sdk generate --from` and
`iohr sdk examples --from` read it.
