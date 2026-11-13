//! Browserbase adapter: a browser in Browserbase's cloud, started and
//! ended through its REST API and driven over one WebSocket.
//!
//! `order` says what is asked for (pure), `failure` what went wrong in
//! words for the user (pure), `api` makes the calls, and `session` owns
//! one browser from the request for it to the confirmation that it ended.

mod api;
mod failure;
mod order;
mod session;
mod time;

pub(crate) use api::{Api, Project, SessionState};
pub(crate) use failure::ApiFailure;
pub(crate) use order::{Exit, SessionOrder};
pub(crate) use session::{RemoteBrowser, StartFailure};
