//! The generated C# surface builds against `csharp/` (the runtime), and a call a profile
//! may not make does not. Runs with `IOHR_TEST_COMPILE=csharp` (the csharp CI job sets
//! it): it writes a throwaway project referencing the runtime and runs `dotnet build`.

#![allow(clippy::unwrap_used, clippy::print_stderr, reason = "test helpers")]

mod support;

use std::path::Path;
use std::process::Command;

use iohr_codegen::{Language, Options};

use support::{compile_enabled, repo};

const PROGRAM: &str = r#"using System.Threading.Tasks;
using Acme.Api;
using InOrbit.Sdk;

namespace Check;

public static class Program
{
    /// <summary>What any program may write with this surface.</summary>
    public static async Task Fine()
    {
        using var personal = new Client<Personal>(new ClientOptions { Token = "t" });
        await personal.MeAsync();
        await personal.Radar().ListDigestsAsync(new RadarListDigestsParams { Limit = 5 });
        using var ci = Client.FromEnv<AcmeCi>();
        await ci.Accounts().GetMeAsync();
        await ci.Accounts().GetUsageAsync("acc_1", new AccountsGetUsageParams { From = "2026-09-01" });
        var made = await ci.Events().CreateEndpointAsync(new CreateEndpointRequest { Url = "https://example.com" });
        _ = made.Value.Secret;
    }

    public static void Main()
    {
    }
}
"#;

/// The personal cut has no usage:read: the call must not compile.
const WRONG: &str = r#"using System.Threading.Tasks;
using Acme.Api;
using InOrbit.Sdk;

namespace Check;

public static class Wrong
{
    public static async Task Call(Client<Personal> personal)
    {
        await personal.Accounts().GetUsageAsync("acc_1");
    }
}
"#;

fn project(root: &Path, runtime: &Path) {
    std::fs::write(
        root.join("Check.csproj"),
        format!(
            r#"<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <OutputType>Exe</OutputType>
    <TargetFramework>net8.0</TargetFramework>
    <Nullable>enable</Nullable>
    <TreatWarningsAsErrors>true</TreatWarningsAsErrors>
    <ImplicitUsings>disable</ImplicitUsings>
  </PropertyGroup>
  <ItemGroup>
    <ProjectReference Include="{}" />
  </ItemGroup>
</Project>
"#,
            runtime.display()
        ),
    )
    .unwrap();
}

fn build(root: &Path) -> (bool, String) {
    let output = Command::new("dotnet")
        .args(["build", "-nologo", "-v", "q", "-warnaserror"])
        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
        .env("DOTNET_NOLOGO", "1")
        .current_dir(root)
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.success(), text)
}

#[test]
fn the_two_profile_surface_builds_and_the_wrong_profile_does_not() {
    if !compile_enabled(Language::CSharp) {
        eprintln!("compile test: set IOHR_TEST_COMPILE=csharp to run it (CI does); skipping");
        return;
    }
    let runtime = repo()
        .join("csharp/src/InOrbit.Sdk/InOrbit.Sdk.csproj")
        .canonicalize()
        .unwrap();
    let options = Options {
        package: Some("Acme.Api".into()),
        ..Options::default()
    };
    let (_, files) = support::render("two-profiles", Language::CSharp, &options);
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    files.write(&root.join("iohr"), true).unwrap();
    project(root, &runtime);
    std::fs::write(root.join("Program.cs"), PROGRAM).unwrap();
    let (ok, text) = build(root);
    assert!(ok, "dotnet build failed on the generated surface:\n{text}");

    std::fs::write(root.join("Wrong.cs"), WRONG).unwrap();
    let (ok, text) = build(root);
    assert!(
        !ok,
        "a call the personal profile may not make compiled:\n{text}"
    );
    assert!(
        text.contains("CS0311") || text.contains("CS0315"),
        "the build failed for another reason than the profile's constraint:\n{text}"
    );
}
