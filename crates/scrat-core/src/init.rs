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

// ─── Template constants ────────────────────────────────────────────────────

const TEMPLATE_DOCUMENTED_TOML: &str =
    include_str!("../templates/init-documented.toml");
const TEMPLATE_DOCUMENTED_YAML: &str =
    include_str!("../templates/init-documented.yaml");
const TEMPLATE_MINIMAL_TOML: &str =
    include_str!("../templates/init-minimal.toml");
const TEMPLATE_MINIMAL_YAML: &str =
    include_str!("../templates/init-minimal.yaml");

/// Generate a config file from user selections.
///
/// Picks the right template based on `(style, format)`, fills in section
/// placeholders, and cleans up blank lines.
pub fn generate_config(selections: &InitSelections) -> String {
    let template = match (selections.style, selections.format) {
        (ConfigStyle::Documented, ConfigFormat::Toml) => TEMPLATE_DOCUMENTED_TOML,
        (ConfigStyle::Documented, ConfigFormat::Yaml) => TEMPLATE_DOCUMENTED_YAML,
        (ConfigStyle::Minimal, ConfigFormat::Toml) => TEMPLATE_MINIMAL_TOML,
        (ConfigStyle::Minimal, ConfigFormat::Yaml) => TEMPLATE_MINIMAL_YAML,
    };

    let project = build_project_section(selections);
    let commands = build_commands_section(selections);
    let release = build_release_section(selections);

    let output = template
        .replace("{{project_section}}", &project)
        .replace("{{commands_section}}", &commands)
        .replace("{{release_section}}", &release);

    cleanup_blank_lines(&output)
}

/// Build the `[project]` / `project:` section.
fn build_project_section(selections: &InitSelections) -> String {
    let Some(ecosystem) = selections.ecosystem else {
        return String::new();
    };

    match selections.format {
        ConfigFormat::Toml => {
            let mut lines = vec!["[project]".to_string()];
            lines.push(format!("type = \"{ecosystem}\""));
            match &selections.release_branch {
                Some(branch) => lines.push(format!("release_branch = \"{branch}\"")),
                None if selections.style == ConfigStyle::Documented => {
                    lines.push("# release_branch = \"main\"".to_string());
                }
                None => {}
            }
            lines.join("\n")
        }
        ConfigFormat::Yaml => {
            let mut lines = vec!["project:".to_string()];
            lines.push(format!("  type: {ecosystem}"));
            match &selections.release_branch {
                Some(branch) => lines.push(format!("  release_branch: \"{branch}\"")),
                None if selections.style == ConfigStyle::Documented => {
                    lines.push("  # release_branch: main".to_string());
                }
                None => {}
            }
            lines.join("\n")
        }
    }
}

/// Build the `[commands]` / `commands:` section.
fn build_commands_section(selections: &InitSelections) -> String {
    if selections.style == ConfigStyle::Minimal {
        return String::new();
    }

    let ecosystem = selections.ecosystem.unwrap_or(Ecosystem::Generic);

    match selections.format {
        ConfigFormat::Toml => {
            let mut lines = vec!["# [commands]".to_string()];
            match ecosystem {
                Ecosystem::Rust => {
                    lines.push("# test = \"cargo nextest run\"".to_string());
                    lines.push("# build = \"cargo build --release\"".to_string());
                    lines.push("# publish = \"cargo publish\"".to_string());
                }
                Ecosystem::Node => {
                    lines.push("# test = \"npm test\"".to_string());
                    lines.push("# build = \"npm run build\"".to_string());
                    lines.push("# publish = \"npm publish\"".to_string());
                }
                Ecosystem::Generic => {
                    lines.push("# test = \"make test\"".to_string());
                    lines.push("# build = \"make build\"".to_string());
                }
            }
            lines.join("\n")
        }
        ConfigFormat::Yaml => {
            let mut lines = vec!["# commands:".to_string()];
            match ecosystem {
                Ecosystem::Rust => {
                    lines.push("#   test: cargo nextest run".to_string());
                    lines.push("#   build: cargo build --release".to_string());
                    lines.push("#   publish: cargo publish".to_string());
                }
                Ecosystem::Node => {
                    lines.push("#   test: npm test".to_string());
                    lines.push("#   build: npm run build".to_string());
                    lines.push("#   publish: npm publish".to_string());
                }
                Ecosystem::Generic => {
                    lines.push("#   test: make test".to_string());
                    lines.push("#   build: make build".to_string());
                }
            }
            lines.join("\n")
        }
    }
}

