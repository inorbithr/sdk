use std::fmt;

use iohr_openapi::Api;

use crate::files::Files;

/// Why a surface could not be rendered.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RenderError {
    /// The models could not be turned into types.
    #[error("the models could not be rendered: {0}")]
    Models(String),
    /// A template failed.
    #[error("the {file} template failed: {reason}")]
    Template {
        /// The file the template renders.
        file: &'static str,
        /// What went wrong.
        reason: String,
    },
    /// The rendered code is not syntactically valid, which is a bug in the target.
    #[error("the rendered {file} does not parse: {reason}")]
    Syntax {
        /// The file.
        file: &'static str,
        /// The parser's message.
        reason: String,
    },
}

/// One language. Adding a language is one implementation, with its own options, and
/// nothing else changes.
pub trait Target {
    /// The target's own flags (`--runtime`), flattened into `iohr sdk generate`.
    type Options: clap::Args + Default + fmt::Debug;

    /// The language, as `--lang` names it.
    const LANG: &'static str;

    /// Renders `api` into files relative to the output directory.
    ///
    /// # Errors
    ///
    /// [`RenderError`] when a model or a template cannot be rendered; never because of
    /// the file system, which [`Files::write`] handles.
    fn render(&self, api: &Api, options: &Self::Options) -> Result<Files, RenderError>;
}
