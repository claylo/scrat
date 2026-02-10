//! Error types for scrat-core

use thiserror::Error;

/// Errors that can occur when working with configuration.
#[derive(Error, Debug)]
pub enum ConfigError {
    /// Failed to deserialize configuration.
    #[error("invalid configuration: {0}")]
    Deserialize(#[from] Box<figment::Error>),

    /// Configuration file not found after searching all locations.
    #[error("no configuration file found")]
    NotFound,
}

/// Result type alias using [`ConfigError`].
pub type ConfigResult<T> = Result<T, ConfigError>;
