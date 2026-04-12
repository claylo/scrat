//! Rust ecosystem driver (Cargo.toml / Cargo.lock).
//!
//! Implements [`EcosystemDriver`](super::EcosystemDriver) for Rust projects.
//! All four trait methods (`parse_lockfile_diff`, `detect`,
//! `bump_version_files`, `check_registry_auth`) are present.

use std::process::Command;

use camino::Utf8Path;
use semver::Version;
use tracing::debug;

use super::{EcosystemDriver, emit_change, extract_toml_string_value};
use crate::bump::{BumpError, BumpResult};
use crate::detect::has_binary;
use crate::ecosystem::{DetectedTools, Ecosystem, ProjectDetection, VersionStrategy};
use crate::pipeline::DepChange;
use crate::preflight::CheckResult;

/// Rust ecosystem driver.
pub struct RustDriver;

impl EcosystemDriver for RustDriver {
    fn bump_version_files(
        &self,
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

    fn detect(
        &self,
        _project_root: &Utf8Path,
        version_strategy: VersionStrategy,
    ) -> ProjectDetection {
        let has_cargo = has_binary("cargo");
        let has_nextest = has_binary("cargo-nextest");
        let has_cargo_edit = has_binary("cargo-set-version");

        debug!(has_cargo, has_nextest, has_cargo_edit, "probed Rust tools");

        let test_cmd = if has_nextest {
            "cargo nextest run".into()
        } else if has_cargo {
            "cargo test".into()
        } else {
            String::new()
        };

        let bump_cmd = has_cargo_edit.then(|| "cargo set-version".to_string());
        let changelog_tool = version_strategy.changelog_tool();

        ProjectDetection {
            ecosystem: Ecosystem::Rust,
            version_strategy,
            tools: DetectedTools {
                test_cmd,
                build_cmd: "cargo build --release".into(),
                publish_cmd: has_cargo.then(|| "cargo publish".to_string()),
                bump_cmd,
                changelog_tool,
            },
        }
    }

    fn check_registry_auth(&self) -> CheckResult {
        let env_vars = ["CARGO_REGISTRY_TOKEN"];
        super::check_registry_auth_impl(
            &env_vars,
            "crates.io",
            "set CARGO_REGISTRY_TOKEN or run `cargo login`",
        )
    }

    /// Parse a unified diff of `Cargo.lock` into dependency changes.
    ///
    /// State machine tracking per-`[[package]]` blocks:
    /// - `name` from any `name = "..."` line (context, removed, or added)
    /// - `old_version` from `-version = "..."` lines
    /// - `new_version` from `+version = "..."` lines
    ///
    /// At each `[[package]]` boundary or EOF, emits a [`DepChange`] if
    /// we have a name and at least one version that changed.
    fn parse_lockfile_diff(&self, diff: &str) -> Vec<DepChange> {
        let mut changes: Vec<DepChange> = Vec::new();

        let mut current_name: Option<String> = None;
        let mut old_version: Option<String> = None;
        let mut new_version: Option<String> = None;

        for line in diff.lines() {
            // [[package]] boundary — any prefix (context, +, -)
            let trimmed = line
                .strip_prefix(' ')
                .or_else(|| line.strip_prefix('+'))
                .or_else(|| line.strip_prefix('-'))
                .unwrap_or(line);

            if trimmed.starts_with("[[package]]") {
                // Emit pending change from previous block
                emit_change(&mut changes, &current_name, &old_version, &new_version);
                current_name = None;
                old_version = None;
                new_version = None;
                continue;
            }

            // name = "..." — appears in context, removed, or added lines
            if let Some(name) = extract_toml_string_value(trimmed, "name") {
                current_name = Some(name);
                continue;
            }

            // -version = "..." — old version (removed line)
            if line.starts_with('-') {
                if let Some(ver) = extract_toml_string_value(trimmed, "version") {
                    old_version = Some(ver);
                }
                continue;
            }

            // +version = "..." — new version (added line)
            if line.starts_with('+')
                && let Some(ver) = extract_toml_string_value(trimmed, "version")
            {
                new_version = Some(ver);
            }
        }

        // Emit final pending block
        emit_change(&mut changes, &current_name, &old_version, &new_version);

        // Stable ordering
        changes.sort_by(|a, b| a.name.cmp(&b.name));
        changes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecosystem::ChangelogTool;
    use std::fs;
    use tempfile::TempDir;

    fn utf8_tmp(tmp: &TempDir) -> &Utf8Path {
        Utf8Path::from_path(tmp.path()).expect("tempdir is UTF-8")
    }

    #[test]
    fn rust_detection_basic() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("Cargo.toml"), "[package]").unwrap();

        let det = RustDriver.detect(utf8_tmp(&tmp), VersionStrategy::Interactive);
        assert_eq!(det.ecosystem, Ecosystem::Rust);
        assert_eq!(det.tools.build_cmd, "cargo build --release");
        // publish_cmd depends on whether cargo is on PATH in the test env
    }

