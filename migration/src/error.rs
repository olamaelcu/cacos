use miette::Diagnostic;
use thiserror::Error;

#[derive(Error, Debug, Diagnostic)]
pub enum PdsError {
    #[error("Database error: {0}")]
    #[diagnostic(code(cacos::db_error))]
    Database(sea_orm::DbErr),

    #[error("Not found: {entity} with id {id}")]
    #[help("Check that the entity exists and the id is correct")]
    #[diagnostic(code(cacos::not_found))]
    NotFound { entity: String, id: String },

    #[error("Invalid input: {0}")]
    #[diagnostic(code(cacos::invalid_input))]
    InvalidInput(String),
}

impl From<sea_orm::DbErr> for PdsError {
    fn from(err: sea_orm::DbErr) -> Self {
        Self::Database(err)
    }
}

pub type Result<T> = miette::Result<T, PdsError>;
