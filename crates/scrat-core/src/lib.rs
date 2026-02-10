//! Core library for scrat.
//!
//! This crate provides the foundational types and functionality used by the
//! `scrat` CLI and any downstream consumers.
//!
//! # Modules
//!
//! - [`bump`] - Version bump execution (file updates, changelog)
//! - [`config`] - Configuration loading and management
//! - [`deps`] - Dependency diff from lockfile changes
//! - [`detect`] - Project ecosystem and tool detection
//! - [`ecosystem`] - Ecosystem types and smart defaults
//! - [`error`] - Error types and result aliases
//! - [`git`] - Git operations for release workflows
//! - [`hooks`] - Hook executor for shell commands at phase boundaries
//! - [`pipeline`] - Pipeline context accumulator for structured release data
//! - [`preflight`] - Release readiness checks
//! - [`ship`] - Ship orchestrator (full release workflow)
//! - [`stats`] - Release statistics (commits, files changed, contributors)
//! - [`version`] - Version determination and computation
//!
//! # Quick Start
//!
//! ```no_run
//! use scrat_core::{Config, ConfigLoader};
//!
//! let config = ConfigLoader::new()
//!     .with_user_config(true)
//!     .load()
//!     .expect("Failed to load configuration");
//!
//! println!("Log level: {:?}", config.log_level);
//! ```
#![deny(unsafe_code)]

pub mod bump;

pub mod config;

pub mod deps;

pub mod detect;

pub mod ecosystem;

pub mod error;

pub mod git;

pub mod hooks;

pub mod pipeline;

pub mod preflight;

pub mod ship;

pub mod stats;

pub mod version;

pub use config::{Config, ConfigLoader, LogLevel};

pub use error::{ConfigError, ConfigResult};

// Re-export semver so downstream crates don't need a direct dependency.
pub use semver;
