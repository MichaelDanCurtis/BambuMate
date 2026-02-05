use thiserror::Error;

#[derive(Debug, Error)]
pub enum BambuMateError {
    #[error("Keychain error: {0}")]
    Keychain(String),

    #[error("Config error: {0}")]
    Config(String),

    #[error("Health check error: {0}")]
    HealthCheck(String),

    #[error("Profile error: {0}")]
    Profile(String),
}

impl From<BambuMateError> for String {
    fn from(err: BambuMateError) -> Self {
        err.to_string()
    }
}
