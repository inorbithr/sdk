use iohr_openapi::Api;

use crate::files::Files;
use crate::language::{Language, Options};

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
    /// The language is known but this `iohr` does not render it yet.
    #[error(
        "this iohr does not generate {0} yet; the languages it generates are listed by `iohr sdk generate --help`"
    )]
    NotBuilt(Language),
    /// The language is not one `iohr` knows (a lock written by a newer `iohr`).
    #[error("{0:?} is not a language this iohr knows; a newer iohr may")]
    UnknownLanguage(String),
}

/// One language. Adding a language is one implementation, one arm in
/// [`render`](crate::render), and its golden and compile tests.
pub trait Target {
    /// The language this target renders.
    const LANG: Language;

    /// Renders `api` into files relative to the output directory.
    ///
    /// # Errors
    ///
    /// [`RenderError`] when a model or a template cannot be rendered; never because of
    /// the file system, which [`Files::write`] handles.
    fn render(&self, api: &Api, options: &Options) -> Result<Files, RenderError>;
}
