//! Why a capture ended without a result.

use crate::BrowserError;
use crate::cdp::{CdpError, LaunchError};

use super::navigation::NavigationFailure;

#[derive(Debug, thiserror::Error)]
pub(crate) enum CaptureError {
    #[error("the scan was cancelled")]
    Cancelled,
    #[error(transparent)]
    Launch(#[from] LaunchError),
    #[error(transparent)]
    Navigation(#[from] NavigationFailure),
    #[error(transparent)]
    Protocol(#[from] CdpError),
    #[error("the browser did not open a tab to scan")]
    NoPage,
    #[error("the page crashed while it was being scanned")]
    PageCrashed,
}

impl From<CaptureError> for BrowserError {
    fn from(error: CaptureError) -> Self {
        match error {
            CaptureError::Cancelled => BrowserError::Cancelled,
            CaptureError::Launch(_)
            | CaptureError::Protocol(CdpError::Connect(_))
            | CaptureError::NoPage => BrowserError::Launch(error.to_string()),
            CaptureError::Navigation(failure) => BrowserError::Navigation(failure.to_string()),
            CaptureError::Protocol(_) | CaptureError::PageCrashed => {
                BrowserError::Other(error.to_string())
            }
        }
    }
}
