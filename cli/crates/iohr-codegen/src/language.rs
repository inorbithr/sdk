//! The languages, their options, and the one place that picks a target. A new target
//! adds its module, one arm in [`render`], and flips [`Language::is_built`].

use std::fmt;
use std::str::FromStr;

use iohr_openapi::Api;

use crate::files::Files;
use crate::target::{RenderError, Target as _};

/// A language `iohr sdk generate` knows of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Language {
    /// Rust, on the `inorbithr` crate.
    Rust,
    /// TypeScript and JavaScript, on `@inorbithr/sdk`.
    TypeScript,
    /// Python, on `inorbithr`.
    Python,
    /// Go, on `github.com/inorbithr/sdk/go`.
    Go,
    /// Java, on `hr.inorbit:inorbit-sdk`.
    Java,
    /// C#, on `InOrbit.Sdk`.
    CSharp,
}

impl Language {
    /// Every language, in the order the docs list them.
    pub const ALL: [Self; 6] = [
        Self::TypeScript,
        Self::Python,
        Self::Go,
        Self::Java,
        Self::CSharp,
        Self::Rust,
    ];

    /// The name `--lang` and the lock use.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::TypeScript => "typescript",
            Self::Python => "python",
            Self::Go => "go",
            Self::Java => "java",
            Self::CSharp => "csharp",
        }
    }

    /// Whether this `iohr` renders the language yet. A language that is not built is
    /// refused with [`RenderError::NotBuilt`], never rendered as a placeholder.
    #[must_use]
    pub fn is_built(self) -> bool {
        matches!(
            self,
            Self::Rust | Self::TypeScript | Self::Python | Self::Go
        )
    }

    /// The runtime a surface refers to unless `--runtime` says otherwise: the crate, the
    /// package, the module or the namespace the language's runtime is published as.
    #[must_use]
    pub fn default_runtime(self) -> &'static str {
        match self {
            Self::Rust | Self::Python => "inorbithr",
            Self::TypeScript => "@inorbithr/sdk",
            Self::Go => "github.com/inorbithr/sdk/go",
            Self::Java => "hr.inorbit.sdk",
            Self::CSharp => "InOrbit.Sdk",
        }
    }
}

impl fmt::Display for Language {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Language {
    type Err = RenderError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|l| l.as_str() == s)
            .ok_or_else(|| RenderError::UnknownLanguage(s.to_owned()))
    }
}

/// The flags every target shares, flattened into `iohr sdk generate`. A target reads
/// the ones that mean something in its language and ignores the rest.
#[derive(Debug, Clone, Default, PartialEq, Eq, clap::Args)]
pub struct Options {
    /// The runtime the surface runs on, as your project names it (the crate, package,
    /// module or namespace); defaults to the published runtime of the language.
    #[arg(long, value_name = "NAME")]
    pub runtime: Option<String>,
    /// The package or namespace the surface itself is generated into (Go, Java, C#);
    /// defaults to one derived from the output directory.
    #[arg(long, value_name = "NAME")]
    pub package: Option<String>,
    /// Generate the runtime's own public surface, inside the runtime's package.
    #[arg(long, hide = true, alias = "in-crate")]
    pub in_package: bool,
}

impl Options {
    /// The runtime name for `lang`: `--runtime`, or the language's default.
    #[must_use]
    pub fn runtime_for(&self, lang: Language) -> String {
        self.runtime
            .clone()
            .unwrap_or_else(|| lang.default_runtime().to_owned())
    }
}

/// Renders `api` as `lang`'s surface.
///
/// # Errors
///
/// [`RenderError::NotBuilt`] for a language this `iohr` does not render yet, and the
/// target's own errors otherwise.
pub fn render(lang: Language, api: &Api, options: &Options) -> Result<Files, RenderError> {
    match lang {
        Language::Rust => crate::rust::RustTarget.render(api, options),
        Language::TypeScript => crate::typescript::TypeScriptTarget.render(api, options),
        Language::Python => crate::python::PythonTarget.render(api, options),
        Language::Go => crate::go::GoTarget.render(api, options),
        other => Err(RenderError::NotBuilt(other)),
    }
}
