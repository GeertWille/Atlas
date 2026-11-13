//! Browser adapters and the DevTools protocol connection.
//!
//! `client` speaks the protocol over a transport that carries whole
//! messages. For Chrome on this machine, `chrome` owns the process and its
//! lifetime (with `profile` and `watchdog`), `pipe` frames the messages, and
//! they meet in [`connect`]. For a browser at Browserbase, `browserbase`
//! starts and ends it and `websocket` carries the messages. `backend` is
//! the choice between the two.

mod backend;
mod browserbase;
mod chrome;
mod client;
mod pipe;
mod profile;
mod watchdog;
mod websocket;

use serde_json::json;
use tokio::sync::mpsc::UnboundedReceiver;

pub(crate) use backend::{Backend, ConnectFailure};
pub(crate) use browserbase::{
    Api, ApiFailure, Exit, Project, RemoteBrowser, SessionOrder, SessionState, StartFailure,
    StartNote,
};
pub(crate) use chrome::{Chrome, LaunchError, LaunchOptions, ProfileChoice, ProxyServer};
#[cfg(test)]
pub(crate) use client::fake;
pub(crate) use client::{Cdp, CdpError, CdpEvent};
pub(crate) use profile::{
    PersistentProfileError, clear as clear_persistent_profile,
    inspect as inspect_persistent_profile, remove_abandoned as remove_abandoned_profiles,
};
#[cfg(test)]
pub(crate) use websocket::MAX_MESSAGE_BYTES;

/// Connects to a browser that was just spawned and waits until it answers.
pub(crate) async fn connect(
    chrome: &mut Chrome,
) -> Result<(Cdp, UnboundedReceiver<CdpEvent>), LaunchError> {
    let pipe = chrome
        .take_debugging_pipe()
        .ok_or_else(|| LaunchError::Unresponsive("it is already connected".to_string()))?;
    let (cdp, events) = Cdp::start(
        pipe::MessageReader::new(pipe.from_browser),
        pipe::MessageWriter::new(pipe.to_browser),
    );
    let ready = cdp.send(None, "Browser.getVersion", json!({}));
    match tokio::time::timeout(chrome::STARTUP_TIMEOUT, ready).await {
        Ok(Ok(_)) => Ok((cdp, events)),
        Ok(Err(CdpError::Closed)) => Err(LaunchError::Exited(chrome.startup_output().await)),
        Ok(Err(error)) => Err(LaunchError::Unresponsive(error.to_string())),
        Err(_) => Err(LaunchError::Timeout),
    }
}
