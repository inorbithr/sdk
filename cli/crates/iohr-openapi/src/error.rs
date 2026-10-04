/// Why a document could not be normalised. The messages are those of
/// `tools/spec-sync.py`, so the two tools fail the same way on the same input.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum NormaliseError {
    /// The document is not OpenAPI 3.1.
    #[error("expected OpenAPI 3.1, got {0:?}")]
    Version(String),
    /// No operation carries `x-iohr-public: true`.
    #[error("the document marks no operation x-iohr-public")]
    NothingPublic,
    /// The document has no `paths` with an operation in them.
    #[error("the document has no operation")]
    NoOperation,
    /// A `$ref` points at a schema the document does not define.
    #[error("$ref to a schema the document does not define: {0}")]
    DanglingRef(String),
    /// Two schemas shorten to the same name and the package does not tell them apart.
    #[error("N1: {a} and {b} both become {short}")]
    NameClash {
        /// One original name.
        a: String,
        /// The other.
        b: String,
        /// The short name both would get.
        short: String,
    },
    /// The error envelope's schemas are missing, or `Code` states no status table that
    /// matches its codes.
    #[error("N6: {0}")]
    Problem(String),
    /// A fact the platform states itself since core #218 is missing from the document:
    /// a regression upstream, reported instead of patched over.
    #[error("{rule}: {detail}")]
    Regressed {
        /// The former rule (`N2`, `N3`, `N4`).
        rule: String,
        /// What is missing.
        detail: String,
    },
}

/// Why documents could not be modelled as one API.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ModelError {
    /// A document could not be normalised first.
    #[error(transparent)]
    Normalise(#[from] NormaliseError),
    /// Two documents define the same schema differently.
    #[error(
        "the documents of profiles {a} and {b} define schema {schema} differently; regenerate them from the same API version"
    )]
    SchemaConflict {
        /// One profile.
        a: String,
        /// The other.
        b: String,
        /// The schema's short name.
        schema: String,
    },
    /// Two documents define the same operation differently.
    #[error(
        "the documents of profiles {a} and {b} define {method} {path} differently; regenerate them from the same API version"
    )]
    OperationConflict {
        /// One profile.
        a: String,
        /// The other.
        b: String,
        /// The method.
        method: String,
        /// The path.
        path: String,
    },
    /// An operation has no usable id.
    #[error("{method} {path} has no operationId")]
    NoOperationId {
        /// The method.
        method: String,
        /// The path.
        path: String,
    },
    /// The profile's document holds nothing the profile may call.
    #[error(
        "the document of profile {profile} holds no operation: its credential may call nothing, so there is nothing to generate"
    )]
    NothingToCall {
        /// The profile.
        profile: String,
    },
    /// No document was given.
    #[error("no document to generate from")]
    NoDocuments,
    /// Two profile names map to the same environment variables (`acme-ci` and `acme_ci`).
    #[error("profiles {a} and {b} would read the same INORBIT_{env}_* variables; rename one")]
    AmbiguousProfiles {
        /// One name.
        a: String,
        /// The other.
        b: String,
        /// The shared environment name.
        env: String,
    },
}
