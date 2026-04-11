//! Node.js ecosystem version bumping.
//!
//! Edits `package.json` directly. scrat is intentionally *not* a
//! lockfile manager — `package-lock.json` / `yarn.lock` / `pnpm-lock.yaml`
//! sync is the user's package manager's job.

use camino::Utf8Path;
use semver::Version;
use tracing::debug;

use super::{BumpError, BumpResult};

/// Bump the version in `package.json` directly.
///
/// scrat edits only `package.json` — it is intentionally *not* a
/// lockfile manager. If the user needs `package-lock.json` (or
/// `yarn.lock`, `pnpm-lock.yaml`) synced after the bump, that's their
/// package manager's job (e.g. a pre-commit scrat hook running
/// `npm install --package-lock-only`).
///
/// Returns the repo-relative path of the file that was updated.
pub(super) fn bump_node_version(
    project_root: &Utf8Path,
    version: &Version,
) -> BumpResult<Vec<String>> {
    let package_path = project_root.join("package.json");
    let content = std::fs::read_to_string(&package_path).map_err(|e| BumpError::ToolFailed {
        tool: "package.json".into(),
        message: format!("failed to read: {e}"),
    })?;

    let mut parsed: serde_json::Value =
        serde_json::from_str(&content).map_err(|e| BumpError::ToolFailed {
            tool: "package.json".into(),
            message: format!("failed to parse: {e}"),
        })?;

    if parsed.get("version").and_then(|v| v.as_str()).is_none() {
        return Err(BumpError::ToolFailed {
            tool: "package.json".into(),
            message: "no `version` field found — cannot bump".into(),
        });
    }

    parsed["version"] = serde_json::Value::String(version.to_string());

    // npm convention: 2-space indent, trailing newline
    let output = serde_json::to_string_pretty(&parsed).map_err(|e| BumpError::ToolFailed {
        tool: "package.json".into(),
        message: format!("failed to serialize: {e}"),
    })?;

    std::fs::write(&package_path, format!("{output}\n")).map_err(|e| BumpError::ToolFailed {
        tool: "package.json".into(),
        message: format!("failed to write: {e}"),
    })?;

    debug!(%version, "bumped package.json version");
    Ok(vec!["package.json".into()])
}
