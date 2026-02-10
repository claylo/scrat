//! Explicit version mode — validate and parse a user-supplied version.

use semver::Version;
use tracing::{debug, instrument};

use crate::version::{VersionResult, parse_version};

/// Validate and parse an explicit version string.
///
/// Accepts `"1.2.3"` or `"v1.2.3"` formats.
#[instrument]
pub fn validate_explicit(version_str: &str) -> VersionResult<Version> {
    let version = parse_version(version_str)?;
    debug!(%version, "validated explicit version");
    Ok(version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_explicit() {
        assert_eq!(validate_explicit("1.2.3").unwrap(), Version::new(1, 2, 3));
    }

    #[test]
    fn valid_with_prefix() {
        assert_eq!(validate_explicit("v2.0.0").unwrap(), Version::new(2, 0, 0));
    }

    #[test]
    fn invalid_explicit() {
        assert!(validate_explicit("not-semver").is_err());
    }
}
