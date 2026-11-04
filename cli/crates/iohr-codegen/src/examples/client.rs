//! How every example is laid out: one program per language, and the only place an
//! example says how a client is built, closed, stopped and how an API error is reported.
//! When the client configuration changes, this file changes and every example follows;
//! the compile tests prove the new program against each runtime.
//!
//! Each program names the imports it needs (merged into the snippet's own) and its lines
//! in order, with [`BODY`] where the operation's call goes; every line of the call takes
//! that line's indentation. The construction leaves the profile's surface in a variable
//! named [`VAR`]. Every flow (one call, pages, a stream) fills the same program, so a
//! language's skeleton is written once.
//!
//! What each program does, by language, and why:
//!
//! - the client is closed when the runtime gives it a close: Go (`defer api.Close()`),
//!   Python (`with`), C# (`using var`); TypeScript (fetch), Java (JDK 17 `HttpClient`),
//!   Swift (the session ends in `deinit`) and Rust (`Drop`) hold nothing to close;
//! - Ctrl-C stops the program cleanly: Go cancels its context with
//!   `signal.NotifyContext`, Rust drops the call on `tokio::signal::ctrl_c`, so streams
//!   and pages end and the client is closed; Python's `KeyboardInterrupt` leaves its `with`
//!   blocks; the other runtimes end the process, which closes its connections;
//! - an API error prints its code, problem and request id on standard error and the
//!   program exits with status 1; any other error is left to the language's default,
//!   which also exits non-zero.

/// The variable the examples call the operations on.
pub const VAR: &str = "api";

/// Where the operation's call goes in a [`Construction::program`].
pub const BODY: &str = "{body}";

/// One language's program.
#[derive(Debug, Clone, Copy)]
pub struct Construction {
    /// What the program imports from the SDK and the standard library, in the language's
    /// own import form (Go and Python keep the standard library apart, as their style
    /// guides group it).
    pub imports: &'static [&'static str],
    /// The standard-library imports, for a language that groups them apart.
    pub std: &'static [&'static str],
    /// The program's lines, without the imports, with one [`BODY`] line.
    pub program: &'static [&'static str],
}

impl Construction {
    /// The program with `body` at [`BODY`], each of its lines indented as that line is;
    /// an empty line stays empty.
    #[must_use]
    pub fn frame(&self, body: &str) -> String {
        let mut out = String::new();
        for line in self.program {
            if let Some(indent) = line.strip_suffix(BODY) {
                for l in body.lines() {
                    if !l.is_empty() {
                        out.push_str(indent);
                        out.push_str(l);
                    }
                    out.push('\n');
                }
            } else {
                out.push_str(line);
                out.push('\n');
            }
        }
        out
    }
}

/// What the comment on each construction says.
#[cfg(test)]
const FROM_ENV: &str = "INORBIT_TOKEN, or INORBIT_KEY_ID and INORBIT_KEY_SECRET";

/// TypeScript and JavaScript, on `@inorbithr/sdk`: an ES module with top-level `await`.
pub const TYPESCRIPT: Construction = Construction {
    imports: &["ApiError", "Public"],
    std: &[],
    program: &[
        "// Reads INORBIT_TOKEN, or INORBIT_KEY_ID and INORBIT_KEY_SECRET.",
        "const api = Public.fromEnv();",
        "",
        "try {",
        "  {body}",
        "} catch (e) {",
        "  if (!(e instanceof ApiError)) throw e;",
        "  console.error(`${e.code}: ${e.problem} (request id ${e.raw.requestId})`);",
        "  process.exitCode = 1;",
        "}",
    ],
};

/// Python, on `inorbithr`: `with` closes the client; `sys.exit` with a message prints it
/// on standard error and exits with status 1.
pub const PYTHON: Construction = Construction {
    imports: &["ApiError", "Public"],
    std: &["sys"],
    program: &[
        "# Reads INORBIT_TOKEN, or INORBIT_KEY_ID and INORBIT_KEY_SECRET.",
        "with Public.from_env() as api:",
        "    try:",
        "        {body}",
        "    except ApiError as e:",
        "        sys.exit(f\"{e.code}: {e.problem} (request id {e.raw.request_id})\")",
    ],
};

/// Go, on `github.com/inorbithr/sdk/go`: `run` returns the error, so its deferred calls
/// run before `main` exits; Ctrl-C or SIGTERM cancels `ctx`, which ends the call and is
/// not reported as a failure.
pub const GO: Construction = Construction {
    imports: &[
        "inorbit \"github.com/inorbithr/sdk/go\"",
        "\"github.com/inorbithr/sdk/go/public\"",
    ],
    std: &[
        "context",
        "errors",
        "fmt",
        "log",
        "os",
        "os/signal",
        "syscall",
    ],
    program: &[
        "func main() {",
        "\terr := run()",
        "\tif err == nil || errors.Is(err, context.Canceled) {",
        "\t\treturn // done, or stopped with Ctrl-C",
        "\t}",
        "\tvar apiErr *inorbit.APIError",
        "\tif errors.As(err, &apiErr) {",
        "\t\tlog.Fatalf(\"%s: %s (request id %s)\", apiErr.Code, apiErr.Problem, apiErr.Raw.RequestID)",
        "\t}",
        "\tlog.Fatal(err)",
        "}",
        "",
        "func run() error {",
        "\tctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)",
        "\tdefer stop()",
        "",
        "\t// Reads INORBIT_TOKEN, or INORBIT_KEY_ID and INORBIT_KEY_SECRET.",
        "\tapi, err := public.FromEnv()",
        "\tif err != nil {",
        "\t\treturn err",
        "\t}",
        "\tdefer api.Close()",
        "",
        "\t{body}",
        "\treturn nil",
        "}",
    ],
};

