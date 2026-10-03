# C# package

NuGet `InOrbit.Sdk`, namespace `InOrbit.Sdk`, `net8.0` (ADR 0013), built with the .NET 10
SDK. Read the root `AGENTS.md`, `docs/design.md` (section 12 above all) and ADR 0011
first; this file adds only what is specific to C#.

## Runtime and surface

The package is the hand-written **runtime** plus the **public surface** `iohr sdk
generate --lang csharp` writes into `src/InOrbit.Sdk/Generated/`. The runtime holds no
operation; the surface calls the runtime's one request path. The Rust crate (`rust/`) is
the reference: mirror its client, builder, token providers, errors, retries, hooks and
`Int64`.

A generated surface follows the Rust shape: `Client<P>` over a profile type, one marker
interface per operation implemented by the profiles whose cut holds it, and extension
methods constrained to the marker (`where P : IListDigests`), so a call a profile may
not make does not compile. `--package` names the surface's namespace.

## Commands

- `mise run csharp:check`: `dotnet format --verify-no-changes`, `dotnet build
  -warnaserror`, `dotnet test`
- `mise run csharp:fmt`, `mise run csharp:gen` (`iohr sdk generate` into `Generated/`)
- `mise run conformance:csharp`: the driver, `dotnet test --filter Category=Conformance`
- The generator's C# target lives in `cli/crates/iohr-codegen/src/csharp/`; golden files
  under `tests/golden/*/expected/csharp/`; `IOHR_TEST_COMPILE=csharp mise run
  cli:compile-test` builds a generated surface against this package and proves the wrong
  profile does not.

## Layout

```
csharp/
  InOrbit.Sdk.sln
  Directory.Build.props   Nullable enable, TreatWarningsAsErrors, LangVersion latest
  src/InOrbit.Sdk/
    Client.cs             Client<P>, the builder, the one request path
    Auth/, Errors/        ITokenProvider, client credentials (single flight); ApiException, Code, Detail
    Int64.cs, IHook.cs
    Codegen.cs            what generated surfaces import (Version, PathSegment)
    Generated/            written by iohr; never edit
  tests/InOrbit.Sdk.Tests/
    Conformance/          the driver for conformance/cases
```

## Rules

- Runtime dependencies: none beyond the framework (`HttpClient`, `System.Text.Json`).
  A dependency needs an ADR.
- Every call is `async` and takes a `CancellationToken` last.
- Errors: `InOrbitException` base, `ApiException` with `Code`, `Status`, `Details`;
  connection, timeout, auth and config exceptions as subclasses.
- `int64` values are `long` through `Int64`; a message field the gateway left out is
  `null` (nullable reference types on).
- `ToString()` of credentials redacts the secret; a test asserts it.
- XML doc comments on every public member (`GenerateDocumentationFile`).
- Publishing stays off until the first release (M4).
