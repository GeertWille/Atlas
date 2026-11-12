//! Use cases and the scan pipeline. Depends on the domain and on ports only.

pub mod configurations;
pub mod error;
pub mod export;
pub mod judge;
pub mod llm;
pub mod pipeline;
pub mod ports;
pub mod scans;
pub mod settings;

pub use error::AppError;
