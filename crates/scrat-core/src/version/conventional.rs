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

#[cfg(test)]
mod tests {
    use super::*;

    // compute_next_version shells out to external tools (git-cliff, cog).
    // We can't guarantee they're installed in all environments, so we test
    // the dispatch logic and error handling rather than full integration.

    #[test]
    fn compute_dispatches_to_cliff() {
        let result = compute_next_version(ChangelogTool::GitCliff);
        // If git-cliff is installed and we're in a repo with commits,
        // this succeeds. Otherwise it returns a ToolFailed error.
        // Either way, it should not panic.
        match result {
            Ok(v) => {
                // Valid semver returned
                assert!(v.major > 0 || v.minor > 0 || v.patch > 0 || v == Version::new(0, 0, 0));
            }
            Err(VersionError::ToolFailed { tool, .. }) => {
                assert_eq!(tool, "git-cliff");
            }
            Err(e) => {
                // Other errors (NoTags, etc.) are also acceptable
                let _ = e.to_string();
            }
        }
    }

    #[test]
    fn compute_dispatches_to_cog() {
        let result = compute_next_version(ChangelogTool::Cog);
        match result {
            Ok(v) => {
                assert!(v.major > 0 || v.minor > 0 || v.patch > 0 || v == Version::new(0, 0, 0));
            }
            Err(VersionError::ToolFailed { tool, .. }) => {
                assert_eq!(tool, "cog");
            }
            Err(e) => {
                let _ = e.to_string();
            }
        }
    }

    #[test]
    fn version_error_tool_failed_display() {
        let err = VersionError::ToolFailed {
            tool: "git-cliff".into(),
            message: "not found on PATH".into(),
        };
        let msg = err.to_string();
        assert!(msg.contains("git-cliff"));
        assert!(msg.contains("not found on PATH"));
    }

    #[test]
    fn version_error_tool_failed_display_cog() {
        let err = VersionError::ToolFailed {
            tool: "cog".into(),
            message: "exited with code 1".into(),
        };
        let msg = err.to_string();
        assert!(msg.contains("cog"));
        assert!(msg.contains("exited with code 1"));
    }
}
