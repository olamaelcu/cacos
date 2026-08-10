//! XRPC `ApiError` enum and its poem [`IntoResponse`] adapter.
//!
//! Ports the variant set from `olamaelcu/rsky` `src/apis/mod.rs` and maps
//! each variant onto a `{error, message}` JSON body and an HTTP status
//! code. Handler functions return [`ApiResult<T>`]; `?` converts anyhow
//! errors into [`ApiError::RuntimeError`].

use poem::http::StatusCode;
use poem::{IntoResponse, Response};
use serde::Serialize;

/// XRPC error variants surfaced by handler functions.
#[derive(Clone, Debug)]
pub enum ApiError {
    RuntimeError,
    InvalidLogin,
    AccountTakendown,
    InvalidRequest(String),
    ExpiredToken,
    InvalidToken,
    RecordNotFound,
    InvalidHandle,
    InvalidEmail,
    InvalidPassword,
    InvalidInviteCode,
    HandleNotAvailable,
    EmailNotAvailable,
    UnsupportedDomain,
    UnresolvableDid,
    IncompatibleDidDoc,
    WellKnownNotFound,
    AccountNotFound,
    BlobNotFound,
    BadRequest(String, String),
    AuthRequiredError(String),
    /// Error passed through from an upstream service: status code, error, message
    UpstreamResponse(u16, String, String),
}

/// JSON body shape for every XRPC error response.
#[derive(Serialize)]
pub struct ErrorBody {
    pub error: String,
    pub message: String,
}

impl ApiError {
    fn body(self) -> (StatusCode, ErrorBody) {
        match self {
            ApiError::RuntimeError => (
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorBody {
                    error: "InternalServerError".to_string(),
                    message: "Something went wrong".to_string(),
                },
            ),
            ApiError::InvalidLogin => (
                StatusCode::BAD_REQUEST,
                ErrorBody {
                    error: "InvalidLogin".to_string(),
                    message: "Invalid identifier or password".to_string(),
                },
            ),
            ApiError::AccountTakendown => (
                StatusCode::BAD_REQUEST,
                ErrorBody {
                    error: "AccountTakendown".to_string(),
                    message: "Account has been taken down".to_string(),
                },
            ),
            ApiError::InvalidRequest(message) => (
                StatusCode::BAD_REQUEST,
                ErrorBody {
                    error: "InvalidRequest".to_string(),
                    message,
                },
            ),
            ApiError::ExpiredToken => (
                StatusCode::BAD_REQUEST,
                ErrorBody {
                    error: "ExpiredToken".to_string(),
                    message: "Token is expired".to_string(),
                },
            ),
            ApiError::InvalidToken => (
                StatusCode::BAD_REQUEST,
                ErrorBody {
                    error: "InvalidToken".to_string(),
                    message: "Token is invalid".to_string(),
                },
            ),
            ApiError::InvalidHandle => (
                StatusCode::BAD_REQUEST,
                ErrorBody {
                    error: "InvalidHandle".to_string(),
                    message: "Handle is invalid".to_string(),
                },
            ),
            ApiError::InvalidEmail => (
                StatusCode::BAD_REQUEST,
                ErrorBody {
                    error: "InvalidEmail".to_string(),
                    message: "Invalid email".to_string(),
                },
            ),
            ApiError::InvalidPassword => (
                StatusCode::BAD_REQUEST,
                ErrorBody {
                    error: "InvalidPassword".to_string(),
                    message: "Invalid Password".to_string(),
                },
            ),
            ApiError::InvalidInviteCode => (
                StatusCode::BAD_REQUEST,
                ErrorBody {
                    error: "InvalidInviteCode".to_string(),
                    message: "Invalid invite code".to_string(),
                },
            ),
            ApiError::HandleNotAvailable => (
                StatusCode::BAD_REQUEST,
                ErrorBody {
                    error: "HandleNotAvailable".to_string(),
                    message: "Handle not available".to_string(),
                },
            ),
            ApiError::EmailNotAvailable => (
                StatusCode::BAD_REQUEST,
                ErrorBody {
                    error: "EmailNotAvailable".to_string(),
                    message: "Email not available".to_string(),
                },
            ),
            ApiError::UnsupportedDomain => (
                StatusCode::BAD_REQUEST,
                ErrorBody {
                    error: "UnsupportedDomain".to_string(),
                    message: "Unsupported domain".to_string(),
                },
            ),
            ApiError::UnresolvableDid => (
                StatusCode::BAD_REQUEST,
                ErrorBody {
                    error: "UnresolvableDid".to_string(),
                    message: "Unresolved Did".to_string(),
                },
            ),
            ApiError::IncompatibleDidDoc => (
                StatusCode::BAD_REQUEST,
                ErrorBody {
                    error: "IncompatibleDidDoc".to_string(),
                    message: "IncompatibleDidDoc".to_string(),
                },
            ),
            ApiError::AccountNotFound => (
                StatusCode::BAD_REQUEST,
                ErrorBody {
                    error: "AccountNotFound".to_string(),
                    message: "Account could not be found".to_string(),
                },
            ),
            ApiError::BlobNotFound => (
                StatusCode::BAD_REQUEST,
                ErrorBody {
                    error: "BlobNotFound".to_string(),
                    message: "Blob could not be found".to_string(),
                },
            ),
            ApiError::WellKnownNotFound => (
                StatusCode::NOT_FOUND,
                ErrorBody {
                    error: "WellKnownNotFound".to_string(),
                    message: "User not found".to_string(),
                },
            ),
            ApiError::BadRequest(error, message) => {
                (StatusCode::BAD_REQUEST, ErrorBody { error, message })
            }
            ApiError::AuthRequiredError(message) => (
                StatusCode::UNAUTHORIZED,
                ErrorBody {
                    error: "AuthRequiredError".to_string(),
                    message,
                },
            ),
            ApiError::UpstreamResponse(status, error, message) => (
                StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY),
                ErrorBody { error, message },
            ),
            ApiError::RecordNotFound => (
                StatusCode::NOT_FOUND,
                ErrorBody {
                    error: "RecordNotFound".to_string(),
                    message: "Record could not be found".to_string(),
                },
            ),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, body) = self.body();
        let bytes = match serde_json::to_vec(&body) {
            Ok(bytes) => bytes,
            Err(_) => {
                br#"{"error":"InternalServerError","message":"Something went wrong"}"#.to_vec()
            }
        };
        Response::builder()
            .status(status)
            .content_type("application/json")
            .body(bytes)
    }
}

impl From<ApiError> for poem::Error {
    fn from(err: ApiError) -> Self {
        poem::Error::from_response(err.into_response())
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(_value: anyhow::Error) -> Self {
        ApiError::RuntimeError
    }
}

pub type ApiResult<T> = Result<T, ApiError>;
