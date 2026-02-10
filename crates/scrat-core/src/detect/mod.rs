//! Project detection — discover ecosystem, tools, and version strategy.
//!
//! Walks the working directory for marker files, probes `PATH` for available
//! tools, and checks for conventional-commit configuration to assemble a
//! [`ProjectDetection`](crate::ecosystem::ProjectDetection).
//!
//! # Example
//!
//! ```no_run
//! use camino::Utf8Path;
//! use scrat_core::detect;
//!
//! let project = detect::detect_project(Utf8Path::new("."));
//! match project {
//!     Some(det) => println!("Detected: {}", det.ecosystem),
//!     None => println!("Unknown project type"),
//! }
//! ```

mod rust;

use camino::Utf8Path;
use tracing::{debug, instrument};

use crate::ecosystem::{ChangelogTool, Ecosystem, ProjectDetection, VersionStrategy};

/// Detect the project ecosystem and available tooling from `project_root`.
///
/// Returns `None` if no recognized marker file is found.
#[instrument(fields(root = %project_root))]
pub fn detect_project(project_root: &Utf8Path) -> Option<ProjectDetection> {
    let ecosystem = detect_ecosystem(project_root)?;
    debug!(%ecosystem, "detected ecosystem");

    let version_strategy = detect_version_strategy(project_root);
    debug!(%version_strategy, "detected version strategy");

    let detection = match ecosystem {
        Ecosystem::Rust => rust::detect_rust(project_root, version_strategy),
        Ecosystem::Node => detect_node_stub(version_strategy),
    };

    Some(detection)
}

/// Identify the ecosystem by scanning for marker files.
fn detect_ecosystem(project_root: &Utf8Path) -> Option<Ecosystem> {
    for ecosystem in Ecosystem::ALL {
        let marker = project_root.join(ecosystem.marker_file());
        if marker.is_file() {
            return Some(*ecosystem);
        }
    }
    None
}

/// Determine the version strategy from config files in the project root.
///
/// Priority:
/// 1. `cliff.toml` → `ConventionalCommits(GitCliff)`
/// 2. `cog.toml`   → `ConventionalCommits(Cog)`
/// 3. Neither      → `Interactive`
fn detect_version_strategy(project_root: &Utf8Path) -> VersionStrategy {
    if project_root.join("cliff.toml").is_file() {
        debug!("found cliff.toml");
        return VersionStrategy::ConventionalCommits {
            tool: ChangelogTool::GitCliff,
        };
    }

    if project_root.join("cog.toml").is_file() {
        debug!("found cog.toml");
        return VersionStrategy::ConventionalCommits {
            tool: ChangelogTool::Cog,
        };
    }

    VersionStrategy::Interactive
}

/// Stub detection for Node ecosystem (future implementation).
fn detect_node_stub(version_strategy: VersionStrategy) -> ProjectDetection {
    use crate::ecosystem::DetectedTools;

    ProjectDetection {
        ecosystem: Ecosystem::Node,
        version_strategy,
        tools: DetectedTools {
            test_cmd: "npm test".into(),
            build_cmd: "npm run build".into(),
            publish_cmd: Some("npm publish".into()),
            bump_cmd: Some("npm version --no-git-tag-version".into()),
            changelog_tool: None,
        },
    }
}

/// Check whether a binary is available on `PATH`.
pub fn has_binary(name: &str) -> bool {
    which::which(name).is_ok()
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
    fn detect_rust_ecosystem() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("Cargo.toml"), "[package]\nname = \"test\"").unwrap();

        let detection = detect_project(utf8_tmp(&tmp));
        assert!(detection.is_some());
        let det = detection.unwrap();
        assert_eq!(det.ecosystem, Ecosystem::Rust);
    }

    #[test]
    fn detect_node_ecosystem() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("package.json"), "{}").unwrap();

        let detection = detect_project(utf8_tmp(&tmp));
        assert!(detection.is_some());
        let det = detection.unwrap();
        assert_eq!(det.ecosystem, Ecosystem::Node);
    }

    #[test]
    fn detect_unknown_ecosystem() {
        let tmp = TempDir::new().unwrap();
        // No marker files
        let detection = detect_project(utf8_tmp(&tmp));
        assert!(detection.is_none());
    }

    #[test]
    fn rust_takes_priority_over_node() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("Cargo.toml"), "[package]").unwrap();
        fs::write(tmp.path().join("package.json"), "{}").unwrap();

        let det = detect_project(utf8_tmp(&tmp)).unwrap();
        assert_eq!(det.ecosystem, Ecosystem::Rust);
    }

    #[test]
    fn detect_cc_strategy_cliff() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("Cargo.toml"), "[package]").unwrap();
        fs::write(tmp.path().join("cliff.toml"), "").unwrap();

        let det = detect_project(utf8_tmp(&tmp)).unwrap();
        assert_eq!(
            det.version_strategy,
            VersionStrategy::ConventionalCommits {
                tool: ChangelogTool::GitCliff
            }
        );
    }

    #[test]
    fn detect_cc_strategy_cog() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("Cargo.toml"), "[package]").unwrap();
        fs::write(tmp.path().join("cog.toml"), "").unwrap();

        let det = detect_project(utf8_tmp(&tmp)).unwrap();
        assert_eq!(
            det.version_strategy,
            VersionStrategy::ConventionalCommits {
                tool: ChangelogTool::Cog
            }
        );
    }

    #[test]
    fn cliff_takes_priority_over_cog() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("Cargo.toml"), "[package]").unwrap();
        fs::write(tmp.path().join("cliff.toml"), "").unwrap();
        fs::write(tmp.path().join("cog.toml"), "").unwrap();

        let det = detect_project(utf8_tmp(&tmp)).unwrap();
        assert!(matches!(
            det.version_strategy,
            VersionStrategy::ConventionalCommits {
                tool: ChangelogTool::GitCliff
            }
        ));
    }

    #[test]
    fn interactive_when_no_cc_config() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("Cargo.toml"), "[package]").unwrap();

        let det = detect_project(utf8_tmp(&tmp)).unwrap();
        assert_eq!(det.version_strategy, VersionStrategy::Interactive);
    }
}