/// Build the `[release]` / `release:` section.
fn build_release_section(selections: &InitSelections) -> String {
    match selections.format {
        ConfigFormat::Toml => {
            let mut lines = vec!["[release]".to_string()];
            lines.push(format!("github_release = {}", selections.github_release));
            lines.push(format!("draft = {}", selections.draft));
            if selections.style == ConfigStyle::Documented {
                lines.push("# title = \"{tag}\"".to_string());
                lines.push("# assets = []".to_string());
                lines.push("# notes_template = \"templates/release-notes.tera\"".to_string());
                lines.push("# discussion_category = \"releases\"".to_string());
            }
            lines.join("\n")
        }
        ConfigFormat::Yaml => {
            let mut lines = vec!["release:".to_string()];
            lines.push(format!("  github_release: {}", selections.github_release));
            lines.push(format!("  draft: {}", selections.draft));
            if selections.style == ConfigStyle::Documented {
                lines.push("  # title: \"{tag}\"".to_string());
                lines.push("  # assets: []".to_string());
                lines.push("  # notes_template: templates/release-notes.tera".to_string());
                lines.push("  # discussion_category: releases".to_string());
            }
            lines.join("\n")
        }
    }
}

/// Collapse runs of 3+ consecutive blank lines down to 2.
fn cleanup_blank_lines(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut consecutive_blanks = 0u32;

    for line in input.lines() {
        if line.trim().is_empty() {
            consecutive_blanks += 1;
            if consecutive_blanks <= 2 {
                result.push('\n');
            }
        } else {
            consecutive_blanks = 0;
            result.push_str(line);
            result.push('\n');
        }
    }

    // Preserve trailing newline if input had one
    if !input.ends_with('\n') && result.ends_with('\n') {
        result.pop();
    }

    result
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

    // ── generate_config tests ──────────────────────────────────────────

    fn default_selections() -> InitSelections {
        InitSelections {
            format: ConfigFormat::Toml,
            style: ConfigStyle::Documented,
            ecosystem: Some(Ecosystem::Rust),
            release_branch: None,
            github_release: true,
            draft: true,
        }
    }

    #[test]
    fn generate_config_documented_toml() {
        let output = generate_config(&default_selections());
        assert!(output.contains("# scrat configuration"), "missing header");
        assert!(output.contains("type = \"rust\""), "missing ecosystem");
        assert!(output.contains("github_release = true"), "missing gh release");
        assert!(output.contains("draft = true"), "missing draft");
        assert!(output.contains("# [hooks]"), "missing hooks section");
    }

    #[test]
    fn generate_config_documented_yaml() {
        let mut sel = default_selections();
        sel.format = ConfigFormat::Yaml;
        let output = generate_config(&sel);
        assert!(output.contains("# scrat configuration"), "missing header");
        assert!(output.contains("type: rust"), "missing ecosystem");
        assert!(output.contains("github_release: true"), "missing gh release");
        assert!(output.contains("# hooks:"), "missing hooks section");
    }

    #[test]
    fn generate_config_minimal_toml() {
        let mut sel = default_selections();
        sel.style = ConfigStyle::Minimal;
        let output = generate_config(&sel);
        assert!(output.contains("type = \"rust\""), "missing ecosystem");
        assert!(output.contains("github_release = true"), "missing gh release");
        assert!(!output.contains("# ───"), "should not have dividers");
    }

    #[test]
    fn generate_config_minimal_yaml() {
        let mut sel = default_selections();
        sel.format = ConfigFormat::Yaml;
        sel.style = ConfigStyle::Minimal;
        let output = generate_config(&sel);
        assert!(output.contains("type: rust"), "missing ecosystem");
        assert!(output.contains("github_release: true"), "missing gh release");
        assert!(!output.contains("# ───"), "should not have dividers");
    }

    #[test]
    fn generate_config_no_ecosystem_omits_project() {
        let mut sel = default_selections();
        sel.style = ConfigStyle::Minimal;
        sel.ecosystem = None;
        let output = generate_config(&sel);
        assert!(!output.contains("[project]"), "should omit project section");
        assert!(!output.contains("type ="), "should omit type field");
    }

    #[test]
    fn generate_config_no_github_release() {
        let mut sel = default_selections();
        sel.style = ConfigStyle::Minimal;
        sel.github_release = false;
        let output = generate_config(&sel);
        assert!(
            output.contains("github_release = false"),
            "should contain github_release = false"
        );
    }

    #[test]
    fn generate_config_with_release_branch() {
        let mut sel = default_selections();
        sel.release_branch = Some("release".to_string());
        let output = generate_config(&sel);
        assert!(
            output.contains("release_branch = \"release\""),
            "should contain release_branch"
        );
    }

    #[test]
    fn generate_config_generic_ecosystem() {
        let mut sel = default_selections();
        sel.ecosystem = Some(Ecosystem::Generic);
        let output = generate_config(&sel);
        assert!(
            output.contains("type = \"generic\""),
            "should contain generic type"
        );
    }
}
