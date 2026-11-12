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
    #[error("A browse visit can follow links to at most {max} further pages")]
    ExtraPagesOutOfRange { max: u32 },
    #[error("Give the proxy a name of at most {max} characters")]
    ProxyNameInvalid { max: usize },
    #[error("A proxy named \"{0}\" already exists")]
    ProxyNameTaken(String),
    #[error(
        "Enter the proxy's host name or IP address only, without scheme, port, path or credentials"
    )]
    ProxyHostInvalid,
    #[error("The proxy port must be between 1 and 65535")]
    ProxyPortInvalid,
    #[error(
        "Chrome cannot sign in to a SOCKS5 proxy. Remove the user name or use an http or https proxy."
    )]
    ProxySocksCredentials,
    #[error("The chosen proxy no longer exists. Pick another proxy.")]
    UnknownProxy,
    #[error("Give the scan configuration a name of at most {max} characters")]
    ConfigurationNameInvalid { max: usize },
    #[error("A scan configuration named \"{0}\" already exists")]
    ConfigurationNameTaken(String),
    #[error("The storage limit for scan data must be between {min_mb} MB and {max_gb} GB")]
    ArchiveLimitOutOfRange { min_mb: u32, max_gb: u32 },
}
