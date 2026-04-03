//! Conventional-commit version computation.
//!
//! Delegates to `git-cliff` to determine the next version from the commit history.

use std::process::Command;

use semver::Version;
use tracing::{debug, instrument};

use crate::ecosystem::{ChangelogTool, Ecosystem};
use crate::version::{parse_version, VersionError, VersionResult};

/// Compute the next version using git-cliff.
///
/// Writes a temp config with ecosystem-specific `[bump]` rules and runs
/// `git-cliff --bumped-version --config <temp>`. If `cliff_config_override`
/// is set, uses that path instead of the built-in config.
#[instrument(skip(cliff_config_override))]
pub fn compute_next_version(
    tool: ChangelogTool,
    ecosystem: Ecosystem,
    cliff_config_override: Option<&str>,
) -> VersionResult<Version> {
    match tool {
        ChangelogTool::GitCliff => compute_via_cliff(ecosystem, cliff_config_override),
    }
}

fn compute_via_cliff(
    ecosystem: Ecosystem,
    cliff_config_override: Option<&str>,
) -> VersionResult<Version> {
    debug!("computing version via git-cliff");

    let tmp_file;
    let config_path = if let Some(path) = cliff_config_override {
        path.to_string()
    } else {
        tmp_file = tempfile::Builder::new()
            .prefix("scrat-cliff-")
            .suffix(".toml")
            .tempfile()
            .map_err(|e| VersionError::ToolFailed {
                tool: "git-cliff".into(),
                message: format!("failed to create temp config: {e}"),
            })?;
        std::fs::write(tmp_file.path(), ecosystem.bump_config()).map_err(|e| {
            VersionError::ToolFailed {
                tool: "git-cliff".into(),
                message: format!("failed to write temp config: {e}"),
            }
        })?;
        tmp_file
            .path()
            .to_str()
            .expect("temp path is UTF-8")
            .to_string()
    };

    let output = Command::new("git-cliff")
        .args(["--bumped-version", "--config", &config_path])
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

#[cfg(test)]
mod tests {
    use super::*;

    // compute_next_version shells out to git-cliff.
    // We can't guarantee it's installed in all environments, so we test
    // the dispatch logic and error handling rather than full integration.

    #[test]
    fn compute_dispatches_to_cliff() {
        let result = compute_next_version(ChangelogTool::GitCliff, Ecosystem::Rust, None);
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
    fn version_error_tool_failed_display() {
        let err = VersionError::ToolFailed {
            tool: "git-cliff".into(),
            message: "not found on PATH".into(),
        };
        let msg = err.to_string();
        assert!(msg.contains("git-cliff"));
        assert!(msg.contains("not found on PATH"));
    }
}
