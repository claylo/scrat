//! Generic ecosystem driver.
//!
//! A first-class driver with no-op/empty implementations of every
//! [`EcosystemDriver`] method. Generic is selected interactively
//! when auto-detection finds no marker files, or via
//! `project.type = "generic"` in config. It skips ecosystem-specific
//! behavior but still participates in changelog, git commit/tag/push,
//! GitHub release, and hook execution.
//!
//! Call sites must NOT match on `Ecosystem::Generic` to skip the
//! driver — they trust the no-op bodies here. This is the pattern
//! that makes `ecosystem.driver().method(...)` dispatch uniform
//! across all variants.

use camino::Utf8Path;
use semver::Version;

use super::EcosystemDriver;
use crate::bump::BumpResult;
use crate::ecosystem::{ProjectDetection, VersionStrategy};
use crate::pipeline::DepChange;
use crate::preflight::CheckResult;

/// Generic ecosystem driver.
pub struct GenericDriver;

impl EcosystemDriver for GenericDriver {
    fn bump_version_files(
        &self,
        _project_root: &Utf8Path,
        _version: &Version,
        _detection: &ProjectDetection,
    ) -> BumpResult<Vec<String>> {
        tracing::debug!("generic ecosystem — no project files to bump");
        Ok(Vec::new())
    }

    fn detect(
        &self,
        _project_root: &Utf8Path,
        version_strategy: VersionStrategy,
    ) -> ProjectDetection {
        ProjectDetection::generic(version_strategy)
    }

    /// No lockfile — returns an empty `Vec`.
    fn parse_lockfile_diff(&self, _diff: &str) -> Vec<DepChange> {
        Vec::new()
    }

    fn check_registry_auth(&self) -> CheckResult {
        CheckResult {
            name: "Registry auth".into(),
            passed: true,
            message: "No registry publish for this ecosystem".into(),
            skip_flag: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecosystem::Ecosystem;

    #[test]
    fn generic_parse_lockfile_diff_returns_empty() {
        let changes = GenericDriver.parse_lockfile_diff("any input");
        assert!(changes.is_empty());
    }

    #[test]
    fn generic_detect_returns_generic_project_detection() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = camino::Utf8Path::from_path(tmp.path()).unwrap();
        let detection = GenericDriver.detect(root, VersionStrategy::Interactive);
        assert_eq!(detection.ecosystem, Ecosystem::Generic);
        assert_eq!(detection.version_strategy, VersionStrategy::Interactive);
        assert_eq!(detection.tools.test_cmd, "");
        assert_eq!(detection.tools.build_cmd, "");
        assert!(detection.tools.publish_cmd.is_none());
    }

    #[test]
    fn generic_bump_version_files_returns_empty() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = camino::Utf8Path::from_path(tmp.path()).unwrap();
        let detection = ProjectDetection::generic(VersionStrategy::Interactive);
        let files = GenericDriver
            .bump_version_files(root, &semver::Version::new(1, 0, 0), &detection)
            .unwrap();
        assert!(files.is_empty());
    }

    #[test]
    fn generic_check_registry_auth_returns_no_registry() {
        let result = GenericDriver.check_registry_auth();
        assert!(result.passed);
        assert_eq!(result.message, "No registry publish for this ecosystem");
        assert!(result.skip_flag.is_none());
    }

    #[test]
    fn check_registry_auth_generic_skips() {
        let result = GenericDriver.check_registry_auth();
        assert!(result.passed);
    }
}
