//! Why a capture ended without a result.

use crate::BrowserError;
use crate::cdp::{CdpError, LaunchError, PersistentProfileError};

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
    #[error("the page closed its own window while it was being scanned")]
    TabClosed,
    #[error("{0}")]
    Unsupported(&'static str),
}

impl From<CaptureError> for BrowserError {
    fn from(error: CaptureError) -> Self {
        match error {
            CaptureError::Cancelled => BrowserError::Cancelled,
            CaptureError::Launch(LaunchError::PersistentProfile(PersistentProfileError::Busy)) => {
                BrowserError::ProfileBusy
            }
            CaptureError::Launch(_) | CaptureError::NoPage => {
                BrowserError::Launch(error.to_string())
            }
            CaptureError::Navigation(failure) => BrowserError::Navigation(failure.to_string()),
            CaptureError::Protocol(_)
            | CaptureError::PageCrashed
            | CaptureError::TabClosed
            | CaptureError::Unsupported(_) => BrowserError::Other(error.to_string()),
        }
    }
}
