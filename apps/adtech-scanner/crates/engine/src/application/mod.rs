//! Use cases and the scan pipeline. Depends on the domain and on ports only.

pub mod error;
pub mod export;
pub mod llm;
pub mod pipeline;
pub mod ports;
pub mod scans;
pub mod settings;

pub use error::AppError;
