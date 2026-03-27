//! Version bump planning and execution.
//!
//! All orchestration logic lives here. The CLI is purely a display layer.
//!
//! # Two-phase workflow
//!
//! 1. **Plan** ([`plan_bump`]) — detect ecosystem, resolve version strategy,
//!    compute the next version (or gather interactive context).
//! 2. **Execute** ([`ReadyBump::execute`]) — update project files and
//!    generate changelog.
//!
//! If the plan comes back as [`BumpPlan::NeedsInteraction`], the CLI prompts
//! the user and calls [`resolve_interactive`] to get a [`ReadyBump`].

use std::process::Command;

use camino::Utf8Path;
use semver::Version;
use serde::Serialize;
use thiserror::Error;
use tracing::{debug, info, instrument};

use crate::config::Config;
use crate::ecosystem::{ChangelogTool, Ecosystem, ProjectDetection, VersionStrategy};
use crate::version::{self, conventional, explicit, interactive};

// ──────────────────────────────────────────────
// Errors
// ──────────────────────────────────────────────

/// Errors from bump operations.
#[derive(Error, Debug)]
pub enum BumpError {
    /// A shell command failed during the bump.
    #[error("{tool} failed: {message}")]
    ToolFailed {
        /// Tool name.
        tool: String,
        /// Error details.
        message: String,
    },

    /// No bump tool available for this ecosystem.
    #[error("no bump tool available (install cargo-edit for Rust)")]
    NoBumpTool,

    /// Ecosystem not supported for bump operations.
    #[error("bump not yet supported for {0} ecosystem")]
    UnsupportedEcosystem(Ecosystem),

    /// Project detection failed.
    #[error("project detection failed: {0}")]
    Detection(String),

