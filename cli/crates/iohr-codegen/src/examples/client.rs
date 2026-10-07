//! How every example builds its client: one template per language, and the only place
//! an example says how a client is configured. When the client configuration changes,
//! this file changes and every example follows; the compile tests prove the new
//! construction against each runtime.
//!
//! Each template holds the imports it needs (merged into the snippet's own) and the
//! lines that leave the profile's surface in a variable named [`VAR`].

/// The variable the examples call the operations on.
pub const VAR: &str = "api";

/// One language's construction.
#[derive(Debug, Clone, Copy)]
pub struct Construction {
    /// What the construction imports, in the language's own import form.
    pub imports: &'static [&'static str],
    /// The lines that build the client, without indentation.
    pub lines: &'static [&'static str],
}

/// What the comment on each construction says.
#[cfg(test)]
const FROM_ENV: &str = "INORBIT_TOKEN, or INORBIT_KEY_ID and INORBIT_KEY_SECRET";

/// TypeScript and JavaScript, on `@inorbithr/sdk`.
pub const TYPESCRIPT: Construction = Construction {
    imports: &["Public"],
    lines: &[
        "// Reads INORBIT_TOKEN, or INORBIT_KEY_ID and INORBIT_KEY_SECRET.",
        "const api = Public.fromEnv();",
    ],
};

/// Python, on `inorbithr`.
pub const PYTHON: Construction = Construction {
    imports: &["Public"],
    lines: &[
        "# Reads INORBIT_TOKEN, or INORBIT_KEY_ID and INORBIT_KEY_SECRET.",
        "api = Public.from_env()",
    ],
};

/// Go, on `github.com/inorbithr/sdk/go`: inside `run() error`, so the error returns.
pub const GO: Construction = Construction {
    imports: &["github.com/inorbithr/sdk/go/public"],
    lines: &[
        "// Reads INORBIT_TOKEN, or INORBIT_KEY_ID and INORBIT_KEY_SECRET.",
        "api, err := public.FromEnv()",
        "if err != nil {",
        "\treturn err",
        "}",
    ],
};

/// Java, on `hr.inorbit:inorbit-sdk`.
pub const JAVA: Construction = Construction {
    imports: &["hr.inorbit.sdk.generated.Public"],
    lines: &[
        "// Reads INORBIT_TOKEN, or INORBIT_KEY_ID and INORBIT_KEY_SECRET.",
        "Public api = Public.fromEnv();",
    ],
};

/// C#, on `InOrbit.Sdk`.
pub const CSHARP: Construction = Construction {
    imports: &["InOrbit.Sdk"],
    lines: &[
        "// Reads INORBIT_TOKEN, or INORBIT_KEY_ID and INORBIT_KEY_SECRET.",
        "using var api = Client.FromEnv();",
    ],
};

/// Swift, on the `InOrbit` package: inside an `async throws` context, so the error throws.
pub const SWIFT: Construction = Construction {
    imports: &["InOrbit"],
    lines: &[
        "// Reads INORBIT_TOKEN, or INORBIT_KEY_ID and INORBIT_KEY_SECRET.",
        "let api = try Client<Public>.fromEnv()",
    ],
};

/// Rust, on `inorbithr`: inside `async fn run() -> Result<(), Error>`.
pub const RUST: Construction = Construction {
    imports: &["inorbithr::Client"],
    lines: &[
        "// Reads INORBIT_TOKEN, or INORBIT_KEY_ID and INORBIT_KEY_SECRET.",
        "let api: Client = Client::from_env()?;",
    ],
};

#[cfg(test)]
mod tests {
    #[test]
    fn every_construction_names_the_same_environment() {
        for c in [
            super::TYPESCRIPT,
            super::PYTHON,
            super::GO,
            super::JAVA,
            super::CSHARP,
            super::SWIFT,
            super::RUST,
        ] {
            assert!(c.lines[0].contains(super::FROM_ENV));
            assert!(c.lines.iter().any(|l| l.contains(super::VAR)));
        }
    }
}
