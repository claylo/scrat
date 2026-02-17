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
