//! Ecosystem types and smart defaults for release workflows.
//!
//! This module defines the project ecosystem types (Rust, Node, etc.) and
//! the associated tool/command defaults. Detection logic lives in the CLI
//! crate (`detect` module) — this module is pure types and data.

use serde::{Deserialize, Serialize};
use std::fmt;

/// A recognized project ecosystem.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Ecosystem {
    /// Rust project (detected via `Cargo.toml`).
    Rust,
    /// Node.js project (detected via `package.json`).
    Node,
}

impl fmt::Display for Ecosystem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rust => write!(f, "rust"),
            Self::Node => write!(f, "node"),
        }
    }
}

impl Ecosystem {
    /// Filename that signals this ecosystem when found in a directory.
    pub const fn marker_file(self) -> &'static str {
        match self {
            Self::Rust => "Cargo.toml",
            Self::Node => "package.json",
        }
    }

    /// Primary lockfile for this ecosystem, relative to project root.
    pub const fn lockfile_path(self) -> &'static str {
        match self {
            Self::Rust => "Cargo.lock",
            Self::Node => "package-lock.json",
        }
    }

    /// All recognized ecosystems, in detection priority order.
    pub const ALL: &[Self] = &[Self::Rust, Self::Node];
}

/// Version-determination strategy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum VersionStrategy {
    /// Auto-compute from conventional commits via a changelog tool.
    ConventionalCommits {
        /// Which tool drives the CC analysis.
        tool: ChangelogTool,
    },
    /// Interactive semver picker (show recent commits, prompt user).
    Interactive,
    /// Explicit version passed on the CLI (e.g., `--version v1.2.3`).
    Explicit(String),
}

impl fmt::Display for VersionStrategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ConventionalCommits { tool } => write!(f, "conventional-commits ({tool})"),
            Self::Interactive => write!(f, "interactive"),
            Self::Explicit(v) => write!(f, "explicit ({v})"),
        }
    }
}

/// Changelog generation tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ChangelogTool {
    /// [git-cliff](https://git-cliff.org/) — template-driven changelogs.
    GitCliff,
    /// [cocogitto](https://docs.cocogitto.io/) — conventional-commit tooling.
    Cog,
}

impl fmt::Display for ChangelogTool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GitCliff => write!(f, "git-cliff"),
            Self::Cog => write!(f, "cog"),
        }
    }
}

/// Tools detected on `PATH` and their resolved commands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetectedTools {
    /// Command to run tests (e.g. `"cargo nextest run"` or `"cargo test"`).
    pub test_cmd: String,
    /// Command to build a release artifact.
    pub build_cmd: String,
    /// Command to publish to a registry (`None` if not applicable).
    pub publish_cmd: Option<String>,
    /// Command to bump the version in project files (`None` if manual).
    pub bump_cmd: Option<String>,
    /// Changelog tool, if one is configured.
    pub changelog_tool: Option<ChangelogTool>,
}

/// Full detection result for a project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectDetection {
    /// The detected ecosystem.
    pub ecosystem: Ecosystem,
    /// How to determine the next version.
    pub version_strategy: VersionStrategy,
    /// Resolved tool commands.
    pub tools: DetectedTools,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ecosystem_display() {
        assert_eq!(Ecosystem::Rust.to_string(), "rust");
        assert_eq!(Ecosystem::Node.to_string(), "node");
    }

    #[test]
    fn ecosystem_marker_files() {
        assert_eq!(Ecosystem::Rust.marker_file(), "Cargo.toml");
        assert_eq!(Ecosystem::Node.marker_file(), "package.json");
    }

    #[test]
    fn version_strategy_display() {
        let cc = VersionStrategy::ConventionalCommits {
            tool: ChangelogTool::GitCliff,
        };
        assert_eq!(cc.to_string(), "conventional-commits (git-cliff)");
        assert_eq!(VersionStrategy::Interactive.to_string(), "interactive");
        assert_eq!(
            VersionStrategy::Explicit("v1.0.0".into()).to_string(),
            "explicit (v1.0.0)"
        );
    }

    #[test]
    fn changelog_tool_display() {
        assert_eq!(ChangelogTool::GitCliff.to_string(), "git-cliff");
        assert_eq!(ChangelogTool::Cog.to_string(), "cog");
    }

    #[test]
    fn lockfile_paths() {
        assert_eq!(Ecosystem::Rust.lockfile_path(), "Cargo.lock");
        assert_eq!(Ecosystem::Node.lockfile_path(), "package-lock.json");
    }

    #[test]
    fn serde_roundtrip_ecosystem() {
        let json = serde_json::to_string(&Ecosystem::Rust).unwrap();
        assert_eq!(json, "\"rust\"");
        let parsed: Ecosystem = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, Ecosystem::Rust);
    }

    #[test]
    fn serde_roundtrip_version_strategy() {
        let strategy = VersionStrategy::ConventionalCommits {
            tool: ChangelogTool::Cog,
        };
        let json = serde_json::to_string(&strategy).unwrap();
        let parsed: VersionStrategy = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, strategy);
    }
}
