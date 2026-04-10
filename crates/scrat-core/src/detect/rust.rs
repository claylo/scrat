//! Rust ecosystem detection.
//!
//! Probes `PATH` for Rust-specific tools and assembles smart defaults.

use camino::Utf8Path;
use tracing::debug;

use super::has_binary;
use crate::ecosystem::{DetectedTools, Ecosystem, ProjectDetection, VersionStrategy};

/// Detect Rust project tooling and build a [`ProjectDetection`].
pub(super) fn detect_rust(
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

        let det = detect_rust(utf8_tmp(&tmp), VersionStrategy::Interactive);
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
        let det = detect_rust(utf8_tmp(&tmp), strategy);
        assert_eq!(det.tools.changelog_tool, Some(ChangelogTool::GitCliff));
    }

    #[test]
    fn rust_interactive_strategy_has_no_changelog_tool() {
        let tmp = TempDir::new().unwrap();
        let det = detect_rust(utf8_tmp(&tmp), VersionStrategy::Interactive);
        assert_eq!(det.tools.changelog_tool, None);
    }
}
