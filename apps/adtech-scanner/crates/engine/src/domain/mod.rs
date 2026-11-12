//! Pure domain logic: no I/O, no async runtime, no storage or HTTP types.

pub mod chunks;
pub mod diagnostics;
pub mod error;
pub mod finding;
pub mod guide;
pub mod heuristics;
pub mod knowledge;
pub mod pricing;
pub mod questions;
pub mod scan;
pub mod scripts;
pub mod synthesis;
pub mod telemetry;
pub mod triage;

pub use error::DomainError;
