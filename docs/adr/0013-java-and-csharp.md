# 0013. Java and C#

Status: accepted, 2026-10-03

## Context

Platform RFC 0020 decided that `iohr sdk generate` produces SDKs in every language the
API docs show samples for: TypeScript and JavaScript, Python, Go, Java, C# and Rust.
ADR 0006 named the first four packages; Java and C# had no names, no minimum runtimes
and no place in this repository. Their surface follows ADR 0011 like every other: a
hand-written runtime, and the operations generated onto it per credential.

## Decision

| Language | Distribution name | Import | Minimum | CI |
|---|---|---|---|---|
| Java | Maven Central `hr.inorbit:inorbit-sdk` | `import hr.inorbit.sdk.Client;` | Java 17 | Temurin 17 and 21 |
| C# | NuGet `InOrbit.Sdk` | `using InOrbit.Sdk;` | `net8.0` | built with the .NET 10 SDK |

- **Java.** The group is the reversed domain `inorbit.hr`, which Maven Central verifies
  by a DNS record before the first publish. Java 17 is the oldest long-term release with
  updates from Temurin through 2027. The runtime uses `java.net.http` from the JDK and
  Jackson for JSON, nothing else; it builds with Maven at `--release 17` and `-Werror`.
  A generated surface is one class per profile with a handle per tag and records for the
  models, so a call a profile may not make does not compile.
- **C#.** One package, `InOrbit.Sdk`, targeting `net8.0`, which every supported .NET
  runtime loads; the repository builds it with the .NET 10 SDK, the current long-term
  release. The runtime uses `HttpClient` and `System.Text.Json` from the framework,
  nothing else, with nullable reference types and warnings as errors. A generated
  surface follows the Rust shape: a marker interface per operation and extension methods
  constrained to it (`where P : IListDigests`), so the compiler refuses a call the
  profile may not make.
- Both live in `java/` and `csharp/`, have their own CI jobs and conformance drivers, and
  publish nothing before the first release (M4), like the other four.

## Consequences

- .NET 8 leaves support on 2026-11-10. `net8.0` still loads on .NET 9 and 10; before 1.0
  the target moves to the oldest .NET release then supported, in a minor release.
- The first Maven Central publish waits for the `hr.inorbit` namespace to be verified,
  which needs a TXT record on `inorbit.hr` from whoever controls the zone.
- Six runtimes are six times the maintenance of one; the conformance suite and the
  shared generator model (`iohr-codegen` `ir` and `context`) keep them the same.

## Sources

- Maven Central namespace verification: https://central.sonatype.org/register/namespace/
- Temurin support roadmap: https://adoptium.net/support/
- .NET support policy and dates: https://dotnet.microsoft.com/en-us/platform/support/policy/dotnet-core
- Target frameworks (`net8.0`): https://learn.microsoft.com/en-us/dotnet/standard/frameworks
