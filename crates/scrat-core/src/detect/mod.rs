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

use std::process::Command;

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

/// Minimum required version of git-cliff.
///
/// 2.5.0 introduced `--bump [major|minor|patch]` which we need for
/// forced bump type. Earlier features we rely on (`--bumped-version`,
/// `--prepend`, `--with-commit`, `--context`, `--strip`,
/// `[bump]` config, `initial_tag`, `--with-tag-message`) all landed
/// in 2.4.0 or earlier.
pub const MIN_GIT_CLIFF_VERSION: semver::Version = semver::Version::new(2, 5, 0);

/// Result of a tool version check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolVersionCheck {
    /// Tool meets the minimum version.
    Ok(semver::Version),
    /// Tool is too old.
    TooOld {
        /// The version that was found.
        found: semver::Version,
        /// The minimum required version.
        minimum: semver::Version,
    },
    /// Could not determine the version (binary missing, parse failure, etc.).
    Unknown(String),
}

/// Check the installed version of a CLI tool.
///
/// Runs `<binary> --version`, parses the semver from its output, and
/// compares against `minimum`. Output is expected to match the pattern
/// `<name> X.Y.Z` (e.g. `git-cliff 2.12.0`).
pub fn check_tool_version(binary: &str, minimum: &semver::Version) -> ToolVersionCheck {
    let output = match Command::new(binary).arg("--version").output() {
        Ok(o) if o.status.success() => o,
        Ok(o) => {
            return ToolVersionCheck::Unknown(format!(
                "`{binary} --version` exited with {}",
                o.status,
            ));
        }
        Err(e) => {
            return ToolVersionCheck::Unknown(format!(
                "failed to run `{binary} --version`: {e}",
            ));
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let Some(version) = parse_version_from_output(&stdout) else {
        return ToolVersionCheck::Unknown(format!(
            "could not parse version from `{binary} --version` output: {stdout}",
        ));
    };

    if version >= *minimum {
        ToolVersionCheck::Ok(version)
    } else {
        ToolVersionCheck::TooOld {
            found: version,
            minimum: minimum.clone(),
        }
    }
}

/// Extract a semver version from tool output like `"git-cliff 2.12.0\n"`.
///
/// Scans for the first token that parses as a valid semver version.
fn parse_version_from_output(output: &str) -> Option<semver::Version> {
    output
        .split_whitespace()
        .find_map(|token| semver::Version::parse(token).ok())
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
    fn parse_version_from_git_cliff_output() {
        let v = parse_version_from_output("git-cliff 2.12.0\n");
        assert_eq!(v, Some(semver::Version::new(2, 12, 0)));
    }

    #[test]
    fn parse_version_from_minimal_output() {
        let v = parse_version_from_output("2.5.0");
        assert_eq!(v, Some(semver::Version::new(2, 5, 0)));
    }

    #[test]
    fn parse_version_from_garbage() {
        assert!(parse_version_from_output("not a version").is_none());
        assert!(parse_version_from_output("").is_none());
    }

    #[test]
    fn parse_version_with_prerelease() {
        let v = parse_version_from_output("tool 3.0.0-rc.1");
        assert_eq!(
            v,
            Some(semver::Version::parse("3.0.0-rc.1").unwrap())
        );
    }

    #[test]
    fn tool_version_check_too_old() {
        // Simulate: we have 1.0.0 but need 2.5.0
        let found = semver::Version::new(1, 0, 0);
        let minimum = semver::Version::new(2, 5, 0);
        assert!(found < minimum);
    }

    #[test]
    fn min_git_cliff_version_is_correct() {
        assert_eq!(MIN_GIT_CLIFF_VERSION, semver::Version::new(2, 5, 0));
    }

    #[test]
    fn interactive_when_no_cc_config() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("Cargo.toml"), "[package]").unwrap();

        let det = detect_project(utf8_tmp(&tmp)).unwrap();
        assert_eq!(det.version_strategy, VersionStrategy::Interactive);
    }
}
