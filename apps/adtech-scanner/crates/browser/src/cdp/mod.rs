//! Chrome adapter: the browser process and the DevTools protocol connection.

mod chrome;
mod client;

pub(crate) use chrome::{Chrome, LaunchError, LaunchOptions};
pub(crate) use client::{Cdp, CdpError, CdpEvent};
