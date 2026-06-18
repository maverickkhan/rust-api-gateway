//! Configuration errors.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to read config: {0}")]
    Io(String),
    #[error("failed to parse config: {0}")]
    Parse(String),
    #[error("invalid config: {0}")]
    Invalid(String),
}
