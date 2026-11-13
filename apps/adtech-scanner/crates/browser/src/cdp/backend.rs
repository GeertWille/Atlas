//! Where the browser of a capture runs: Chrome on this machine, reached
//! over its debugging pipe, or a browser at Browserbase, reached over a
//! WebSocket. The capture talks to either through the same client; what
//! differs is how the browser starts, how it is reached and how it ends.

use std::time::Duration;

use serde_json::json;
use tokio::sync::mpsc::UnboundedReceiver;

use super::browserbase::{ApiFailure, RemoteBrowser, SessionState};
use super::{Cdp, CdpEvent, Chrome, LaunchError};

/// A browser on the kept browser profile is asked to close and given this
/// long to write its cookies before it is killed.
const CLOSE_PATIENCE: Duration = Duration::from_secs(5);

pub(crate) enum Backend {
    Local(Chrome),
    Browserbase(RemoteBrowser),
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ConnectFailure {
    #[error(transparent)]
    Local(#[from] LaunchError),
    #[error(transparent)]
    Browserbase(#[from] ApiFailure),
}

impl Backend {
    /// Connects to the browser and waits until it answers.
    pub(crate) async fn connect(
        &mut self,
    ) -> Result<(Cdp, UnboundedReceiver<CdpEvent>), ConnectFailure> {
        match self {
            Self::Local(chrome) => Ok(super::connect(chrome).await?),
            Self::Browserbase(browser) => Ok(browser.connect().await?),
        }
    }

    /// The Browserbase browser, when that is where the capture runs.
    pub(crate) fn remote(&self) -> Option<&RemoteBrowser> {
        match self {
            Self::Local(_) => None,
            Self::Browserbase(browser) => Some(browser),
        }
    }

    /// Ends the browser; it is gone when this returns. `cdp` is the
    /// connection to it, when one was made. For a Browserbase browser the
    /// answer is the session as the service says it ended, `None` when
    /// that could not be confirmed.
    pub(crate) async fn close(self, cdp: Option<&Cdp>) -> Option<SessionState> {
        match self {
            Self::Local(mut chrome) => {
                if let (Some(cdp), true) = (cdp, chrome.keeps_its_profile()) {
                    // Killed, the browser would lose the cookies it has not
                    // written yet; closing makes it write them.
                    cdp.notify(None, "Browser.close", json!({}));
                    chrome.wait_for_exit(CLOSE_PATIENCE).await;
                }
                chrome.shutdown().await;
                None
            }
            Self::Browserbase(browser) => browser.end(cdp).await,
        }
    }
}
