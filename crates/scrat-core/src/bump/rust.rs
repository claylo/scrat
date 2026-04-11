//! Rust ecosystem version bumping.
//!
//! Bumps `Cargo.toml` via whichever `bump_cmd` was detected (typically
//! `cargo set-version` from the `cargo-edit` extension). The tool
//! invocation and stderr propagation are handled here; `bump/mod.rs`
//! owns the dispatch and result aggregation.

use std::process::Command;

use camino::Utf8Path;
use semver::Version;
use tracing::debug;

use super::{BumpError, BumpResult};
use crate::ecosystem::ProjectDetection;

/// Bump the version in Cargo.toml using `cargo set-version`.
///
/// Returns the repo-relative path of the file that was updated.
pub(super) fn bump_rust_version(
    project_root: &Utf8Path,
    version: &Version,
    detection: &ProjectDetection,
) -> BumpResult<Vec<String>> {
    let Some(ref bump_cmd) = detection.tools.bump_cmd else {
        return Err(BumpError::NoBumpTool);
    };

    debug!(%bump_cmd, %version, "bumping Rust version");

    let parts: Vec<&str> = bump_cmd.split_whitespace().collect();
    let (bin, args) = parts.split_first().unwrap_or((&"cargo", &[]));

    let output = Command::new(bin)
        .args(args)
        .arg(version.to_string())
        .current_dir(project_root.as_std_path())
        .output()
        .map_err(|e| BumpError::ToolFailed {
            tool: bump_cmd.clone(),
            message: format!("failed to execute: {e}"),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(BumpError::ToolFailed {
            tool: bump_cmd.clone(),
            message: stderr,
        });
    }

    Ok(vec!["Cargo.toml".into()])
}
