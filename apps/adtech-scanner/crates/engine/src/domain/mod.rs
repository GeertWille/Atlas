//! Pure domain logic: no I/O, no async runtime, no storage or HTTP types.

pub mod error;
pub mod finding;
pub mod heuristics;
pub mod knowledge;
pub mod pricing;
pub mod scan;
pub mod scripts;
pub mod synthesis;
pub mod telemetry;

pub use error::DomainError;
