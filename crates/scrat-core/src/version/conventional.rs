//! Conventional-commit version computation.
//!
//! Delegates to `git-cliff` or `cog` to determine the next version
//! from the commit history.

use std::process::Command;

use semver::Version;
use tracing::{debug, instrument};

use crate::ecosystem::ChangelogTool;
use crate::version::{VersionError, VersionResult, parse_version};

/// Compute the next version using a conventional-commit tool.
///
/// - **git-cliff**: runs `git cliff --bumped-version`
/// - **cog**: runs `cog bump --dry-run --auto`
#[instrument]
pub fn compute_next_version(tool: ChangelogTool) -> VersionResult<Version> {
    match tool {
        ChangelogTool::GitCliff => compute_via_cliff(),
        ChangelogTool::Cog => compute_via_cog(),
    }
}

fn compute_via_cliff() -> VersionResult<Version> {
    debug!("computing version via git-cliff");

    let output = Command::new("git-cliff")
        .arg("--bumped-version")
        .output()
        .map_err(|e| VersionError::ToolFailed {
            tool: "git-cliff".into(),
            message: format!("failed to execute: {e}"),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(VersionError::ToolFailed {
            tool: "git-cliff".into(),
            message: stderr,
        });
    }

    let version_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
    debug!(%version_str, "git-cliff suggested version");
    parse_version(&version_str)
}

fn compute_via_cog() -> VersionResult<Version> {
    debug!("computing version via cog");

    let output = Command::new("cog")
        .args(["bump", "--dry-run", "--auto"])
        .output()
        .map_err(|e| VersionError::ToolFailed {
            tool: "cog".into(),
            message: format!("failed to execute: {e}"),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(VersionError::ToolFailed {
            tool: "cog".into(),
            message: stderr,
        });
    }

    // cog outputs something like "1.2.3" on stdout
    let version_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
    debug!(%version_str, "cog suggested version");
    parse_version(&version_str)
}
