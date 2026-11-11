use adscan_model::ErrorCode;

use super::llm::LlmError;
use super::ports::{CaptureError, RepositoryError, SecretError};
use crate::domain::DomainError;

/// Failure of a use case, in terms the UI can show.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum AppError {
    #[error("{0}")]
    InvalidInput(String),
    #[error("{0} was not found")]
    NotFound(String),
    #[error("No Chrome or Chromium installation was found. Install Chrome or set its path in Settings.")]
    BrowserNotFound,
    #[error("{0}")]
    BrowserFailed(String),
    #[error("No Anthropic API key is configured")]
    ApiKeyMissing,
    #[error("The Anthropic API key was rejected: {0}")]
    ApiKeyInvalid(String),
    #[error("The AI request failed: {0}")]
    Llm(String),
    #[error("Local storage failed: {0}")]
    Storage(String),
    #[error("The scan was cancelled")]
    Cancelled,
    #[error("{0}")]
    Internal(String),
}

impl AppError {
    pub fn code(&self) -> ErrorCode {
        match self {
            Self::InvalidInput(_) => ErrorCode::InvalidInput,
            Self::NotFound(_) => ErrorCode::NotFound,
            Self::BrowserNotFound => ErrorCode::BrowserNotFound,
            Self::BrowserFailed(_) => ErrorCode::BrowserFailed,
            Self::ApiKeyMissing => ErrorCode::ApiKeyMissing,
            Self::ApiKeyInvalid(_) => ErrorCode::ApiKeyInvalid,
            Self::Llm(_) => ErrorCode::LlmFailed,
            Self::Storage(_) => ErrorCode::Storage,
            Self::Cancelled => ErrorCode::Cancelled,
            Self::Internal(_) => ErrorCode::Internal,
        }
    }
}

impl From<DomainError> for AppError {
    fn from(error: DomainError) -> Self {
        Self::InvalidInput(error.to_string())
    }
}

impl From<RepositoryError> for AppError {
    fn from(error: RepositoryError) -> Self {
        Self::Storage(error.0)
    }
}

impl From<SecretError> for AppError {
    fn from(error: SecretError) -> Self {
        Self::Internal(error.to_string())
    }
}

impl From<CaptureError> for AppError {
    fn from(error: CaptureError) -> Self {
        match error {
            CaptureError::BrowserNotFound => Self::BrowserNotFound,
            CaptureError::Cancelled => Self::Cancelled,
            failure @ (CaptureError::Launch(_) | CaptureError::Navigation(_) | CaptureError::Other(_)) => {
                Self::BrowserFailed(failure.to_string())
            }
        }
    }
}

impl From<LlmError> for AppError {
    fn from(error: LlmError) -> Self {
        match error {
            LlmError::ApiKeyInvalid(message) => Self::ApiKeyInvalid(message),
            LlmError::Cancelled => Self::Cancelled,
            other => Self::Llm(other.to_string()),
        }
    }
}
