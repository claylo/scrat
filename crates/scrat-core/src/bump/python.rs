//! Python ecosystem version bumping.
//!
//! Edits `pyproject.toml` directly when a `version` field exists under
//! the `[project]` table. Absence is treated as a silent skip — a
//! `pyproject.toml` without a `[project] version` field is valid (the
//! version may come from a dynamic source like `setuptools-scm`).

use camino::Utf8Path;
use semver::Version;
use tracing::debug;

use super::{BumpError, BumpResult};

/// Bump the version in `pyproject.toml` if it has a `version` field under `[project]`.
///
/// Returns the repo-relative path of the file that was updated, or an
/// empty vec if `pyproject.toml` is missing or has no `[project] version` field.
pub(super) fn bump_pyproject_version(
    project_root: &Utf8Path,
    version: &Version,
) -> BumpResult<Vec<String>> {
    let pyproject_path = project_root.join("pyproject.toml");
    let content = match std::fs::read_to_string(&pyproject_path) {
        Ok(c) => c,
        Err(_) => return Ok(vec![]),
    };

    // Look for `version = "..."` under `[project]` section
    let mut in_project = false;
    let mut found = false;
    let mut lines: Vec<String> = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_project = trimmed == "[project]";
        }
        if in_project
            && trimmed.starts_with("version")
            && let Some((key, _)) = trimmed.split_once('=')
            && key.trim() == "version"
        {
            lines.push(format!("version = \"{version}\""));
            found = true;
        } else {
            lines.push(line.to_string());
        }
    }

    if !found {
        return Ok(vec![]);
    }

    std::fs::write(&pyproject_path, lines.join("\n") + "\n").map_err(|e| {
        BumpError::ToolFailed {
            tool: "pyproject.toml".into(),
            message: format!("failed to write: {e}"),
        }
    })?;

    debug!(%version, "bumped pyproject.toml version");
    Ok(vec!["pyproject.toml".into()])
}
