//! Error types for the cacos PDS crate.
//!
//! NOTE on `anyhow`: This crate currently has `anyhow` as a direct dependency
//! to (a) name `anyhow::Result` in the `ReadableBlockstore` and `RepoStorage`
//! trait impl signatures (edition 2024 forbids referencing transitive-only
//! deps), and (b) hold the `anyhow::Error` source inside the single `Internal`
//! variant below for wrapping anyhow-returning calls
//! (`Repo::format_commit`, `blocks_to_car_file`, `BlockMap::get_many`).
//!
//! `anyhow` is contained here in exactly one place. The intent is to remove
//! this dependency entirely once the upstream protocol crates return a proper
//! error type; that removal should be done in a SEPARATE BRANCH. The upstream
//! crates themselves carry `@TODO: Remove anyhow in lib` comments — this is a
//! known migration target.

use miette::Diagnostic;
use thiserror::Error;

#[derive(Debug, Error, Diagnostic)]
pub enum PdsError {
    #[error("database error: {0}")]
    #[diagnostic(code(cacos::pds::database))]
    Database(sea_orm::DbErr),

    #[error("{reason}")]
    #[diagnostic(code(cacos::pds::internal))]
    Internal {
        reason: String,
        #[source]
        source: anyhow::Error,
    },

    #[error("not found: {0}")]
    #[diagnostic(code(cacos::pds::not_found))]
    NotFound(String),

    #[error("invalid input: {0}")]
    #[diagnostic(code(cacos::pds::invalid_input))]
    InvalidInput(String),
}

impl PdsError {
    /// Wrap an anyhow-returning error from a downstream crate into our root
    /// error with a reason string.
    pub fn internal(reason: impl Into<String>, source: impl Into<anyhow::Error>) -> Self {
        Self::Internal {
            reason: reason.into(),
            source: source.into(),
        }
    }
}

impl From<sea_orm::DbErr> for PdsError {
    fn from(err: sea_orm::DbErr) -> Self {
        Self::Database(err)
    }
}

impl From<migration::error::PdsError> for PdsError {
    fn from(err: migration::error::PdsError) -> Self {
        match err {
            migration::error::PdsError::Database(e) => Self::Database(e),
            other => Self::internal("migration error", anyhow::anyhow!(other.to_string())),
        }
    }
}

/// Auto-convert anyhow::Error into our `Internal` variant so `?` works
/// directly. The site that produces the anyhow error is still the source of
/// truth for context (via #[source]).
impl From<anyhow::Error> for PdsError {
    fn from(err: anyhow::Error) -> Self {
        Self::internal("wrapped anyhow error", err)
    }
}

/// Convert `cid::Error` (from lexicon_cid's `Cid::from_str`) into our
/// `Internal` variant. The site of the parse failure is the source via
/// `#[source]`.
impl From<lexicon_cid::Error> for PdsError {
    fn from(err: lexicon_cid::Error) -> Self {
        Self::internal("cid parse error", anyhow::Error::from(err))
    }
}

pub type Result<T> = miette::Result<T, PdsError>;
