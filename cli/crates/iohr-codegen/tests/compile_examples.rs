//! Every example `iohr sdk examples` writes builds against the language's runtime and
//! its public surface: the examples of `spec/openapi.json`, the document the runtimes'
//! surfaces are generated from, one program per operation and language. This is the
//! guard against an example that drifts from the generated surface. Each language runs
//! with `IOHR_TEST_COMPILE=<lang>` and the same setup as its surface compile test.
//!
//! Java and C# examples are statements, as the reference shows them; the test puts each
//! in a method of its own class, imports and `using`s first.

#![allow(clippy::unwrap_used, clippy::print_stderr, reason = "test helpers")]

mod support;

use std::fmt::Write as _;
use std::path::Path;
use std::process::{Command, Output};

use iohr_codegen::Language;
use iohr_codegen::examples::Examples;
use iohr_openapi::Api;

use support::{compile_enabled, repo};

/// The examples of `spec/openapi.json` in `lang`, as (operation id, code).
fn examples(lang: Language) -> Vec<(String, String)> {
    let text = std::fs::read_to_string(repo().join("spec/openapi.json")).unwrap();
    let doc = serde_json::from_str(&text).unwrap();
    let api = Api::from_documents([("public".to_owned(), doc)]).unwrap();
    let examples: Examples = iohr_codegen::examples::render(&api, &[lang], "iohr test").unwrap();
    assert!(
        examples.notes.is_empty(),
        "an operation has no example: {:?}",
        examples.notes
    );
    assert_eq!(examples.operations.len(), api.operations.len());
    examples
        .operations
        .into_iter()
        .map(|(id, op)| (id, op.code[lang.as_str()].clone()))
        .collect()
}

fn text(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

/// The examples, numbered, for an error message that names the operation.
fn index(all: &[(String, String)]) -> String {
    all.iter()
        .enumerate()
        .fold(String::new(), |mut s, (i, (id, _))| {
            let _ = writeln!(s, "  {i}: {id}");
            s
        })
}

fn check(lang: Language, all: &[(String, String)], out: &Output, what: &str) {
    assert!(
        out.status.success(),
        "{what} failed on the {lang} examples:\n{}\nthe examples by number:\n{}",
        text(out),
        index(all)
    );
}

#[test]
fn the_typescript_examples_type_check() {
    if !compile_enabled(Language::TypeScript) {
        eprintln!("compile test: set IOHR_TEST_COMPILE=typescript to run it (CI does); skipping");
        return;
    }
    let package = repo().join("typescript").canonicalize().unwrap();
    let tsc = package.join("node_modules/.bin/tsc");
    assert!(
        tsc.is_file(),
        "no {}: run `pnpm install` in typescript/ first",
        tsc.display()
    );
    let all = examples(Language::TypeScript);
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    for (i, (_, code)) in all.iter().enumerate() {
        std::fs::write(root.join(format!("ex{i}.ts")), code).unwrap();
    }
    std::fs::write(
        root.join("tsconfig.json"),
        format!(
            r#"{{
  "compilerOptions": {{
    "target": "ES2023", "lib": ["ES2023", "DOM"], "module": "NodeNext", "moduleResolution": "NodeNext",
    "strict": true, "noUncheckedIndexedAccess": true, "exactOptionalPropertyTypes": true,
    "verbatimModuleSyntax": true, "noEmit": true, "skipLibCheck": true, "types": [],
    "paths": {{ "@inorbithr/sdk": [{:?}] }}
  }},
  "include": ["*.ts"]
}}"#,
            package.join("src/index.ts").display().to_string()
        ),
    )
    .unwrap();
    std::fs::write(
        root.join("package.json"),
        r#"{ "type": "module", "private": true }"#,
    )
    .unwrap();
    let out = Command::new(&tsc)
        .args(["-p", "tsconfig.json"])
        .current_dir(root)
        .output()
        .unwrap();
    check(Language::TypeScript, &all, &out, "tsc");
}

