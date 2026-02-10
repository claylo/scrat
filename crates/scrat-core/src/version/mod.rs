//! Version determination and computation.
//!
//! This module handles figuring out what the next version should be via
//! three strategies: conventional commits (auto), interactive (prompted),
//! and explicit (user-supplied).

pub mod conventional;
pub mod explicit;
pub mod interactive;

use semver::Version;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Errors from version operations.
#[derive(Error, Debug)]
pub enum VersionError {
    /// Failed to parse a semver string.
    #[error("invalid semver: {0}")]
    InvalidSemver(#[from] semver::Error),

    /// The conventional-commit tool failed.
    #[error("{tool} failed: {message}")]
    ToolFailed {
        /// Tool name (e.g., "git-cliff").
        tool: String,
        /// Error details.
        message: String,
    },

    /// No version tag found in the repository.
    #[error("no version tags found — is this the first release?")]
    NoTags,

    /// A git operation failed.
    #[error("git error: {0}")]
    Git(#[from] crate::git::GitError),
}

/// Result alias for version operations.
pub type VersionResult<T> = Result<T, VersionError>;

/// Semver bump level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BumpLevel {
    /// Patch release (x.y.Z).
    Patch,
    /// Minor release (x.Y.0).
    Minor,
    /// Major release (X.0.0).
    Major,
}

impl std::fmt::Display for BumpLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Patch => write!(f, "patch"),
            Self::Minor => write!(f, "minor"),
            Self::Major => write!(f, "major"),
        }
    }
}

/// Compute the next version by applying a bump level.
pub const fn next_version(current: &Version, level: BumpLevel) -> Version {
    match level {
        BumpLevel::Patch => Version::new(current.major, current.minor, current.patch + 1),
        BumpLevel::Minor => Version::new(current.major, current.minor + 1, 0),
        BumpLevel::Major => Version::new(current.major + 1, 0, 0),
    }
}

/// Parse a version string, stripping an optional `v` prefix.
pub fn parse_version(s: &str) -> VersionResult<Version> {
    let s = s.strip_prefix('v').unwrap_or(s);
    Ok(Version::parse(s)?)
}

/// Get the current version from git tags.
///
/// Returns `None` if no version tags exist (first release).
pub fn current_version_from_tags() -> VersionResult<Option<Version>> {
    let tag = crate::git::latest_version_tag()?;
    match tag {
        Some(t) => Ok(Some(parse_version(&t)?)),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bump_patch() {
        let v = Version::new(1, 2, 3);
        assert_eq!(next_version(&v, BumpLevel::Patch), Version::new(1, 2, 4));
    }

    #[test]
    fn bump_minor() {
        let v = Version::new(1, 2, 3);
        assert_eq!(next_version(&v, BumpLevel::Minor), Version::new(1, 3, 0));
    }

    #[test]
    fn bump_major() {
        let v = Version::new(1, 2, 3);
        assert_eq!(next_version(&v, BumpLevel::Major), Version::new(2, 0, 0));
    }

    #[test]
    fn parse_with_v_prefix() {
        assert_eq!(parse_version("v1.2.3").unwrap(), Version::new(1, 2, 3));
    }

    #[test]
    fn parse_without_v_prefix() {
        assert_eq!(parse_version("1.2.3").unwrap(), Version::new(1, 2, 3));
    }

    #[test]
    fn parse_invalid() {
        assert!(parse_version("not-a-version").is_err());
    }

    #[test]
    fn bump_from_zero() {
        let v = Version::new(0, 1, 0);
        assert_eq!(next_version(&v, BumpLevel::Patch), Version::new(0, 1, 1));
        assert_eq!(next_version(&v, BumpLevel::Minor), Version::new(0, 2, 0));
        assert_eq!(next_version(&v, BumpLevel::Major), Version::new(1, 0, 0));
    }
}
