use adscan_model::ErrorCode;

use super::llm::LlmError;
use super::ports::{BrowserbaseError, CaptureError, RepositoryError, SecretError};
use crate::domain::DomainError;

/// Failure of a use case, in terms the UI can show.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum AppError {
    #[error("{0}")]
    InvalidInput(String),
    #[error("{0} was not found")]
    NotFound(String),
    #[error(
        "No Chrome or Chromium installation was found. Install Chrome or set its path in Settings."
    )]
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
    #[error(
        "The browser profile is in use by a running scan. Try again when that scan has finished."
    )]
    ProfileBusy,
    /// Browserbase is not set up, or refused the key or the project.
    #[error("{0}")]
    BrowserbaseAuth(String),
    #[error("{0}")]
    BrowserbaseLimit(String),
    #[error("{0}")]
    BrowserbaseFailed(String),
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
            Self::ProfileBusy => ErrorCode::ProfileBusy,
            Self::BrowserbaseAuth(_) => ErrorCode::BrowserbaseAuth,
            Self::BrowserbaseLimit(_) => ErrorCode::BrowserbaseLimit,
            Self::BrowserbaseFailed(_) => ErrorCode::BrowserbaseFailed,
            Self::Internal(_) => ErrorCode::Internal,
        }
    }
}

impl From<DomainError> for AppError {
    fn from(error: DomainError) -> Self {
        match error {
            DomainError::BrowserbaseNotSetUp => Self::BrowserbaseAuth(error.to_string()),
            other => Self::InvalidInput(other.to_string()),
        }
    }
}

impl From<BrowserbaseError> for AppError {
    fn from(error: BrowserbaseError) -> Self {
        match error {
            BrowserbaseError::Auth(message) => Self::BrowserbaseAuth(message),
            BrowserbaseError::Limit(message) => Self::BrowserbaseLimit(message),
            BrowserbaseError::Failed(message) => Self::BrowserbaseFailed(message),
        }
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
            CaptureError::ProfileBusy => Self::ProfileBusy,
            CaptureError::RemoteAuth(message) => Self::BrowserbaseAuth(message),
            CaptureError::RemoteLimit(message) => Self::BrowserbaseLimit(message),
            CaptureError::Remote(message) => Self::BrowserbaseFailed(message),
            failure @ (CaptureError::Launch(_)
            | CaptureError::Navigation(_)
            | CaptureError::Other(_)) => Self::BrowserFailed(failure.to_string()),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browserbase_failures_keep_their_kind_and_their_message_up_to_the_command_layer() {
        let cases = [
            (
                AppError::from(CaptureError::RemoteAuth("key refused".into())),
                ErrorCode::BrowserbaseAuth,
                "key refused",
            ),
            (
                AppError::from(CaptureError::RemoteLimit("at the limit".into())),
                ErrorCode::BrowserbaseLimit,
                "at the limit",
            ),
            (
                AppError::from(CaptureError::Remote("connection lost".into())),
                ErrorCode::BrowserbaseFailed,
                "connection lost",
            ),
            (
                AppError::from(BrowserbaseError::Auth("project not found".into())),
                ErrorCode::BrowserbaseAuth,
                "project not found",
            ),
            (
                AppError::from(BrowserbaseError::Limit("too many".into())),
                ErrorCode::BrowserbaseLimit,
                "too many",
            ),
            (
                AppError::from(BrowserbaseError::Failed("unreachable".into())),
                ErrorCode::BrowserbaseFailed,
                "unreachable",
            ),
        ];
        for (error, code, message) in cases {
            assert_eq!(error.code(), code);
            assert_eq!(error.to_string(), message);
        }
    }

    #[test]
    fn browserbase_without_credentials_is_an_authentication_problem_not_bad_input() {
        let error = AppError::from(DomainError::BrowserbaseNotSetUp);
        assert_eq!(error.code(), ErrorCode::BrowserbaseAuth);
        assert_eq!(
            error.to_string(),
            "Browserbase is not set up. Add the API key in Settings."
        );
        assert_eq!(
            AppError::from(DomainError::NotReachableFromCloud).code(),
            ErrorCode::InvalidInput
        );
    }

    #[test]
    fn a_cancelled_remote_capture_is_still_a_cancellation() {
        assert_eq!(AppError::from(CaptureError::Cancelled), AppError::Cancelled);
    }
}