#[test]
fn the_python_examples_type_check() {
    if !compile_enabled(Language::Python) {
        eprintln!("compile test: set IOHR_TEST_COMPILE=python to run it (CI does); skipping");
        return;
    }
    let package = repo().join("python").canonicalize().unwrap();
    let bin = package.join(if cfg!(windows) {
        ".venv/Scripts"
    } else {
        ".venv/bin"
    });
    let pyright = bin.join("pyright");
    assert!(
        pyright.exists(),
        "no pyright in {}: run `uv sync` in python/ first",
        bin.display()
    );
    let all = examples(Language::Python);
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    for (i, (_, code)) in all.iter().enumerate() {
        std::fs::write(root.join(format!("ex{i}.py")), code).unwrap();
    }
    std::fs::write(
        root.join("pyrightconfig.json"),
        format!(
            r#"{{ "typeCheckingMode": "strict", "pythonVersion": "3.11", "venvPath": {:?}, "venv": ".venv" }}"#,
            package.display().to_string()
        ),
    )
    .unwrap();
    let out = Command::new(&pyright)
        .arg(".")
        .current_dir(root)
        .output()
        .unwrap();
    check(Language::Python, &all, &out, "pyright");
}

fn go(dir: &Path, args: &[&str]) -> Output {
    Command::new("go")
        .args(args)
        .current_dir(dir)
        .env("GOFLAGS", "-mod=mod")
        .env("GOWORK", "off")
        .env("GOPROXY", "off")
        .env("GOTOOLCHAIN", "local")
        .output()
        .unwrap()
}

#[test]
fn the_go_examples_build() {
    if !compile_enabled(Language::Go) {
        eprintln!("compile test: set IOHR_TEST_COMPILE=go to run it (CI does); skipping");
        return;
    }
    let runtime = repo().join("go").canonicalize().unwrap();
    let all = examples(Language::Go);
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(
        root.join("go.mod"),
        format!(
            "module example.com/app\n\ngo 1.26\n\nrequire github.com/inorbithr/sdk/go v0.0.0\n\nreplace github.com/inorbithr/sdk/go => {}\n",
            runtime.display()
        ),
    )
    .unwrap();
    for (i, (_, code)) in all.iter().enumerate() {
        let sub = root.join(format!("ex{i}"));
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("main.go"), code).unwrap();
    }
    let vet = go(root, &["vet", "./..."]);
    check(Language::Go, &all, &vet, "go vet");
    let fmt = Command::new("gofmt")
        .arg("-l")
        .arg(".")
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        fmt.status.success() && fmt.stdout.is_empty(),
        "the Go examples are not gofmt-clean:\n{}\nthe examples by number:\n{}",
        text(&fmt),
        index(&all)
    );
}

#[test]
fn the_rust_examples_compile() {
    if !compile_enabled(Language::Rust) {
        eprintln!("compile test: set IOHR_TEST_COMPILE=rust to run it (CI does); skipping");
        return;
    }
    let runtime = repo().join("rust").canonicalize().unwrap();
    let all = examples(Language::Rust);
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(
        root.join("Cargo.toml"),
        format!(
            r#"[package]
name = "examples-check"
version = "0.0.0"
edition = "2024"
publish = false

[dependencies]
inorbithr = {{ path = {runtime:?} }}
tokio = {{ version = "1", features = ["macros", "rt-multi-thread"] }}
"#
        ),
    )
    .unwrap();
    let bins = root.join("src/bin");
    std::fs::create_dir_all(&bins).unwrap();
    for (i, (_, code)) in all.iter().enumerate() {
        std::fs::write(bins.join(format!("ex{i}.rs")), code).unwrap();
    }
    let out = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
        .args(["check", "--bins", "--message-format", "short", "--quiet"])
        .current_dir(root)
        .output()
        .unwrap();
    check(Language::Rust, &all, &out, "cargo check");
}

/// A snippet's leading `import` or `using` lines, and the rest.
fn split(code: &str, keyword: &str) -> (String, String) {
    let mut head = String::new();
    let mut body = String::new();
    let mut in_head = true;
    for line in code.lines() {
        if in_head && (line.starts_with(keyword) || line.is_empty()) {
            head.push_str(line);
            head.push('\n');
        } else {
            in_head = false;
            body.push_str("        ");
            body.push_str(line);
            body.push('\n');
        }
    }
    (head, body)
}

