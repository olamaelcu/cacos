use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorKind {
    InvalidHandle,
    HandleNotAvailable,
    UnsupportedDomain,
    InternalError,
}

#[derive(Debug, Error)]
#[error("{kind:?}: {message}")]
pub struct Error {
    pub kind: ErrorKind,
    pub message: String,
}

impl Error {
    pub fn new(kind: ErrorKind, message: &str) -> Self {
        Self {
            kind,
            message: message.to_string(),
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;