//! Init command — project discovery and config file generation.

use camino::Utf8Path;
use serde::Serialize;
use tracing::debug;

use crate::config;
use crate::detect;
use crate::ecosystem::{ChangelogTool, Ecosystem, VersionStrategy};
use crate::git;

/// Result of project discovery — what scrat detected about the project.
///
/// Returned by [`plan_init`] so the CLI can present findings to the user
/// and collect their preferences before generating a config file.
#[derive(Debug, Clone, Serialize)]
pub struct InitPlan {
    /// Detected ecosystem (Rust, Node, etc.) or `None` if unrecognized.
    pub ecosystem: Option<Ecosystem>,
    /// Detected main/master branch, if any.
    pub release_branch: Option<String>,
    /// How version bumps will be determined.
    pub version_strategy: VersionStrategy,
    /// Changelog tool extracted from `version_strategy`, if applicable.
    pub changelog_tool: Option<ChangelogTool>,
    /// Path to an existing scrat config file, if found.
    pub existing_config: Option<String>,
}

/// Output format for the generated config file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, clap::ValueEnum)]
pub enum ConfigFormat {
    /// TOML format (default).
    #[default]
    Toml,
    /// YAML format.
    Yaml,
}

/// How much commentary to include in the generated config.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, clap::ValueEnum)]
pub enum ConfigStyle {
    /// Full config with section dividers and commented-out defaults.
    #[default]
    Documented,
    /// Bare minimum — only active settings.
    Minimal,
}

/// User's confirmed choices for config generation.
///
/// Built by the CLI after presenting the [`InitPlan`] and collecting
/// user preferences. Passed to [`generate_config`] to produce the file.
#[derive(Debug, Clone, Serialize)]
pub struct InitSelections {
    /// Output format.
    pub format: ConfigFormat,
    /// How verbose the generated file should be.
    pub style: ConfigStyle,
    /// Ecosystem to set in config, or `None` to omit.
    pub ecosystem: Option<Ecosystem>,
    /// Release branch to set, or `None` to omit.
    pub release_branch: Option<String>,
    /// Whether to enable GitHub releases.
    pub github_release: bool,
    /// Whether to create releases as drafts.
    pub draft: bool,
}

/// Discover the project and return an [`InitPlan`] for the CLI to present.
///
/// Runs detection for ecosystem, version strategy, release branch, and
/// existing config — all non-interactive. The CLI decides what to show
/// and how to prompt based on the returned plan.
pub fn plan_init(project_root: &Utf8Path) -> InitPlan {
    let detection = detect::detect_project(project_root);
    let ecosystem = detection.as_ref().map(|d| d.ecosystem);
    debug!(?ecosystem, "init: detected ecosystem");

    let version_strategy = detect::detect_version_strategy(project_root);
    debug!(%version_strategy, "init: detected version strategy");

    let changelog_tool = match &version_strategy {
        VersionStrategy::ConventionalCommits { tool } => Some(*tool),
        _ => None,
    };

    let release_branch = git::detect_release_branch().ok().flatten();
    debug!(?release_branch, "init: detected release branch");

    let existing_config = config::find_project_config(project_root).map(|p| p.to_string());
    debug!(?existing_config, "init: existing config");

    InitPlan {
        ecosystem,
        release_branch,
        version_strategy,
        changelog_tool,
        existing_config,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use camino::Utf8Path;
    use std::fs;
    use tempfile::TempDir;

    fn utf8_tmp(tmp: &TempDir) -> &Utf8Path {
        Utf8Path::from_path(tmp.path()).expect("tempdir is UTF-8")
    }

    #[test]
    fn plan_init_detects_rust_ecosystem() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("Cargo.toml"), "[package]\nname = \"test\"").unwrap();

        let plan = plan_init(utf8_tmp(&tmp));
        assert_eq!(plan.ecosystem, Some(Ecosystem::Rust));
    }

    #[test]
    fn plan_init_detects_node_ecosystem() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("package.json"), "{}").unwrap();

        let plan = plan_init(utf8_tmp(&tmp));
        assert_eq!(plan.ecosystem, Some(Ecosystem::Node));
    }

    #[test]
    fn plan_init_no_ecosystem_detected() {
        let tmp = TempDir::new().unwrap();

        let plan = plan_init(utf8_tmp(&tmp));
        assert_eq!(plan.ecosystem, None);
    }

    #[test]
    fn plan_init_detects_cliff_toml() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("Cargo.toml"), "[package]\nname = \"test\"").unwrap();
        fs::write(tmp.path().join("cliff.toml"), "").unwrap();

        let plan = plan_init(utf8_tmp(&tmp));
        assert_eq!(
            plan.version_strategy,
            VersionStrategy::ConventionalCommits {
                tool: ChangelogTool::GitCliff
            }
        );
        assert_eq!(plan.changelog_tool, Some(ChangelogTool::GitCliff));
    }

    #[test]
    fn plan_init_finds_existing_config() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("scrat.toml"), "").unwrap();

        let plan = plan_init(utf8_tmp(&tmp));
        assert!(plan.existing_config.is_some());
    }

    #[test]
    fn plan_init_no_existing_config() {
        let tmp = TempDir::new().unwrap();

        let plan = plan_init(utf8_tmp(&tmp));
        assert!(plan.existing_config.is_none());
    }
}