#[test]
fn the_java_examples_compile() {
    if !compile_enabled(Language::Java) {
        eprintln!("compile test: set IOHR_TEST_COMPILE=java to run it (CI does); skipping");
        return;
    }
    let project = repo().join("java").canonicalize().unwrap();
    let classes = project.join("target/classes");
    assert!(
        classes.join("hr/inorbit/sdk/Client.class").is_file(),
        "no {}: run `mise run java:check` (mvn verify) first",
        classes.display()
    );
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let deps = root.join("deps.txt");
    let mvn = Command::new("mvn")
        .args(["-B", "-q", "-f"])
        .arg(project.join("pom.xml"))
        .args(["dependency:build-classpath", "-Dmdep.includeScope=compile"])
        .arg(format!("-Dmdep.outputFile={}", deps.display()))
        .output()
        .unwrap();
    assert!(
        mvn.status.success(),
        "mvn could not list the runtime's classpath:\n{}",
        text(&mvn)
    );
    let classpath = format!(
        "{}{}{}",
        classes.display(),
        if cfg!(windows) { ";" } else { ":" },
        std::fs::read_to_string(&deps).unwrap().trim()
    );
    let all = examples(Language::Java);
    let src = root.join("src/ex");
    std::fs::create_dir_all(&src).unwrap();
    let mut sources = Vec::new();
    for (i, (_, code)) in all.iter().enumerate() {
        let (imports, body) = split(code, "import ");
        let file = src.join(format!("Ex{i}.java"));
        std::fs::write(
            &file,
            format!(
                "package ex;\n\n{imports}\n/** Example {i}. */\npublic final class Ex{i} {{\n    private Ex{i}() {{}}\n\n    /** Runs it. */\n    public static void run() {{\n{body}    }}\n}}\n"
            ),
        )
        .unwrap();
        sources.push(file);
    }
    let out = Command::new("javac")
        .args(["--release", "17", "-Xlint:all", "-Werror", "-d"])
        .arg(root.join("classes"))
        .args(["-cp", &classpath])
        .args(&sources)
        .current_dir(root)
        .output()
        .unwrap();
    check(Language::Java, &all, &out, "javac");
}

#[test]
fn the_csharp_examples_build() {
    if !compile_enabled(Language::CSharp) {
        eprintln!("compile test: set IOHR_TEST_COMPILE=csharp to run it (CI does); skipping");
        return;
    }
    let runtime = repo()
        .join("csharp/src/InOrbit.Sdk/InOrbit.Sdk.csproj")
        .canonicalize()
        .unwrap();
    let all = examples(Language::CSharp);
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
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
    std::fs::write(
        root.join("Program.cs"),
        "namespace Check;\n\npublic static class Program\n{\n    public static void Main()\n    {\n    }\n}\n",
    )
    .unwrap();
    for (i, (_, code)) in all.iter().enumerate() {
        let (usings, body) = split(code, "using ");
        std::fs::write(
            root.join(format!("Ex{i}.cs")),
            format!(
                "using System.Threading.Tasks;\n{usings}\nnamespace Check;\n\npublic static class Ex{i}\n{{\n    public static async Task Run()\n    {{\n{body}    }}\n}}\n"
            ),
        )
        .unwrap();
    }
    let out = Command::new("dotnet")
        .args(["build", "-nologo", "-v", "q", "-warnaserror"])
        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
        .env("DOTNET_NOLOGO", "1")
        .current_dir(root)
        .output()
        .unwrap();
    check(Language::CSharp, &all, &out, "dotnet build");
}

#[test]
fn the_swift_examples_build() {
    if !compile_enabled(Language::Swift) {
        eprintln!("compile test: set IOHR_TEST_COMPILE=swift to run it (CI does); skipping");
        return;
    }
    let runtime = repo().canonicalize().unwrap();
    let identity = runtime
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_lowercase();
    let all = examples(Language::Swift);
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(
        root.join("Package.swift"),
        format!(
            "// swift-tools-version:5.9\nimport PackageDescription\n\nlet package = Package(\n    name: \"Check\",\n    platforms: [.macOS(.v12)],\n    dependencies: [.package(path: {:?})],\n    targets: [.target(name: \"Check\", dependencies: [.product(name: \"InOrbit\", package: \"{identity}\")], path: \"Sources/Check\")]\n)\n",
            runtime.display().to_string()
        ),
    )
    .unwrap();
    let sources = root.join("Sources/Check");
    std::fs::create_dir_all(&sources).unwrap();
    for (i, (_, code)) in all.iter().enumerate() {
        let (imports, body) = split(code, "import ");
        let body: String = body.lines().fold(String::new(), |mut s, l| {
            let _ = writeln!(s, "    {l}");
            s
        });
        std::fs::write(
            sources.join(format!("Ex{i}.swift")),
            format!("{imports}\nfunc ex{i}() async throws {{\n{body}}}\n"),
        )
        .unwrap();
    }
    let out = Command::new("swift")
        .args(["build", "--package-path"])
        .arg(root)
        .output()
        .unwrap();
    check(Language::Swift, &all, &out, "swift build");
}
