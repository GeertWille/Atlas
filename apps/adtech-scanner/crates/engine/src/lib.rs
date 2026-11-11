//! Scan engine for the Ad-Tech Agentic Scanner.
//!
//! Layered inward: `infrastructure` (SQLite, keychain, Anthropic HTTP, the
//! JavaScript beautifier) implements the ports declared by `application` (use
//! cases and the scan pipeline), which builds on the pure `domain`.

pub mod application;
pub mod domain;
pub mod infrastructure;
