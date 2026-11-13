//! Chrome adapter: the browser process and the DevTools protocol connection.
//!
//! `chrome` owns the process and its lifetime (with `profile` and `watchdog`),
//! `client` speaks the protocol, `pipe` frames its messages. They meet in
//! [`connect`].

mod chrome;
mod client;
mod pipe;
mod profile;
mod watchdog;

use serde_json::json;
use tokio::sync::mpsc::UnboundedReceiver;

pub(crate) use chrome::{Chrome, LaunchError, LaunchOptions, ProfileChoice, ProxyServer};
#[cfg(test)]
pub(crate) use client::fake;
pub(crate) use client::{Cdp, CdpError, CdpEvent};
pub(crate) use profile::{
    PersistentProfileError, clear as clear_persistent_profile,
    inspect as inspect_persistent_profile, remove_abandoned as remove_abandoned_profiles,
};

/// Connects to a browser that was just spawned and waits until it answers.
pub(crate) async fn connect(
    chrome: &mut Chrome,
) -> Result<(Cdp, UnboundedReceiver<CdpEvent>), LaunchError> {
    let pipe = chrome
        .take_debugging_pipe()
        .ok_or_else(|| LaunchError::Unresponsive("it is already connected".to_string()))?;
    let (cdp, events) = Cdp::start(pipe.from_browser, pipe.to_browser);
    let ready = cdp.send(None, "Browser.getVersion", json!({}));
    match tokio::time::timeout(chrome::STARTUP_TIMEOUT, ready).await {
        Ok(Ok(_)) => Ok((cdp, events)),
        Ok(Err(CdpError::Closed)) => Err(LaunchError::Exited(chrome.startup_output().await)),
        Ok(Err(error)) => Err(LaunchError::Unresponsive(error.to_string())),
        Err(_) => Err(LaunchError::Timeout),
    }
}
