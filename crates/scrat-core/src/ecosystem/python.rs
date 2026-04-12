//! Python ecosystem driver (`pyproject.toml` / `uv.lock`).
//!
//! `parse_lockfile_diff` delegates literally to
//! [`super::rust::RustDriver`] because `uv.lock` currently uses the
//! same TOML `[[package]]` format as `Cargo.lock`. This is NOT a
//! commitment to a shared "TOML package diff" abstraction — it's an
//! incidental format match. If uv diverges from Cargo's lockfile
//! format in a future release, this module grows its own state
//! machine and stops delegating. Do NOT extract a shared
//! TOML-package-diff helper on the assumption that Python and Rust
//! will always share an implementation.

use camino::Utf8Path;
use semver::Version;
use tracing::debug;

use super::EcosystemDriver;
use crate::bump::{BumpError, BumpResult};
use crate::detect::has_binary;
use crate::ecosystem::{DetectedTools, Ecosystem, ProjectDetection, VersionStrategy};
use crate::pipeline::DepChange;
use crate::preflight::CheckResult;

/// Python ecosystem driver.
pub struct PythonDriver;

impl EcosystemDriver for PythonDriver {
    fn bump_version_files(
        &self,
        project_root: &Utf8Path,
        version: &Version,
        _detection: &ProjectDetection,
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

    fn detect(
        &self,
        _project_root: &Utf8Path,
        version_strategy: VersionStrategy,
    ) -> ProjectDetection {
        let has_uv = has_binary("uv");
        let has_pytest = has_binary("pytest");
        let has_python = has_binary("python3") || has_binary("python");
        let has_twine = has_binary("twine");
        debug!(
            has_uv,
            has_pytest, has_python, has_twine, "probed Python tools"
        );

        let test_cmd = if has_uv {
            "uv run pytest".into()
        } else if has_pytest {
            "pytest".into()
        } else {
            String::new()
        };
        let build_cmd = if has_uv {
            "uv build".into()
        } else if has_python {
            "python -m build".into()
        } else {
            String::new()
        };
        let publish_cmd = if has_uv {
            Some("uv publish".into())
        } else if has_twine {
            Some("twine upload dist/*".into())
        } else {
            None
        };

        let changelog_tool = version_strategy.changelog_tool();

        ProjectDetection {
            ecosystem: Ecosystem::Python,
            version_strategy,
            tools: DetectedTools {
                test_cmd,
                build_cmd,
                publish_cmd,
                bump_cmd: None, // Python bump is done directly in pyproject.toml
                changelog_tool,
            },
        }
    }

    fn check_registry_auth(&self) -> CheckResult {
        let env_vars = ["TWINE_PASSWORD", "PYPI_TOKEN"];
        super::check_registry_auth_impl(&env_vars, "PyPI", "set TWINE_PASSWORD or PYPI_TOKEN")
    }

    /// Delegates to [`super::rust::RustDriver::parse_lockfile_diff`]
    /// because `uv.lock` uses the same TOML `[[package]]` format as
    /// `Cargo.lock`. See the module-level doc comment for the
    /// landmine this comment protects against.
    fn parse_lockfile_diff(&self, diff: &str) -> Vec<DepChange> {
        super::rust::RustDriver.parse_lockfile_diff(diff)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_uv_lock_diff_update() {
        // Identical to Cargo.lock format
        let diff = r#"
 [[package]]
 name = "requests"
-version = "2.31.0"
+version = "2.32.0"
 source = { registry = "https://pypi.org/simple" }
"#;
        let changes = PythonDriver.parse_lockfile_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "requests");
        assert_eq!(changes[0].from.as_deref(), Some("2.31.0"));
        assert_eq!(changes[0].to.as_deref(), Some("2.32.0"));
    }

    #[test]
    fn parse_uv_lock_diff_added() {
        let diff = r#"
+[[package]]
+name = "new-dep"
+version = "1.0.0"
"#;
        let changes = PythonDriver.parse_lockfile_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].from, None);
        assert_eq!(changes[0].to.as_deref(), Some("1.0.0"));
    }

    #[test]
    fn parse_uv_lock_diff_skips_header() {
        // uv.lock has file-level version/requires-python before [[package]]
        let diff = r#"
-version = 1
+version = 2
 requires-python = ">=3.14"
 [[package]]
 name = "foo"
-version = "1.0.0"
+version = "1.1.0"
"#;
        let changes = PythonDriver.parse_lockfile_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "foo");
    }

    #[test]
    fn check_registry_auth_python() {
        let result = PythonDriver.check_registry_auth();
        assert_eq!(result.name, "Registry auth");
        if !result.passed {
            assert!(
                result.message.contains("TWINE_PASSWORD") || result.message.contains("PYPI_TOKEN")
            );
        }
    }
}