/// Java, on `hr.inorbit:inorbit-sdk`: statements, as the reference shows them.
pub const JAVA: Construction = Construction {
    imports: &[
        "hr.inorbit.sdk.errors.ApiException",
        "hr.inorbit.sdk.generated.Public",
    ],
    std: &[],
    program: &[
        "// Reads INORBIT_TOKEN, or INORBIT_KEY_ID and INORBIT_KEY_SECRET.",
        "Public api = Public.fromEnv();",
        "try {",
        "    {body}",
        "} catch (ApiException e) {",
        "    System.err.println(e.code() + \": \" + e.problem() + \" (request id \" + e.raw().requestId() + \")\");",
        "    System.exit(1);",
        "}",
    ],
};

/// C#, on `InOrbit.Sdk`: top-level statements; `using var` disposes the client.
pub const CSHARP: Construction = Construction {
    imports: &["System", "InOrbit.Sdk", "InOrbit.Sdk.Api"],
    std: &[],
    program: &[
        "// Reads INORBIT_TOKEN, or INORBIT_KEY_ID and INORBIT_KEY_SECRET.",
        "using var api = Client.FromEnv();",
        "try",
        "{",
        "    {body}",
        "}",
        "catch (ApiException e)",
        "{",
        "    Console.Error.WriteLine($\"{e.Code}: {e.Problem} (request id {e.Raw.RequestId})\");",
        "    Environment.ExitCode = 1;",
        "}",
    ],
};

/// Swift, on the `InOrbit` package: top-level code (`main.swift`) in an `async throws`
/// context; the construction is inside `do`, so its error is handled with the call's.
pub const SWIFT: Construction = Construction {
    imports: &["Foundation", "InOrbit"],
    std: &[],
    program: &[
        "do {",
        "    // Reads INORBIT_TOKEN, or INORBIT_KEY_ID and INORBIT_KEY_SECRET.",
        "    let api = try Client<Public>.fromEnv()",
        "    {body}",
        "} catch let e as APIError {",
        "    FileHandle.standardError.write(Data(\"\\(e.code): \\(e.problem) (request id \\(e.raw.requestId))\\n\".utf8))",
        "    exit(1)",
        "}",
    ],
};

/// Rust, on `inorbithr` and tokio: `run` returns the error and `main` maps it to the exit
/// status; Ctrl-C drops `run`, which ends the call and drops the client.
pub const RUST: Construction = Construction {
    imports: &[
        "inorbithr::Client",
        "inorbithr::Error",
        "std::process::ExitCode",
    ],
    std: &[],
    program: &[
        "#[tokio::main]",
        "async fn main() -> ExitCode {",
        "    let result = tokio::select! {",
        "        result = run() => result,",
        "        _ = tokio::signal::ctrl_c() => Ok(()),",
        "    };",
        "    match result {",
        "        Ok(()) => ExitCode::SUCCESS,",
        "        Err(Error::Api(e)) => {",
        "            eprintln!(\"{}: {} (request id {})\", e.code, e.message, e.raw.request_id);",
        "            ExitCode::FAILURE",
        "        }",
        "        Err(e) => {",
        "            eprintln!(\"{e}\");",
        "            ExitCode::FAILURE",
        "        }",
        "    }",
        "}",
        "",
        "async fn run() -> Result<(), Error> {",
        "    // Reads INORBIT_TOKEN, or INORBIT_KEY_ID and INORBIT_KEY_SECRET.",
        "    let api: Client = Client::from_env()?;",
        "    {body}",
        "    Ok(())",
        "}",
    ],
};

/// Every language's program, for the tests.
#[cfg(test)]
pub(crate) const ALL: [Construction; 7] = [TYPESCRIPT, PYTHON, GO, JAVA, CSHARP, SWIFT, RUST];

#[cfg(test)]
mod tests {
    use super::{ALL, BODY, FROM_ENV, VAR};

    #[test]
    fn every_program_builds_its_client_from_the_same_environment_once() {
        for c in ALL {
            assert!(c.program.iter().any(|l| l.contains(FROM_ENV)));
            assert!(c.program.iter().any(|l| l.contains(VAR)));
            assert_eq!(
                c.program.iter().filter(|l| l.ends_with(BODY)).count(),
                1,
                "one place for the call"
            );
        }
    }

    #[test]
    fn the_body_takes_the_indentation_of_its_line() {
        let framed = super::PYTHON.frame("for x in y:\n    print(x)\n\nprint(1)");
        assert!(
            framed.contains("\n        for x in y:\n            print(x)\n\n        print(1)\n")
        );
    }
}
