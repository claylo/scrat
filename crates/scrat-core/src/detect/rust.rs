//! Rust ecosystem detection.
//!
//! Probes `PATH` for Rust-specific tools and assembles smart defaults.

use camino::Utf8Path;
use tracing::debug;

use super::has_binary;
use crate::ecosystem::{
    ChangelogTool, DetectedTools, Ecosystem, ProjectDetection, VersionStrategy,
};

/// Detect Rust project tooling and build a [`ProjectDetection`].
pub(super) fn detect_rust(
    project_root: &Utf8Path,
    version_strategy: VersionStrategy,
) -> ProjectDetection {
    let has_nextest = has_binary("cargo-nextest");
    let has_cargo_edit = has_binary("cargo-set-version");

    debug!(has_nextest, has_cargo_edit, "probed Rust tools");

    let test_cmd = if has_nextest {
        "cargo nextest run".into()
    } else {
        "cargo test".into()
    };

    let bump_cmd = if has_cargo_edit {
        Some("cargo set-version".into())
    } else {
        None
    };

    let changelog_tool = detect_changelog_tool(project_root);

    ProjectDetection {
        ecosystem: Ecosystem::Rust,
        version_strategy,
        tools: DetectedTools {
            test_cmd,
            build_cmd: "cargo build --release".into(),
            publish_cmd: Some("cargo publish".into()),
            bump_cmd,
            changelog_tool,
        },
    }
}

/// Check which changelog tool is configured for this Rust project.
fn detect_changelog_tool(project_root: &Utf8Path) -> Option<ChangelogTool> {
    if project_root.join("cliff.toml").is_file() {
        Some(ChangelogTool::GitCliff)
    } else if project_root.join("cog.toml").is_file() {
        Some(ChangelogTool::Cog)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        assert_eq!(det.tools.publish_cmd.as_deref(), Some("cargo publish"));
    }

    #[test]
    fn rust_changelog_tool_cliff() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("cliff.toml"), "").unwrap();

        assert_eq!(
            detect_changelog_tool(utf8_tmp(&tmp)),
            Some(ChangelogTool::GitCliff)
        );
    }

    #[test]
    fn rust_changelog_tool_cog() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("cog.toml"), "").unwrap();

        assert_eq!(
            detect_changelog_tool(utf8_tmp(&tmp)),
            Some(ChangelogTool::Cog)
        );
    }

    #[test]
    fn rust_no_changelog_tool() {
        let tmp = TempDir::new().unwrap();
        assert_eq!(detect_changelog_tool(utf8_tmp(&tmp)), None);
    }
}