    /// Version computation failed.
    #[error(transparent)]
    Version(#[from] crate::version::VersionError),
}

/// Result alias for bump operations.
pub type BumpResult<T> = Result<T, BumpError>;

// ──────────────────────────────────────────────
// Plan types
// ──────────────────────────────────────────────

/// The result of planning a bump — either ready to execute or needs user input.
#[derive(Debug)]
pub enum BumpPlan {
    /// Version fully determined (explicit or conventional commits).
    Ready(ReadyBump),
    /// Interactive mode — the CLI must prompt the user and call [`resolve_interactive`].
    NeedsInteraction(InteractiveBump),
}

/// A bump plan that is ready to execute.
#[derive(Debug, Clone)]
pub struct ReadyBump {
    /// The previous version (from tags, or 0.0.0 for first release).
    pub previous: Version,
    /// The computed next version.
    pub next: Version,
    /// How the version was determined.
    pub strategy: VersionStrategy,
    /// Detected ecosystem and tools.
    pub detection: ProjectDetection,
}

/// A bump plan that requires the user to pick a version interactively.
#[derive(Debug)]
pub struct InteractiveBump {
    /// Context for the interactive picker (commits, candidates).
    pub context: interactive::InteractiveContext,
    /// Detected ecosystem and tools.
    pub detection: ProjectDetection,
}

// ──────────────────────────────────────────────
// Plan
// ──────────────────────────────────────────────

/// Plan a version bump: detect ecosystem, resolve strategy, compute version.
///
/// Returns [`BumpPlan::Ready`] when the version can be determined automatically
/// (explicit or conventional commits), or [`BumpPlan::NeedsInteraction`] when
/// the user must pick a version from candidates.
///
/// # Arguments
/// * `project_root` — project working directory
/// * `config` — loaded configuration (for strategy overrides)
/// * `explicit_version` — if set, overrides everything (from CLI `--version` flag)
#[instrument(skip(config), fields(%project_root))]
pub fn plan_bump(
    project_root: &Utf8Path,
    config: &Config,
    explicit_version: Option<&str>,
) -> BumpResult<BumpPlan> {
    // Step 1: Detect ecosystem (config override > auto-detect)
    let detection = crate::detect::resolve_detection(project_root, config).ok_or_else(|| {
        BumpError::Detection(
            "could not detect project type — use `project.type` in config or select interactively"
                .into(),
        )
    })?;

    // Step 2: Determine version strategy
    // CLI --version flag > config override > auto-detected
    let strategy = explicit_version.map_or_else(
        || resolve_strategy(config, &detection),
        |v| VersionStrategy::Explicit(v.to_owned()),
    );

    debug!(%strategy, "resolved version strategy");

    // Step 3: Compute version (or gather interactive context)
    match strategy {
        VersionStrategy::Explicit(ref v) => {
            let next = explicit::validate_explicit(v)?;
            let previous = current_or_zero()?;
            Ok(BumpPlan::Ready(ReadyBump {
                previous,
                next,
                strategy,
                detection,
            }))
        }
        VersionStrategy::ConventionalCommits { tool } => {
            let next = conventional::compute_next_version(tool)?;
            let previous = current_or_zero()?;
            Ok(BumpPlan::Ready(ReadyBump {
                previous,
                next,
                strategy: VersionStrategy::ConventionalCommits { tool },
                detection,
            }))
        }
        VersionStrategy::Interactive => {
            let context = interactive::gather_interactive_context(20)?;
            Ok(BumpPlan::NeedsInteraction(InteractiveBump {
                context,
                detection,
            }))
        }
    }
}

/// Finalize an interactive plan with the user's chosen version.
pub fn resolve_interactive(plan: InteractiveBump, chosen_version: Version) -> ReadyBump {
    let previous = plan
        .context
        .current_version
        .clone()
        .unwrap_or_else(|| Version::new(0, 0, 0));
    ReadyBump {
        previous,
        next: chosen_version,
        strategy: VersionStrategy::Interactive,
        detection: plan.detection,
    }
}

/// Determine the version strategy from config overrides or auto-detection.
fn resolve_strategy(config: &Config, detection: &ProjectDetection) -> VersionStrategy {
    // Config strategy override
    if let Some(ref vc) = config.version
        && let Some(ref s) = vc.strategy
    {
        match s.as_str() {
            "conventional-commits" => {
                // Use the detected changelog tool, or default to git-cliff
                let tool = detection
                    .tools
                    .changelog_tool
                    .unwrap_or(ChangelogTool::GitCliff);
                return VersionStrategy::ConventionalCommits { tool };
            }
            "interactive" => return VersionStrategy::Interactive,
            // Anything else: fall through to detection
            _ => {}
        }
    }
    detection.version_strategy.clone()
}

/// Get the current version from tags, defaulting to 0.0.0 for first releases.
fn current_or_zero() -> BumpResult<Version> {
    let current = version::current_version_from_tags()?;
    Ok(current.unwrap_or_else(|| Version::new(0, 0, 0)))
}

// ──────────────────────────────────────────────
// Execute
// ──────────────────────────────────────────────

/// Result of a successful bump operation.
#[derive(Debug, Clone, Serialize)]
pub struct BumpOutcome {
    /// The previous version.
    pub previous: Version,
    /// The new version.
    pub new: Version,
    /// Whether the changelog was updated.
    pub changelog_updated: bool,
    /// Files that were modified.
    pub modified_files: Vec<String>,
}

impl ReadyBump {
    /// Execute the bump: update project files and optionally generate changelog.
    #[instrument(skip(self), fields(ecosystem = %self.detection.ecosystem, next = %self.next))]
    pub fn execute(
        &self,
        project_root: &Utf8Path,
        update_changelog: bool,
    ) -> BumpResult<BumpOutcome> {
        let mut modified_files = Vec::new();

        // Update version in project files (Generic has no project files to update)
        match self.detection.ecosystem {
            Ecosystem::Rust => {
                bump_rust_version(project_root, &self.next, &self.detection)?;
                modified_files.push("Cargo.toml".into());
            }
            Ecosystem::Node => {
                return Err(BumpError::UnsupportedEcosystem(Ecosystem::Node));
            }
            Ecosystem::Go | Ecosystem::Swift => {
                debug!(%self.detection.ecosystem, "version lives in git tags, no file to bump");
            }
            Ecosystem::Php => {
                if bump_composer_version(project_root, &self.next)? {
                    modified_files.push("composer.json".into());
                } else {
                    debug!("composer.json has no version field, skipping");
                }
            }
            Ecosystem::Python => {
                if bump_pyproject_version(project_root, &self.next)? {
                    modified_files.push("pyproject.toml".into());
                } else {
                    debug!("pyproject.toml has no version field, skipping");
                }
            }
            Ecosystem::Ruby => {
                debug!("ruby version bump not yet supported — version lives in gemspec/version.rb");
            }
            Ecosystem::Generic => {
                debug!("generic ecosystem — no project files to bump");
            }
        }

        // Generate/update changelog (if requested and tool available)
        let changelog_updated = if update_changelog {
            if let Some(tool) = self.detection.tools.changelog_tool {
                generate_changelog(project_root, &self.next, tool)?;
                modified_files.push("CHANGELOG.md".into());
                true
            } else {
                debug!("no changelog tool configured, skipping");
                false
            }
        } else {
            false
        };

        info!(
            previous = %self.previous,
            new = %self.next,
            changelog_updated,
            "bump complete"
        );

        Ok(BumpOutcome {
            previous: self.previous.clone(),
            new: self.next.clone(),
            changelog_updated,
            modified_files,
        })
    }
}

// ──────────────────────────────────────────────
// Internal helpers
// ──────────────────────────────────────────────

/// Bump the version in Cargo.toml using `cargo set-version`.
fn bump_rust_version(
    project_root: &Utf8Path,
    version: &Version,
    detection: &ProjectDetection,
) -> BumpResult<()> {
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

    Ok(())
}

/// Bump the version in `composer.json` if it has a `"version"` field.
///
/// Returns `true` if the file was modified, `false` if no version field exists.
fn bump_composer_version(project_root: &Utf8Path, version: &Version) -> BumpResult<bool> {
    let composer_path = project_root.join("composer.json");
    let content = match std::fs::read_to_string(&composer_path) {
        Ok(c) => c,
        Err(_) => return Ok(false),
    };

    let mut parsed: serde_json::Value =
        serde_json::from_str(&content).map_err(|e| BumpError::ToolFailed {
            tool: "composer.json".into(),
            message: format!("failed to parse: {e}"),
        })?;

    // Only write if the field already exists — don't add it if absent
    if parsed.get("version").and_then(|v| v.as_str()).is_none() {
        return Ok(false);
    }

    parsed["version"] = serde_json::Value::String(version.to_string());

    let output = serde_json::to_string_pretty(&parsed).map_err(|e| BumpError::ToolFailed {
        tool: "composer.json".into(),
        message: format!("failed to serialize: {e}"),
    })?;

    // Composer convention: trailing newline
    std::fs::write(&composer_path, format!("{output}\n")).map_err(|e| BumpError::ToolFailed {
        tool: "composer.json".into(),
        message: format!("failed to write: {e}"),
    })?;

    debug!(%version, "bumped composer.json version");
    Ok(true)
}

/// Bump the version in `pyproject.toml` if it has a `version` field under `[project]`.
///
/// Returns `true` if the file was modified, `false` if no version field exists.
fn bump_pyproject_version(project_root: &Utf8Path, version: &Version) -> BumpResult<bool> {
    let pyproject_path = project_root.join("pyproject.toml");
    let content = match std::fs::read_to_string(&pyproject_path) {
        Ok(c) => c,
        Err(_) => return Ok(false),
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
        return Ok(false);
    }

    std::fs::write(&pyproject_path, lines.join("\n") + "\n").map_err(|e| {
        BumpError::ToolFailed {
            tool: "pyproject.toml".into(),
            message: format!("failed to write: {e}"),
        }
    })?;

    debug!(%version, "bumped pyproject.toml version");
    Ok(true)
}

/// Generate or update the changelog.
fn generate_changelog(
    project_root: &Utf8Path,
    version: &Version,
    tool: ChangelogTool,
) -> BumpResult<()> {
    match tool {
        ChangelogTool::GitCliff => {
            debug!("generating changelog via git-cliff");
            let output = Command::new("git-cliff")
                .args(["--output", "CHANGELOG.md", "--tag"])
                .arg(format!("v{version}"))
                .current_dir(project_root.as_std_path())
                .output()
                .map_err(|e| BumpError::ToolFailed {
                    tool: "git-cliff".into(),
                    message: format!("failed to execute: {e}"),
                })?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                return Err(BumpError::ToolFailed {
                    tool: "git-cliff".into(),
                    message: stderr,
                });
            }
        }
        ChangelogTool::Cog => {
            debug!("generating changelog via cog");
            let output = Command::new("cog")
                .arg("changelog")
                .current_dir(project_root.as_std_path())
                .output()
                .map_err(|e| BumpError::ToolFailed {
                    tool: "cog".into(),
                    message: format!("failed to execute: {e}"),
                })?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                return Err(BumpError::ToolFailed {
                    tool: "cog".into(),
                    message: stderr,
                });
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecosystem::{ChangelogTool, DetectedTools};

    /// Build a minimal Rust ProjectDetection for testing.
    fn rust_detection() -> ProjectDetection {
        ProjectDetection {
            ecosystem: Ecosystem::Rust,
            version_strategy: VersionStrategy::Interactive,
            tools: DetectedTools {
                test_cmd: "cargo test".into(),
                build_cmd: "cargo build --release".into(),
                publish_cmd: Some("cargo publish".into()),
                bump_cmd: Some("cargo set-version".into()),
                changelog_tool: None,
            },
        }
    }

    /// Build a minimal Generic ProjectDetection for testing.
    fn generic_detection() -> ProjectDetection {
        ProjectDetection::generic(VersionStrategy::Interactive)
    }

    // ── resolve_interactive ─────────────────────────────────

    #[test]
    fn resolve_interactive_with_current_version() {
        let context = interactive::InteractiveContext {
            current_version: Some(Version::new(1, 2, 3)),
            recent_commits: vec![("abc1234".into(), "feat: stuff".into())],
            candidates: vec![],
        };
        let plan = InteractiveBump {
            context,
            detection: generic_detection(),
        };

        let ready = resolve_interactive(plan, Version::new(2, 0, 0));
        assert_eq!(ready.previous, Version::new(1, 2, 3));
        assert_eq!(ready.next, Version::new(2, 0, 0));
        assert_eq!(ready.strategy, VersionStrategy::Interactive);
        assert_eq!(ready.detection.ecosystem, Ecosystem::Generic);
    }

    #[test]
    fn resolve_interactive_without_current_version_defaults_to_zero() {
        let context = interactive::InteractiveContext {
            current_version: None,
            recent_commits: vec![],
            candidates: vec![],
        };
        let plan = InteractiveBump {
            context,
            detection: generic_detection(),
        };

        let ready = resolve_interactive(plan, Version::new(0, 1, 0));
        assert_eq!(ready.previous, Version::new(0, 0, 0));
        assert_eq!(ready.next, Version::new(0, 1, 0));
    }

    // ── resolve_strategy ────────────────────────────────────

    #[test]
    fn resolve_strategy_uses_config_conventional_commits() {
        let config = Config {
            version: Some(crate::config::VersionConfig {
                strategy: Some("conventional-commits".into()),
            }),
            ..Config::default()
        };
        let detection = ProjectDetection {
            ecosystem: Ecosystem::Rust,
            version_strategy: VersionStrategy::Interactive,
            tools: DetectedTools {
                test_cmd: String::new(),
                build_cmd: String::new(),
                publish_cmd: None,
                bump_cmd: None,
                changelog_tool: Some(ChangelogTool::Cog),
            },
        };

        let strategy = resolve_strategy(&config, &detection);
        // Should use detected tool (Cog)
        assert_eq!(
            strategy,
            VersionStrategy::ConventionalCommits {
                tool: ChangelogTool::Cog
            }
        );
    }

    #[test]
    fn resolve_strategy_cc_defaults_to_git_cliff_when_no_tool_detected() {
        let config = Config {
            version: Some(crate::config::VersionConfig {
                strategy: Some("conventional-commits".into()),
            }),
            ..Config::default()
        };
        let detection = ProjectDetection {
            ecosystem: Ecosystem::Rust,
            version_strategy: VersionStrategy::Interactive,
            tools: DetectedTools {
                test_cmd: String::new(),
                build_cmd: String::new(),
                publish_cmd: None,
                bump_cmd: None,
                changelog_tool: None,
            },
        };

        let strategy = resolve_strategy(&config, &detection);
        assert_eq!(
            strategy,
            VersionStrategy::ConventionalCommits {
                tool: ChangelogTool::GitCliff
            }
        );
    }

    #[test]
    fn resolve_strategy_uses_config_interactive() {
        let config = Config {
            version: Some(crate::config::VersionConfig {
                strategy: Some("interactive".into()),
            }),
            ..Config::default()
        };
        let detection = rust_detection();

        let strategy = resolve_strategy(&config, &detection);
        assert_eq!(strategy, VersionStrategy::Interactive);
    }

    #[test]
    fn resolve_strategy_falls_through_on_unknown_string() {
        let config = Config {
            version: Some(crate::config::VersionConfig {
                strategy: Some("unknown-thing".into()),
            }),
            ..Config::default()
        };
        let detection = rust_detection();

        let strategy = resolve_strategy(&config, &detection);
        // Should fall through to the detection's strategy
        assert_eq!(strategy, detection.version_strategy);
    }

    #[test]
    fn resolve_strategy_no_config_version_uses_detection() {
        let config = Config::default();
        let detection = ProjectDetection {
            ecosystem: Ecosystem::Rust,
            version_strategy: VersionStrategy::ConventionalCommits {
                tool: ChangelogTool::GitCliff,
            },
            tools: DetectedTools {
                test_cmd: String::new(),
                build_cmd: String::new(),
                publish_cmd: None,
                bump_cmd: None,
                changelog_tool: Some(ChangelogTool::GitCliff),
            },
        };

        let strategy = resolve_strategy(&config, &detection);
        assert_eq!(
            strategy,
            VersionStrategy::ConventionalCommits {
                tool: ChangelogTool::GitCliff
            }
        );
    }

    #[test]
    fn resolve_strategy_config_version_without_strategy_uses_detection() {
        let config = Config {
            version: Some(crate::config::VersionConfig { strategy: None }),
            ..Config::default()
        };
        let detection = rust_detection();

        let strategy = resolve_strategy(&config, &detection);
        assert_eq!(strategy, detection.version_strategy);
    }

    // ── ReadyBump::execute ──────────────────────────────────

    #[test]
    fn execute_generic_no_changelog_tool() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = camino::Utf8Path::from_path(tmp.path()).unwrap();

        let ready = ReadyBump {
            previous: Version::new(0, 0, 0),
            next: Version::new(0, 1, 0),
            strategy: VersionStrategy::Interactive,
            detection: generic_detection(),
        };

        let outcome = ready.execute(root, false).unwrap();
        assert_eq!(outcome.previous, Version::new(0, 0, 0));
        assert_eq!(outcome.new, Version::new(0, 1, 0));
        assert!(!outcome.changelog_updated);
        assert!(outcome.modified_files.is_empty());
    }

    #[test]
    fn execute_generic_changelog_requested_but_no_tool() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = camino::Utf8Path::from_path(tmp.path()).unwrap();

        let ready = ReadyBump {
            previous: Version::new(1, 0, 0),
            next: Version::new(1, 1, 0),
            strategy: VersionStrategy::Interactive,
            detection: generic_detection(),
        };

        // Changelog requested but no tool available — should succeed with no changelog
        let outcome = ready.execute(root, true).unwrap();
        assert!(!outcome.changelog_updated);
        assert!(outcome.modified_files.is_empty());
    }

