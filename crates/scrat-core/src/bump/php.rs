//! PHP / Composer ecosystem version bumping.
//!
//! Edits `composer.json` directly if and only if a `"version"` field
//! already exists. Composer does not require a version field at the
//! package level — most packages rely on git tags — so absence is
//! treated as a silent skip, not an error.

use camino::Utf8Path;
use semver::Version;
use tracing::debug;

use super::{BumpError, BumpResult};

/// Bump the version in `composer.json` if it has a `"version"` field.
///
/// Returns the repo-relative path of the file that was updated, or an
/// empty vec if `composer.json` is missing or has no `version` field.
pub(super) fn bump_composer_version(
    project_root: &Utf8Path,
    version: &Version,
) -> BumpResult<Vec<String>> {
    let composer_path = project_root.join("composer.json");
    let content = match std::fs::read_to_string(&composer_path) {
        Ok(c) => c,
        Err(_) => return Ok(vec![]),
    };

    let mut parsed: serde_json::Value =
        serde_json::from_str(&content).map_err(|e| BumpError::ToolFailed {
            tool: "composer.json".into(),
            message: format!("failed to parse: {e}"),
        })?;

    // Only write if the field already exists — don't add it if absent
    if parsed.get("version").and_then(|v| v.as_str()).is_none() {
        return Ok(vec![]);
    }

    parsed["version"] = serde_json::Value::String(version.to_string());

    let output = serde_json::to_string_pretty(&parsed).map_err(|e| BumpError::ToolFailed {
        tool: "composer.json".into(),
        message: format!("failed to serialize: {e}"),
    })?;

    // Composer convention: trailing newline
    std::fs::write(&composer_path, format!("{output}\n")).map_err(|e| BumpError::ToolFailed {
        tool: "composer.json".into(),
        message: format!("failed to write: {e}"),
    })?;

    debug!(%version, "bumped composer.json version");
    Ok(vec!["composer.json".into()])
}