    #[test]
    fn rust_changelog_tool_wired_from_strategy() {
        let tmp = TempDir::new().unwrap();
        let strategy = VersionStrategy::ConventionalCommits {
            tool: ChangelogTool::GitCliff,
        };
        let det = RustDriver.detect(utf8_tmp(&tmp), strategy);
        assert_eq!(det.tools.changelog_tool, Some(ChangelogTool::GitCliff));
    }

    #[test]
    fn rust_interactive_strategy_has_no_changelog_tool() {
        let tmp = TempDir::new().unwrap();
        let det = RustDriver.detect(utf8_tmp(&tmp), VersionStrategy::Interactive);
        assert_eq!(det.tools.changelog_tool, None);
    }

    #[test]
    fn parse_cargo_lock_diff_update() {
        let diff = r#"
 [[package]]
 name = "serde"
-version = "1.0.0"
+version = "1.0.1"
 source = "registry+https://github.com/rust-lang/crates.io-index"
"#;
        let changes = RustDriver.parse_lockfile_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "serde");
        assert_eq!(changes[0].from.as_deref(), Some("1.0.0"));
        assert_eq!(changes[0].to.as_deref(), Some("1.0.1"));
    }

    #[test]
    fn parse_cargo_lock_diff_added() {
        let diff = r#"
+[[package]]
+name = "new-crate"
+version = "0.1.0"
+source = "registry+https://github.com/rust-lang/crates.io-index"
"#;
        let changes = RustDriver.parse_lockfile_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "new-crate");
        assert_eq!(changes[0].from, None);
        assert_eq!(changes[0].to.as_deref(), Some("0.1.0"));
    }

    #[test]
    fn parse_cargo_lock_diff_removed() {
        let diff = r#"
-[[package]]
-name = "old-crate"
-version = "2.0.0"
-source = "registry+https://github.com/rust-lang/crates.io-index"
"#;
        let changes = RustDriver.parse_lockfile_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "old-crate");
        assert_eq!(changes[0].from.as_deref(), Some("2.0.0"));
        assert_eq!(changes[0].to, None);
    }

    #[test]
    fn parse_cargo_lock_diff_mixed() {
        let diff = r#"
 [[package]]
 name = "serde"
-version = "1.0.0"
+version = "1.0.1"
 source = "registry+https://github.com/rust-lang/crates.io-index"
+[[package]]
+name = "new-crate"
+version = "0.1.0"
+source = "registry+https://github.com/rust-lang/crates.io-index"
-[[package]]
-name = "old-crate"
-version = "2.0.0"
-source = "registry+https://github.com/rust-lang/crates.io-index"
"#;
        let changes = RustDriver.parse_lockfile_diff(diff);
        assert_eq!(changes.len(), 3);
        // Sorted by name
        assert_eq!(changes[0].name, "new-crate");
        assert_eq!(changes[1].name, "old-crate");
        assert_eq!(changes[2].name, "serde");
    }

    #[test]
    fn parse_cargo_lock_diff_empty() {
        let changes = RustDriver.parse_lockfile_diff("");
        assert!(changes.is_empty());
    }

    #[test]
    fn parse_cargo_lock_diff_no_version_change() {
        // A block where name appears but no version lines changed — no dep change
        let diff = r#"
 [[package]]
 name = "unchanged"
 version = "1.0.0"
 source = "registry+https://github.com/rust-lang/crates.io-index"
-dependencies = []
+dependencies = ["foo"]
"#;
        let changes = RustDriver.parse_lockfile_diff(diff);
        assert!(changes.is_empty());
    }

    #[test]
    fn parse_cargo_lock_diff_sorted() {
        let diff = r#"
 [[package]]
 name = "zebra"
-version = "1.0.0"
+version = "2.0.0"
 [[package]]
 name = "alpha"
-version = "0.1.0"
+version = "0.2.0"
 [[package]]
 name = "middle"
-version = "3.0.0"
+version = "3.1.0"
"#;
        let changes = RustDriver.parse_lockfile_diff(diff);
        assert_eq!(changes.len(), 3);
        assert_eq!(changes[0].name, "alpha");
        assert_eq!(changes[1].name, "middle");
        assert_eq!(changes[2].name, "zebra");
    }

    #[test]
    fn check_registry_auth_rust() {
        let result = RustDriver.check_registry_auth();
        assert_eq!(result.name, "Registry auth");
        if !result.passed {
            assert!(result.message.contains("CARGO_REGISTRY_TOKEN"));
            assert_eq!(result.skip_flag.as_deref(), Some("--no-publish"));
        }
    }
}