    #[test]
    fn execute_node_returns_unsupported() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = camino::Utf8Path::from_path(tmp.path()).unwrap();

        let detection = ProjectDetection {
            ecosystem: Ecosystem::Node,
            version_strategy: VersionStrategy::Interactive,
            tools: DetectedTools {
                test_cmd: "npm test".into(),
                build_cmd: "npm run build".into(),
                publish_cmd: Some("npm publish".into()),
                bump_cmd: Some("npm version --no-git-tag-version".into()),
                changelog_tool: None,
            },
        };

        let ready = ReadyBump {
            previous: Version::new(1, 0, 0),
            next: Version::new(1, 0, 1),
            strategy: VersionStrategy::Interactive,
            detection,
        };

        let err = ready.execute(root, false).unwrap_err();
        assert!(matches!(
            err,
            BumpError::UnsupportedEcosystem(Ecosystem::Node)
        ));
    }

    #[test]
    fn execute_rust_no_bump_tool_returns_error() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = camino::Utf8Path::from_path(tmp.path()).unwrap();

        let detection = ProjectDetection {
            ecosystem: Ecosystem::Rust,
            version_strategy: VersionStrategy::Interactive,
            tools: DetectedTools {
                test_cmd: "cargo test".into(),
                build_cmd: "cargo build --release".into(),
                publish_cmd: None,
                bump_cmd: None, // No bump tool!
                changelog_tool: None,
            },
        };

        let ready = ReadyBump {
            previous: Version::new(0, 1, 0),
            next: Version::new(0, 2, 0),
            strategy: VersionStrategy::Interactive,
            detection,
        };

        let err = ready.execute(root, false).unwrap_err();
        assert!(matches!(err, BumpError::NoBumpTool));
    }

    // ── BumpOutcome serialization ───────────────────────────

    #[test]
    fn bump_outcome_serializes() {
        let outcome = BumpOutcome {
            previous: Version::new(1, 0, 0),
            new: Version::new(1, 1, 0),
            changelog_updated: true,
            modified_files: vec!["Cargo.toml".into(), "CHANGELOG.md".into()],
        };

        let json = serde_json::to_string(&outcome).unwrap();
        assert!(json.contains("\"previous\":\"1.0.0\""));
        assert!(json.contains("\"new\":\"1.1.0\""));
        assert!(json.contains("\"changelog_updated\":true"));
        assert!(json.contains("Cargo.toml"));
    }

    // ── Error display ───────────────────────────────────────

    #[test]
    fn bump_error_tool_failed_display() {
        let err = BumpError::ToolFailed {
            tool: "cargo set-version".into(),
            message: "not found".into(),
        };
        let msg = err.to_string();
        assert!(msg.contains("cargo set-version"));
        assert!(msg.contains("not found"));
    }

    #[test]
    fn bump_error_no_bump_tool_display() {
        let err = BumpError::NoBumpTool;
        assert!(err.to_string().contains("no bump tool available"));
    }

    #[test]
    fn bump_error_unsupported_ecosystem_display() {
        let err = BumpError::UnsupportedEcosystem(Ecosystem::Node);
        assert!(err.to_string().contains("node"));
    }

    #[test]
    fn bump_error_detection_display() {
        let err = BumpError::Detection("could not detect".into());
        assert!(err.to_string().contains("could not detect"));
    }

    // ── ReadyBump fields ────────────────────────────────────

    #[test]
    fn ready_bump_clone() {
        let ready = ReadyBump {
            previous: Version::new(1, 2, 3),
            next: Version::new(1, 3, 0),
            strategy: VersionStrategy::Explicit("1.3.0".into()),
            detection: generic_detection(),
        };

        let cloned = ready.clone();
        assert_eq!(cloned.previous, ready.previous);
        assert_eq!(cloned.next, ready.next);
        assert_eq!(cloned.strategy, ready.strategy);
    }

    // ── plan_bump ───────────────────────────────────────────

    #[test]
    fn plan_bump_explicit_version_in_detected_project() {
        // Use a tempdir with config override so detection succeeds
        let tmp = tempfile::TempDir::new().unwrap();
        let root = camino::Utf8Path::from_path(tmp.path()).unwrap();

        let config = Config {
            project: Some(crate::config::ProjectConfig {
                project_type: Some(Ecosystem::Generic),
                release_branch: None,
            }),
            ..Config::default()
        };

        let result = plan_bump(root, &config, Some("2.0.0"));
        // This calls current_or_zero which calls git. It may fail
        // outside a git repo. Inside scrat's repo it should succeed.
        if let Ok(BumpPlan::Ready(ready)) = result {
            assert_eq!(ready.next, Version::new(2, 0, 0));
            assert!(matches!(ready.strategy, VersionStrategy::Explicit(_)));
        }
    }

    #[test]
    fn plan_bump_fails_when_detection_fails() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = camino::Utf8Path::from_path(tmp.path()).unwrap();
        // No marker files, no config override -> detection fails
        let config = Config::default();

        let result = plan_bump(root, &config, None);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, BumpError::Detection(_)));
    }

    #[test]
    fn plan_bump_explicit_overrides_config_strategy() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = camino::Utf8Path::from_path(tmp.path()).unwrap();

        let config = Config {
            project: Some(crate::config::ProjectConfig {
                project_type: Some(Ecosystem::Generic),
                release_branch: None,
            }),
            version: Some(crate::config::VersionConfig {
                strategy: Some("interactive".into()),
            }),
            ..Config::default()
        };

        let result = plan_bump(root, &config, Some("3.0.0"));
        if let Ok(BumpPlan::Ready(ready)) = result {
            // Strategy should be Explicit, not Interactive
            assert!(matches!(ready.strategy, VersionStrategy::Explicit(_)));
            assert_eq!(ready.next, Version::new(3, 0, 0));
        }
    }

    #[test]
    fn plan_bump_explicit_with_v_prefix() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = camino::Utf8Path::from_path(tmp.path()).unwrap();

        let config = Config {
            project: Some(crate::config::ProjectConfig {
                project_type: Some(Ecosystem::Generic),
                release_branch: None,
            }),
            ..Config::default()
        };

        let result = plan_bump(root, &config, Some("v1.5.0"));
        if let Ok(BumpPlan::Ready(ready)) = result {
            assert_eq!(ready.next, Version::new(1, 5, 0));
        }
    }

    #[test]
    fn plan_bump_explicit_invalid_version() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = camino::Utf8Path::from_path(tmp.path()).unwrap();

        let config = Config {
            project: Some(crate::config::ProjectConfig {
                project_type: Some(Ecosystem::Generic),
                release_branch: None,
            }),
            ..Config::default()
        };

        let result = plan_bump(root, &config, Some("not-semver"));
        assert!(result.is_err());
    }
}
