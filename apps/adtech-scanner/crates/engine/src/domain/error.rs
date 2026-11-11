#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum DomainError {
    #[error("Enter a valid web address, for example https://example.com")]
    InvalidUrl,
    #[error("Only http and https pages can be scanned")]
    UnsupportedScheme,
    #[error("Observation time must be between {min} and {max} seconds")]
    ObserveSecondsOutOfRange { min: u32, max: u32 },
    #[error("The per-scan budget must be between $0 and ${max}")]
    BudgetOutOfRange { max: f64 },
    #[error("Model id must not be empty")]
    EmptyModelId,
}
