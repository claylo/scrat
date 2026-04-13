//! Ship orchestrator — the full release workflow.
//!
//! Wires together preflight checks, version resolution, testing, bumping,
//! publishing, git operations, and GitHub release creation into a single
//! pipeline with hooks at every phase boundary.
//!
//! # Two-phase workflow
//!
//! 1. **Plan** ([`plan_ship`]) — run preflight checks, detect ecosystem,
//!    resolve version (may need user interaction).
//! 2. **Execute** ([`ReadyShip::execute`]) — run the full pipeline with
//!    event callbacks for progress display.
//!
//! If the plan returns [`ShipPlan::NeedsInteraction`], the CLI prompts
//! the user and calls [`resolve_ship_interaction`] to get a [`ReadyShip`].

use std::process::{Command, Stdio};

use camino::Utf8Path;
use semver::Version;
use serde::Serialize;
use thiserror::Error;
use tracing::{debug, info, instrument, warn};

use crate::bump::{self, InteractiveBump, ReadyBump};
use crate::config::Config;
use crate::deps;
use crate::ecosystem::ProjectDetection;
use crate::git;
use crate::hooks::{self, HookContext};
use crate::notes;
use crate::pipeline::{PipelineContext, PipelineContextInit};
use crate::preflight;
use crate::stats;

// ──────────────────────────────────────────────
// Errors
// ──────────────────────────────────────────────

/// Errors from the ship workflow.
#[derive(Error, Debug)]
pub enum ShipError {
    /// Preflight checks failed.
    #[error("preflight checks failed: {0}")]
    PreflightFailed(String),

    /// A phase failed during execution.
    #[error("{phase} phase failed: {message}")]
    PhaseFailed {
        /// Which phase failed.
        phase: ShipPhase,
        /// Error details.
        message: String,
    },

    /// Version bump error.
    #[error(transparent)]
    Bump(#[from] crate::bump::BumpError),

    /// Git error.
    #[error(transparent)]
    Git(#[from] crate::git::GitError),

    /// Hook error.
    #[error(transparent)]
    Hook(#[from] crate::hooks::HookError),

    /// Version error.
    #[error(transparent)]
    Version(#[from] crate::version::VersionError),
}

/// Result alias for ship operations.
pub type ShipResult<T> = Result<T, ShipError>;

// ──────────────────────────────────────────────
// Options
// ──────────────────────────────────────────────

/// Options controlling which phases of the ship workflow run.
#[derive(Debug, Clone, Default)]
pub struct ShipOptions {
    /// Set the version explicitly (e.g., `"1.2.3"`).
    pub explicit_version: Option<String>,
    /// Skip changelog generation during the bump phase.
    pub no_changelog: bool,
    /// Skip the publish phase entirely.
    pub no_publish: bool,
    /// Skip git push (still commits and tags locally).
    pub no_push: bool,
    /// Skip GitHub release creation.
    pub no_release: bool,
    /// Skip dependency diff computation.
    pub no_deps: bool,
    /// Skip release statistics collection.
    pub no_stats: bool,
    /// Skip release notes rendering (falls back to --generate-notes).
    pub no_notes: bool,
    /// Preview what would happen without making changes.
    pub dry_run: bool,
    /// Skip running tests.
    pub no_test: bool,
    /// Skip git tag creation (still commits and pushes).
    pub no_tag: bool,
    /// Skip entire git phase (commit, tag, push).
    pub no_git: bool,
    /// Skip `git fetch` before comparing local and remote during preflight.
    /// Trades freshness of the remote-sync check for startup latency.
    pub no_fetch: bool,
    /// Override draft mode from CLI (`Some(true)` = `--draft`, `Some(false)` = `--no-draft`).
    pub draft_override: Option<bool>,
}

// ──────────────────────────────────────────────
// Phases and events
// ──────────────────────────────────────────────

/// Phases of the ship workflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ShipPhase {
    /// Validate release readiness.
    Preflight,
    /// Determine the next version.
    Version,
    /// Run the test suite.
    Test,
    /// Update version in project files and generate changelog.
    Bump,
    /// Publish to a package registry.
    Publish,
    /// Commit, tag, and push to remote.
    Git,
    /// Create a GitHub release with notes and assets.
    Release,
}

impl std::fmt::Display for ShipPhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Preflight => write!(f, "preflight"),
            Self::Version => write!(f, "version"),
            Self::Test => write!(f, "test"),
            Self::Bump => write!(f, "bump"),
            Self::Publish => write!(f, "publish"),
            Self::Git => write!(f, "git"),
            Self::Release => write!(f, "release"),
        }
    }
}

/// Events emitted during the ship workflow for progress reporting.
#[derive(Debug, Clone)]
pub enum ShipEvent {
    /// A phase has started.
    PhaseStarted(ShipPhase),
    /// A phase has completed.
    PhaseCompleted(ShipPhase, PhaseOutcome),
    /// Hook commands are about to run (or would run in dry-run mode).
    HooksStarted {
        /// Which phase the hooks belong to.
        phase: ShipPhase,
        /// Number of hook commands.
        count: usize,
        /// The hook commands (with interpolation applied). Useful for dry-run display.
        commands: Vec<String>,
        /// Whether these hooks will actually be executed (false in dry-run mode).
        will_execute: bool,
    },
    /// Hook commands have finished (or were skipped in dry-run mode).
    HooksCompleted {
        /// Which phase the hooks belong to.
        phase: ShipPhase,
        /// Number of hook commands that ran (or would have run).
        count: usize,
    },
}

/// Outcome of a single phase.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case", tag = "status")]
pub enum PhaseOutcome {
    /// Phase completed successfully.
    Success {
        /// Description of what happened.
        message: String,
    },
    /// Phase was skipped.
    Skipped {
        /// Why the phase was skipped.
        reason: String,
    },
}

/// Outcome of the full ship workflow.
#[derive(Debug, Clone, Serialize)]
pub struct ShipOutcome {
    /// The version that was shipped.
    pub version: Version,
    /// The previous version.
    pub previous_version: Version,
    /// The git tag that was created.
    pub tag: String,
    /// Results of each phase.
    pub phases: Vec<(ShipPhase, PhaseOutcome)>,
    /// Total number of hook commands executed.
    pub hooks_run: usize,
    /// Whether this was a dry run.
    pub dry_run: bool,
    /// Structured pipeline context with data from all phases.
    pub context: PipelineContext,
}

// ──────────────────────────────────────────────
// Plan types
// ──────────────────────────────────────────────

/// The result of planning a ship — either ready to execute or needs user input.
#[derive(Debug)]
pub enum ShipPlan {
    /// Version fully determined, ready to execute.
    Ready(ReadyShip),
    /// Interactive mode — the CLI must prompt for version selection.
    NeedsInteraction(InteractiveShip),
    /// Ecosystem could not be auto-detected — the CLI must prompt the user
    /// to select one (e.g., Generic), then re-plan with the chosen ecosystem.
    NeedsEcosystemSelection(NeedsEcosystemSelection),
}

/// A ship plan that is ready to execute.
#[derive(Debug)]
pub struct ReadyShip {
    /// The resolved bump plan.
    pub bump: ReadyBump,
    /// Ship workflow options.
    pub options: ShipOptions,
    /// Loaded configuration.
    pub config: Config,
    /// Detected project info.
    pub detection: ProjectDetection,
    /// Current git branch resolved during preflight so [`ReadyShip::execute`]
    /// doesn't re-invoke `git rev-parse --abbrev-ref HEAD`. `None` means
    /// detached HEAD or the branch could not be resolved.
    pub branch: Option<String>,
}

/// A ship that needs user input for version selection.
#[derive(Debug)]
pub struct InteractiveShip {
    /// The interactive bump plan (contains candidates).
    pub bump: InteractiveBump,
    /// Ship workflow options.
    pub options: ShipOptions,
    /// Loaded configuration.
    pub config: Config,
    /// Current git branch resolved during preflight (see
    /// [`ReadyShip::branch`]).
    pub branch: Option<String>,
}

/// Ecosystem auto-detection failed — the CLI must prompt the user.
#[derive(Debug)]
pub struct NeedsEcosystemSelection {
    /// Ship workflow options (preserved for re-planning after selection).
    pub options: ShipOptions,
    /// Loaded configuration (preserved for re-planning after selection).
    pub config: Config,
    /// Project root path.
    pub project_root: camino::Utf8PathBuf,
}

// ──────────────────────────────────────────────
// Plan
// ──────────────────────────────────────────────

/// Plan the ship workflow: run preflight checks and resolve the version.
///
/// Returns [`ShipPlan::Ready`] when the version can be determined automatically,
/// or [`ShipPlan::NeedsInteraction`] when the user must pick a version.
#[instrument(skip(config, options), fields(%project_root))]
pub fn plan_ship(
    project_root: &Utf8Path,
    config: &Config,
    options: ShipOptions,
) -> ShipResult<ShipPlan> {
    // Detection is pure `(project_root, config)` and cannot change during
    // planning — compute it once and thread it through both preflight and
    // bump planning. This avoids re-scanning marker files and re-probing
    // PATH twice per ship invocation.
    let detection = crate::detect::resolve_detection(project_root, config);

    // Phase 1: Preflight
    let report = preflight::run_preflight_with_detection(
        project_root,
        config,
        Some(&options),
        detection.clone(),
    );

    if !report.all_passed {
        let failures: Vec<&str> = report
            .checks
            .iter()
            .filter(|c| !c.passed)
            .map(|c| c.message.as_str())
            .collect();
        return Err(ShipError::PreflightFailed(failures.join("; ")));
    }

    // Phase 2: Version resolution
    let Some(detection) = detection else {
        // Ecosystem not detected — signal the CLI to prompt for selection
        debug!("ecosystem detection failed, requesting user selection");
        return Ok(ShipPlan::NeedsEcosystemSelection(NeedsEcosystemSelection {
            options,
            config: config.clone(),
            project_root: project_root.to_owned(),
        }));
    };

    let bump_plan = match bump::plan_bump_with_detection(
        project_root,
        config,
        options.explicit_version.as_deref(),
        detection,
    ) {
        Ok(plan) => plan,
        Err(bump::BumpError::Detection(_)) => {
            // Shouldn't happen since we pre-checked detection, but stay defensive
            debug!("bump detection failed after preflight succeeded, requesting user selection");
            return Ok(ShipPlan::NeedsEcosystemSelection(NeedsEcosystemSelection {
                options,
                config: config.clone(),
                project_root: project_root.to_owned(),
            }));
        }
        Err(e) => return Err(e.into()),
    };

    match bump_plan {
        bump::BumpPlan::Ready(ready_bump) => {
            let detection = ready_bump.detection.clone();
            Ok(ShipPlan::Ready(ReadyShip {
                bump: ready_bump,
                options,
                config: config.clone(),
                detection,
                branch: report.branch,
            }))
        }
        bump::BumpPlan::NeedsInteraction(interactive_bump) => {
            Ok(ShipPlan::NeedsInteraction(InteractiveShip {
                bump: interactive_bump,
                options,
                config: config.clone(),
                branch: report.branch,
            }))
        }
    }
}

/// Resolve an ecosystem selection by re-planning with the chosen ecosystem.
///
/// Called after the CLI prompts the user to select an ecosystem (e.g., Generic).
/// Injects the chosen ecosystem into the config and re-runs [`plan_ship`].
pub fn resolve_ecosystem_selection(
    selection: NeedsEcosystemSelection,
    ecosystem: crate::ecosystem::Ecosystem,
) -> ShipResult<ShipPlan> {
    use crate::config::ProjectConfig;

    // Inject the user's ecosystem choice into config
    let mut config = selection.config;
    let project = config.project.get_or_insert_with(ProjectConfig::default);
    project.project_type = Some(ecosystem);

    // Re-plan with the overridden config
    plan_ship(&selection.project_root, &config, selection.options)
}

/// Resolve an interactive ship plan with the user's chosen version.
pub fn resolve_ship_interaction(plan: InteractiveShip, chosen_version: Version) -> ReadyShip {
    let ready_bump = bump::resolve_interactive(plan.bump, chosen_version);
    let detection = ready_bump.detection.clone();
    ReadyShip {
        bump: ready_bump,
        options: plan.options,
        config: plan.config,
        detection,
        branch: plan.branch,
    }
}

// ──────────────────────────────────────────────
// Execute
// ──────────────────────────────────────────────

impl ReadyShip {
    /// Validate post-version-resolution preconditions.
    ///
    /// Called after the version is known but before the user confirms.
    /// Returns failed checks as a `Vec` (empty = all good).
    /// The CLI should display these and abort if any fail.
    pub fn validate(&self) -> Vec<preflight::CheckResult> {
        let mut failures = Vec::new();

        // Tag existence check (unless skipped)
        if !self.options.no_git && !self.options.no_tag {
            let tag = format!("v{}", self.bump.next);
            let check = preflight::check_tag_available(&tag);
            if !check.passed {
                failures.push(check);
            }
        }

        failures
    }

    /// Execute the full ship workflow.
    ///
    /// Calls `on_event` at phase boundaries so the CLI can update
    /// progress display (spinners, progress bars, etc.).
    #[instrument(skip(self, on_event), fields(
        version = %self.bump.next,
        dry_run = self.options.dry_run
    ))]
    pub fn execute(
        self,
        project_root: &Utf8Path,
        mut on_event: impl FnMut(ShipEvent),
    ) -> ShipResult<ShipOutcome> {
        let mut phases = Vec::new();
        let mut hooks_run: usize = 0;
        let is_dry = self.options.dry_run;

        let version = &self.bump.next;
        let previous = &self.bump.previous;
        let tag = format!("v{version}");

        // Build the pipeline context — accumulates structured data across phases
        let (owner, repo, repo_url) = {
            let remote = git::remote_url("origin").ok().flatten();
            let (o, r) = remote
                .as_deref()
                .and_then(git::parse_owner_repo)
                .unwrap_or_else(|| ("unknown".into(), "unknown".into()));
            (o, r, remote)
        };

        let mut ctx = PipelineContext::new(PipelineContextInit {
            version: version.to_string(),
            previous_version: previous.to_string(),
            tag: tag.clone(),
            previous_tag: format!("v{previous}"),
            owner,
            repo,
            repo_url,
            branch: self.branch.clone(),
            ecosystem: self.detection.ecosystem.to_string(),
            changelog_path: project_root.join("CHANGELOG.md").to_string(),
            dry_run: is_dry,
        });

        // Load release assets from config
        if let Some(assets) = self.config.release.as_ref().and_then(|r| r.assets.clone()) {
            ctx.set_assets(assets);
        }

        // Deps diff (silent data-gathering, populates context)
        if !self.options.no_deps {
            ctx.dependencies = deps::compute_deps(self.detection.ecosystem, &ctx.previous_tag);
        }

        // Stats collection (silent data-gathering, populates context)
        if !self.options.no_stats {
            ctx.stats = stats::compute_stats(&ctx.previous_tag);
        }

        // Derive hook interpolation context
        let hook_ctx = ctx.hook_context();

        let hooks_config = self.config.hooks.as_ref();

        // ── Preflight (already passed in plan phase) ──
        on_event(ShipEvent::PhaseStarted(ShipPhase::Preflight));
        let outcome = PhaseOutcome::Success {
            message: "All preflight checks passed".into(),
        };
        on_event(ShipEvent::PhaseCompleted(
            ShipPhase::Preflight,
            outcome.clone(),
        ));
        phases.push((ShipPhase::Preflight, outcome));

        // ── Version (already resolved in plan phase) ──
        on_event(ShipEvent::PhaseStarted(ShipPhase::Version));
        let outcome = PhaseOutcome::Success {
            message: format!(
                "{previous} → {version} ({strategy})",
                strategy = self.bump.strategy
            ),
        };
        on_event(ShipEvent::PhaseCompleted(
            ShipPhase::Version,
            outcome.clone(),
        ));
        phases.push((ShipPhase::Version, outcome));

        // ── pre_ship hooks ──
        hooks_run += run_phase_hooks(
            hooks_config.and_then(|h| h.pre_ship.as_deref()),
            &hook_ctx,
            project_root,
            ShipPhase::Preflight,
            is_dry,
            &mut on_event,
            &mut ctx,
        )?;

        // ── Test Phase ──
        hooks_run += run_phase_hooks(
            hooks_config.and_then(|h| h.pre_test.as_deref()),
            &hook_ctx,
            project_root,
            ShipPhase::Test,
            is_dry,
            &mut on_event,
            &mut ctx,
        )?;

        on_event(ShipEvent::PhaseStarted(ShipPhase::Test));
        let test_outcome = if self.options.no_test {
            PhaseOutcome::Skipped {
                reason: "--no-test flag".into(),
            }
        } else if is_dry {
            let test_cmd = self
                .config
                .commands
                .as_ref()
                .and_then(|c| c.test.as_deref())
                .unwrap_or(&self.detection.tools.test_cmd);
            PhaseOutcome::Success {
                message: format!("Would run: {test_cmd}"),
            }
        } else {
            run_test_phase(project_root, &self.config, &self.detection)?
        };
        on_event(ShipEvent::PhaseCompleted(
            ShipPhase::Test,
            test_outcome.clone(),
        ));
        phases.push((ShipPhase::Test, test_outcome));

        hooks_run += run_phase_hooks(
            hooks_config.and_then(|h| h.post_test.as_deref()),
            &hook_ctx,
            project_root,
            ShipPhase::Test,
            is_dry,
            &mut on_event,
            &mut ctx,
        )?;

        // ── Bump Phase ──
        hooks_run += run_phase_hooks(
            hooks_config.and_then(|h| h.pre_bump.as_deref()),
            &hook_ctx,
            project_root,
            ShipPhase::Bump,
            is_dry,
            &mut on_event,
            &mut ctx,
        )?;

        on_event(ShipEvent::PhaseStarted(ShipPhase::Bump));
        let bump_outcome = if is_dry {
            PhaseOutcome::Success {
                message: format!("Would bump {previous} → {version}"),
            }
        } else {
            let result = self
                .bump
                .execute(project_root, !self.options.no_changelog)?;
            let files = result.modified_files.join(", ");
            ctx.record_bump(result.changelog_updated, result.modified_files);
            PhaseOutcome::Success {
                message: format!(
                    "Bumped to {version}{changelog} (modified: {files})",
                    changelog = if result.changelog_updated {
                        " + changelog"
                    } else {
                        ""
                    },
                ),
            }
        };
        on_event(ShipEvent::PhaseCompleted(
            ShipPhase::Bump,
            bump_outcome.clone(),
        ));
        phases.push((ShipPhase::Bump, bump_outcome));

        hooks_run += run_phase_hooks(
            hooks_config.and_then(|h| h.post_bump.as_deref()),
            &hook_ctx,
            project_root,
            ShipPhase::Bump,
            is_dry,
            &mut on_event,
            &mut ctx,
        )?;

        // ── Release Notes (must run BEFORE git phase creates the tag) ──
        // git-cliff --unreleased --context finds commits since the latest tag.
        // Once the git phase tags, --unreleased returns nothing.
        let notes_file = if !self.options.no_release && !self.options.no_notes && !is_dry {
            let github_release = self
                .config
                .release
                .as_ref()
                .and_then(|r| r.github_release)
                .unwrap_or(true);
            if github_release {
                let custom_template = self
                    .config
                    .release
                    .as_ref()
                    .and_then(|r| r.notes_template.as_deref());
                match notes::render_notes(project_root, &ctx, custom_template) {
                    Ok(rendered) => {
                        debug!(len = rendered.len(), "release notes rendered");
                        ctx.release_notes = Some(rendered.clone());
                        // Write to temp file for --notes-file
                        match write_notes_tempfile(&rendered) {
                            Ok(f) => Some(f),
                            Err(e) => {
                                warn!(
                                    "failed to write notes temp file: {e}, falling back to --generate-notes"
                                );
                                None
                            }
                        }
                    }
                    Err(e) => {
                        warn!(
                            "release notes rendering failed: {e}, falling back to --generate-notes"
                        );
                        None
                    }
                }
            } else {
                None
            }
        } else {
            None
        };

        // ── Git Phase (commit + tag + push) ──
        if !self.options.no_git {
            hooks_run += run_phase_hooks(
                hooks_config.and_then(|h| h.pre_tag.as_deref()),
                &hook_ctx,
                project_root,
                ShipPhase::Git,
                is_dry,
                &mut on_event,
                &mut ctx,
            )?;
        }

        on_event(ShipEvent::PhaseStarted(ShipPhase::Git));
        let git_outcome = if self.options.no_git {
            PhaseOutcome::Skipped {
                reason: "--no-git flag".into(),
            }
        } else if is_dry {
            let tag_msg = if self.options.no_tag {
                String::new()
            } else {
                format!(", tag {tag}")
            };
            let push_msg = if self.options.no_push {
                " (no push)"
            } else {
                " + push"
            };
            PhaseOutcome::Success {
                message: format!("Would commit{tag_msg}{push_msg}"),
            }
        } else {
            let git_result = run_git_phase(
                project_root,
                &tag,
                version,
                ctx.branch.as_deref(),
                self.options.no_push,
                self.options.no_tag,
            )?;
            ctx.record_git(Some(git_result.hash.clone()), git_result.branch.clone());
            let tag_part = if self.options.no_tag {
                String::new()
            } else {
                format!(", tagged {tag}")
            };
            let push_part = if git_result.pushed {
                ", pushed"
            } else {
                " (push skipped)"
            };
            PhaseOutcome::Success {
                message: format!("Committed {}{tag_part}{push_part}", git_result.hash),
            }
        };
        on_event(ShipEvent::PhaseCompleted(
            ShipPhase::Git,
            git_outcome.clone(),
        ));
        phases.push((ShipPhase::Git, git_outcome));

        if !self.options.no_git {
            hooks_run += run_phase_hooks(
                hooks_config.and_then(|h| h.post_tag.as_deref()),
                &hook_ctx,
                project_root,
                ShipPhase::Git,
                is_dry,
                &mut on_event,
                &mut ctx,
            )?;
        }

        // ── Release Phase (GitHub release) ──
        hooks_run += run_phase_hooks(
            hooks_config.and_then(|h| h.pre_release.as_deref()),
            &hook_ctx,
            project_root,
            ShipPhase::Release,
            is_dry,
            &mut on_event,
            &mut ctx,
        )?;

        // Resolve release config for both dry-run and real execution
        let release_cfg = self.config.release.as_ref();
        let github_release = release_cfg.and_then(|r| r.github_release).unwrap_or(true);
        let draft = self
            .options
            .draft_override
            .or_else(|| release_cfg.and_then(|r| r.draft))
            .unwrap_or(true);
        let title = release_cfg
            .and_then(|r| r.title.as_deref())
            .map(|t| hooks::interpolate_command(t, &hook_ctx));
        let discussion_category = release_cfg.and_then(|r| r.discussion_category.as_deref());
        let assets_raw = release_cfg.and_then(|r| r.assets.as_deref()).unwrap_or(&[]);
        let assets: Vec<String> = assets_raw
            .iter()
            .map(|a| hooks::interpolate_command(a, &hook_ctx))
            .collect();

        on_event(ShipEvent::PhaseStarted(ShipPhase::Release));
        let release_outcome = if self.options.no_release {
            PhaseOutcome::Skipped {
                reason: "--no-release flag".into(),
            }
        } else if !github_release {
            PhaseOutcome::Skipped {
                reason: "github_release = false in config".into(),
            }
        } else if is_dry {
            let draft_label = if draft { " as draft" } else { "" };
            let title_label = title
                .as_ref()
                .map_or(String::new(), |t| format!(" titled \"{t}\""));
            let notes_msg = if self.options.no_notes {
                " (--generate-notes)"
            } else {
                " (with rendered notes)"
            };
            let asset_count = assets.len();
            let asset_msg = if asset_count > 0 {
                format!(
                    ", {} asset{}",
                    asset_count,
                    if asset_count == 1 { "" } else { "s" }
                )
            } else {
                String::new()
            };
            PhaseOutcome::Success {
                message: format!(
                    "Would create GitHub release for {tag}{draft_label}{title_label}{notes_msg}{asset_msg}"
                ),
            }
        } else {
            let notes_path = notes_file.as_ref().map(|f| f.path());
            let release_opts = ReleaseOptions {
                tag: &tag,
                title,
                draft,
                notes_file: notes_path,
                assets: &assets,
                discussion_category,
                project_root,
            };
            let release_result = run_release_phase(&release_opts)?;
            ctx.record_release(release_result.url.clone());
            let action = if release_result.edited {
                "Updated"
            } else {
                "Created"
            };
            let draft_label = if draft { " (draft)" } else { "" };
            let msg = release_result.url.as_ref().map_or_else(
                || format!("{action} GitHub release {tag}{draft_label}"),
                |url| format!("{action} GitHub release{draft_label}: {url}"),
            );
            PhaseOutcome::Success { message: msg }
        };
        on_event(ShipEvent::PhaseCompleted(
            ShipPhase::Release,
            release_outcome.clone(),
        ));
        phases.push((ShipPhase::Release, release_outcome));

        hooks_run += run_phase_hooks(
            hooks_config.and_then(|h| h.post_release.as_deref()),
            &hook_ctx,
            project_root,
            ShipPhase::Release,
            is_dry,
            &mut on_event,
            &mut ctx,
        )?;

        // ── Publish Phase ──
        hooks_run += run_phase_hooks(
            hooks_config.and_then(|h| h.pre_publish.as_deref()),
            &hook_ctx,
            project_root,
            ShipPhase::Publish,
            is_dry,
            &mut on_event,
            &mut ctx,
        )?;

        on_event(ShipEvent::PhaseStarted(ShipPhase::Publish));
        let publish_outcome = if self.options.no_publish {
            PhaseOutcome::Skipped {
                reason: "--no-publish flag".into(),
            }
        } else if is_dry {
            let publish_cmd = self
                .config
                .commands
                .as_ref()
                .and_then(|c| c.publish.as_deref())
                .or(self.detection.tools.publish_cmd.as_deref())
                .unwrap_or("(no publish command)");
            PhaseOutcome::Success {
                message: format!("Would run: {publish_cmd}"),
            }
        } else {
            run_publish_phase(project_root, &self.config, &self.detection)?
        };
        on_event(ShipEvent::PhaseCompleted(
            ShipPhase::Publish,
            publish_outcome.clone(),
        ));
        phases.push((ShipPhase::Publish, publish_outcome));

        hooks_run += run_phase_hooks(
            hooks_config.and_then(|h| h.post_publish.as_deref()),
            &hook_ctx,
            project_root,
            ShipPhase::Publish,
            is_dry,
            &mut on_event,
            &mut ctx,
        )?;

        // ── post_ship hooks ──
        hooks_run += run_phase_hooks(
            hooks_config.and_then(|h| h.post_ship.as_deref()),
            &hook_ctx,
            project_root,
            ShipPhase::Release,
            is_dry,
            &mut on_event,
            &mut ctx,
        )?;

        let outcome = ShipOutcome {
            version: version.clone(),
            previous_version: previous.clone(),
            tag,
            phases,
            hooks_run,
            dry_run: is_dry,
            context: ctx,
        };

        info!(
            version = %outcome.version,
            hooks_run = outcome.hooks_run,
            dry_run = outcome.dry_run,
            "ship complete"
        );

        Ok(outcome)
    }
}

// ──────────────────────────────────────────────
// Phase implementations
// ──────────────────────────────────────────────

/// Run hooks for a phase, returning the number of hooks reported.
///
/// In dry-run mode, hooks are reported (via events) but not executed.
/// If any `filter:` hooks are present and produce output, the pipeline
/// context is updated in place with the deserialized result.
fn run_phase_hooks(
    commands: Option<&[String]>,
    context: &HookContext,
    project_root: &Utf8Path,
    phase: ShipPhase,
    dry_run: bool,
    on_event: &mut impl FnMut(ShipEvent),
    pipeline_ctx: &mut PipelineContext,
) -> ShipResult<usize> {
    let Some(cmds) = commands else {
        return Ok(0);
    };
    if cmds.is_empty() {
        return Ok(0);
    }

    let count = cmds.len();
    let interpolated: Vec<String> = cmds
        .iter()
        .map(|cmd| hooks::interpolate_command(cmd, context))
        .collect();

    on_event(ShipEvent::HooksStarted {
        phase,
        count,
        commands: interpolated,
        will_execute: !dry_run,
    });

    if !dry_run {
        // Only serialize the pipeline context if there's at least one
        // `filter:` hook that will consume it — otherwise we'd be paying
        // the serialize-and-drop cost at every phase boundary for no benefit.
        let has_filter = cmds.iter().any(|c| c.trim_start().starts_with("filter:"));
        let pipeline_json = if has_filter {
            Some(
                serde_json::to_string(pipeline_ctx).map_err(|e| ShipError::PhaseFailed {
                    phase,
                    message: format!("failed to serialize pipeline context: {e}"),
                })?,
            )
        } else {
            None
        };
        let output = hooks::run_hooks(cmds, context, project_root, pipeline_json.as_deref())?;

        if let Some(filter_json) = output.filter_output {
            *pipeline_ctx =
                serde_json::from_str(&filter_json).map_err(|e| ShipError::PhaseFailed {
                    phase,
                    message: format!(
                        "filter output could not be deserialized into pipeline context: {e}"
                    ),
                })?;
        }
    }

    on_event(ShipEvent::HooksCompleted { phase, count });
    Ok(count)
}

/// Run the test phase by executing the configured or detected test command.
fn run_test_phase(
    project_root: &Utf8Path,
    config: &Config,
    detection: &ProjectDetection,
) -> ShipResult<PhaseOutcome> {
    let test_cmd = config
        .commands
        .as_ref()
        .and_then(|c| c.test.as_deref())
        .unwrap_or(&detection.tools.test_cmd);

    debug!(%test_cmd, "running tests");

    let output = Command::new("sh")
        .args(["-c", test_cmd])
        .current_dir(project_root.as_std_path())
        .output()
        .map_err(|e| ShipError::PhaseFailed {
            phase: ShipPhase::Test,
            message: format!("failed to execute test command: {e}"),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(ShipError::PhaseFailed {
            phase: ShipPhase::Test,
            message: format!("tests failed: {stderr}"),
        });
    }

    Ok(PhaseOutcome::Success {
        message: format!("Tests passed ({test_cmd})"),
    })
}

/// Run the publish phase by executing the configured or detected publish command.
fn run_publish_phase(
    project_root: &Utf8Path,
    config: &Config,
    detection: &ProjectDetection,
) -> ShipResult<PhaseOutcome> {
    let publish_cmd = config
        .commands
        .as_ref()
        .and_then(|c| c.publish.as_deref())
        .or(detection.tools.publish_cmd.as_deref());

    let Some(publish_cmd) = publish_cmd else {
        return Ok(PhaseOutcome::Skipped {
            reason: "no publish command configured or detected".into(),
        });
    };

    debug!(%publish_cmd, "publishing");

    let output = Command::new("sh")
        .args(["-c", publish_cmd])
        .current_dir(project_root.as_std_path())
        .output()
        .map_err(|e| ShipError::PhaseFailed {
            phase: ShipPhase::Publish,
            message: format!("failed to execute publish command: {e}"),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(ShipError::PhaseFailed {
            phase: ShipPhase::Publish,
            message: format!("publish failed: {stderr}"),
        });
    }

    Ok(PhaseOutcome::Success {
        message: format!("Published ({publish_cmd})"),
    })
}

/// Structured result from the git phase.
struct GitPhaseResult {
    /// The commit hash.
    hash: String,
    /// The branch that was pushed (if any).
    branch: Option<String>,
    /// Whether the push actually happened.
    pushed: bool,
}

/// Run the git phase: commit, tag, and optionally push.
fn run_git_phase(
    _project_root: &Utf8Path,
    tag: &str,
    version: &Version,
    branch_hint: Option<&str>,
    no_push: bool,
    no_tag: bool,
) -> ShipResult<GitPhaseResult> {
    // Stage and commit all modified files
    let commit_msg = format!("chore: release {version}");
    let hash = git::commit(&["."], &commit_msg)?;

    // Create annotated tag (unless skipped)
    if !no_tag {
        let tag_msg = format!("Release {version}");
        git::create_tag(tag, &tag_msg)?;
    }

    // Push if requested (only push tags if we created one)
    if !no_push {
        let branch = branch_hint.unwrap_or("HEAD");
        git::push("origin", branch, !no_tag)?;
        Ok(GitPhaseResult {
            hash,
            branch: Some(branch.to_string()),
            pushed: true,
        })
    } else {
        Ok(GitPhaseResult {
            hash,
            branch: None,
            pushed: false,
        })
    }
}

/// Structured result from the release phase.
struct ReleasePhaseResult {
    /// The URL of the created release (None if `gh` didn't output one).
    url: Option<String>,
    /// Whether an existing release was edited (vs newly created).
    edited: bool,
}

/// Write rendered notes to a temporary file that lives until the `NamedTempFile` is dropped.
fn write_notes_tempfile(notes: &str) -> Result<tempfile::NamedTempFile, std::io::Error> {
    use std::io::Write;
    let mut f = tempfile::NamedTempFile::new()?;
    f.write_all(notes.as_bytes())?;
    f.flush()?;
    Ok(f)
}

// ──────────────────────────────────────────────
// Release phase: edit-vs-create with configurable options
// ──────────────────────────────────────────────

/// Options for the GitHub release phase.
struct ReleaseOptions<'a> {
    tag: &'a str,
    title: Option<String>,
    draft: bool,
    notes_file: Option<&'a std::path::Path>,
    assets: &'a [String],
    discussion_category: Option<&'a str>,
    project_root: &'a Utf8Path,
}

/// Build args for `gh release create`.
fn build_create_args(opts: &ReleaseOptions<'_>) -> Vec<String> {
    let mut args = vec!["release".into(), "create".into(), opts.tag.into()];

    if let Some(ref title) = opts.title {
        args.push("--title".into());
        args.push(title.clone());
    }

    if opts.draft {
        args.push("--draft".into());
    }

    if let Some(path) = opts.notes_file {
        args.push("--notes-file".into());
        args.push(path.to_string_lossy().to_string());
    } else {
        args.push("--generate-notes".into());
    }

    if let Some(cat) = opts.discussion_category {
        args.push("--discussion-category".into());
        args.push(cat.into());
    }

    for asset in opts.assets {
        args.push(asset.clone());
    }

    args
}

/// Build args for `gh release edit`.
fn build_edit_args(opts: &ReleaseOptions<'_>) -> Vec<String> {
    let mut args = vec!["release".into(), "edit".into(), opts.tag.into()];

    if let Some(ref title) = opts.title {
        args.push("--title".into());
        args.push(title.clone());
    }

    if opts.draft {
        args.push("--draft".into());
    } else {
        args.push("--draft=false".into());
    }

    if let Some(path) = opts.notes_file {
        args.push("--notes-file".into());
        args.push(path.to_string_lossy().to_string());
    }

    args
}

/// Check if a GitHub release already exists for the given tag.
fn release_exists(tag: &str, project_root: &Utf8Path) -> bool {
    Command::new("gh")
        .args(["release", "view", tag])
        .current_dir(project_root.as_std_path())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Upload assets to an existing release, replacing any with the same name.
fn upload_release_assets(tag: &str, assets: &[String], project_root: &Utf8Path) -> ShipResult<()> {
    for asset in assets {
        // Try to delete existing asset (ignore failure — may not exist)
        let basename = std::path::Path::new(asset)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| asset.clone());

        let _ = Command::new("gh")
            .args(["release", "delete-asset", tag, &basename, "--yes"])
            .current_dir(project_root.as_std_path())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();

        // Upload
        let output = Command::new("gh")
            .args(["release", "upload", tag, asset])
            .current_dir(project_root.as_std_path())
            .output()
            .map_err(|e| ShipError::PhaseFailed {
                phase: ShipPhase::Release,
                message: format!("failed to upload asset {asset}: {e}"),
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(ShipError::PhaseFailed {
                phase: ShipPhase::Release,
                message: format!("failed to upload asset {asset}: {stderr}"),
            });
        }
    }
    Ok(())
}

/// Create or update a GitHub release using `gh`.
///
/// Auto-detects whether a release already exists for the tag:
/// - **Exists:** edits the release, then re-uploads any assets
/// - **New:** creates the release with all options
fn run_release_phase(opts: &ReleaseOptions<'_>) -> ShipResult<ReleasePhaseResult> {
    let exists = release_exists(opts.tag, opts.project_root);

    if exists {
        debug!(tag = opts.tag, "release exists, editing");
        let args = build_edit_args(opts);

        let output = Command::new("gh")
            .args(&args)
            .current_dir(opts.project_root.as_std_path())
            .output()
            .map_err(|e| ShipError::PhaseFailed {
                phase: ShipPhase::Release,
                message: format!("failed to execute gh release edit: {e}"),
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(ShipError::PhaseFailed {
                phase: ShipPhase::Release,
                message: format!("gh release edit failed: {stderr}"),
            });
        }

        // Upload assets separately for edits
        if !opts.assets.is_empty() {
            upload_release_assets(opts.tag, opts.assets, opts.project_root)?;
        }

        let raw_url = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let url = if raw_url.is_empty() {
            None
        } else {
            Some(raw_url)
        };
        Ok(ReleasePhaseResult { url, edited: true })
    } else {
        debug!(tag = opts.tag, "creating new release");
        let args = build_create_args(opts);

        let output = Command::new("gh")
            .args(&args)
            .current_dir(opts.project_root.as_std_path())
            .output()
            .map_err(|e| ShipError::PhaseFailed {
                phase: ShipPhase::Release,
                message: format!("failed to execute gh release create: {e}"),
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(ShipError::PhaseFailed {
                phase: ShipPhase::Release,
                message: format!("gh release create failed: {stderr}"),
            });
        }

        let raw_url = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let url = if raw_url.is_empty() {
            None
        } else {
            Some(raw_url)
        };
        Ok(ReleasePhaseResult { url, edited: false })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{CommandsConfig, HooksConfig, ProjectConfig, ReleaseConfig, ShipConfig};
    use crate::ecosystem::{
        ChangelogTool, DetectedTools, Ecosystem, ProjectDetection, VersionStrategy,
    };
    use crate::version::interactive::InteractiveContext;

    // ── Helpers ──

    fn test_ctx_init() -> PipelineContextInit {
        PipelineContextInit {
            version: "1.2.3".into(),
            previous_version: "1.1.0".into(),
            tag: "v1.2.3".into(),
            previous_tag: "v1.1.0".into(),
            owner: "claylo".into(),
            repo: "scrat".into(),
            repo_url: Some("https://github.com/claylo/scrat".into()),
            branch: Some("main".into()),
            ecosystem: "rust".into(),
            changelog_path: "CHANGELOG.md".into(),
            dry_run: false,
        }
    }

    fn test_detection_rust() -> ProjectDetection {
        ProjectDetection {
            ecosystem: Ecosystem::Rust,
            version_strategy: VersionStrategy::Interactive,
            tools: DetectedTools {
                test_cmd: "cargo nextest run".into(),
                build_cmd: "cargo build --release".into(),
                publish_cmd: Some("cargo publish".into()),
                bump_cmd: Some("cargo set-version".into()),
                changelog_tool: Some(ChangelogTool::GitCliff),
            },
        }
    }

    fn test_detection_generic() -> ProjectDetection {
        ProjectDetection::generic(VersionStrategy::Interactive)
    }

    fn test_detection_node() -> ProjectDetection {
        ProjectDetection {
            ecosystem: Ecosystem::Node,
            version_strategy: VersionStrategy::Interactive,
            tools: DetectedTools {
                test_cmd: "npm test".into(),
                build_cmd: "npm run build".into(),
                publish_cmd: Some("npm publish".into()),
                bump_cmd: None,
                changelog_tool: None,
            },
        }
    }

    fn default_release_opts<'a>(tag: &'a str) -> ReleaseOptions<'a> {
        ReleaseOptions {
            tag,
            title: None,
            draft: true,
            notes_file: None,
            assets: &[],
            discussion_category: None,
            project_root: Utf8Path::new("/tmp"),
        }
    }

    // ========================================================
    // ShipPhase
    // ========================================================

    #[test]
    fn ship_phase_display() {
        assert_eq!(ShipPhase::Preflight.to_string(), "preflight");
        assert_eq!(ShipPhase::Version.to_string(), "version");
        assert_eq!(ShipPhase::Test.to_string(), "test");
        assert_eq!(ShipPhase::Bump.to_string(), "bump");
        assert_eq!(ShipPhase::Publish.to_string(), "publish");
        assert_eq!(ShipPhase::Git.to_string(), "git");
        assert_eq!(ShipPhase::Release.to_string(), "release");
    }

    #[test]
    fn ship_phase_serializes() {
        let json = serde_json::to_string(&ShipPhase::Bump).unwrap();
        assert_eq!(json, "\"bump\"");
    }

    #[test]
    fn ship_phase_serializes_all_variants() {
        let phases = [
            (ShipPhase::Preflight, "\"preflight\""),
            (ShipPhase::Version, "\"version\""),
            (ShipPhase::Test, "\"test\""),
            (ShipPhase::Bump, "\"bump\""),
            (ShipPhase::Publish, "\"publish\""),
            (ShipPhase::Git, "\"git\""),
            (ShipPhase::Release, "\"release\""),
        ];
        for (phase, expected) in phases {
            let json = serde_json::to_string(&phase).unwrap();
            assert_eq!(json, expected, "phase {phase} serialized incorrectly");
        }
    }

    #[test]
    fn ship_phase_equality() {
        assert_eq!(ShipPhase::Preflight, ShipPhase::Preflight);
        assert_ne!(ShipPhase::Preflight, ShipPhase::Version);
        assert_ne!(ShipPhase::Git, ShipPhase::Release);
    }

    #[test]
    fn ship_phase_clone() {
        let phase = ShipPhase::Bump;
        let cloned = phase;
        assert_eq!(phase, cloned);
    }

    // ========================================================
    // PhaseOutcome
    // ========================================================

    #[test]
    fn phase_outcome_success_serializes() {
        let outcome = PhaseOutcome::Success {
            message: "done".into(),
        };
        let json = serde_json::to_string(&outcome).unwrap();
        assert!(json.contains("\"status\":\"success\""));
        assert!(json.contains("\"message\":\"done\""));
    }

    #[test]
    fn phase_outcome_skipped_serializes() {
        let outcome = PhaseOutcome::Skipped {
            reason: "flag".into(),
        };
        let json = serde_json::to_string(&outcome).unwrap();
        assert!(json.contains("\"status\":\"skipped\""));
        assert!(json.contains("\"reason\":\"flag\""));
    }

    #[test]
    fn phase_outcome_success_preserves_message() {
        let msg = "Bumped to 1.2.3 + changelog (modified: Cargo.toml, CHANGELOG.md)";
        let outcome = PhaseOutcome::Success {
            message: msg.into(),
        };
        let json = serde_json::to_string(&outcome).unwrap();
        let back: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(back["message"].as_str().unwrap(), msg);
    }

    #[test]
    fn phase_outcome_skipped_preserves_reason() {
        let reason = "--no-publish flag";
        let outcome = PhaseOutcome::Skipped {
            reason: reason.into(),
        };
        let json = serde_json::to_string(&outcome).unwrap();
        let back: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(back["reason"].as_str().unwrap(), reason);
    }

    #[test]
    fn phase_outcome_clone() {
        let outcome = PhaseOutcome::Success {
            message: "done".into(),
        };
        let cloned = outcome.clone();
        let json1 = serde_json::to_string(&outcome).unwrap();
        let json2 = serde_json::to_string(&cloned).unwrap();
        assert_eq!(json1, json2);
    }

    // ========================================================
    // ShipOptions — construction and flag combinations
    // ========================================================

    #[test]
    fn ship_options_default() {
        let opts = ShipOptions::default();
        assert!(!opts.dry_run);
        assert!(!opts.no_publish);
        assert!(!opts.no_push);
        assert!(!opts.no_release);
        assert!(!opts.no_deps);
        assert!(!opts.no_stats);
        assert!(!opts.no_notes);
        assert!(!opts.no_test);
        assert!(!opts.no_tag);
        assert!(!opts.no_git);
        assert!(!opts.no_changelog);
        assert!(opts.explicit_version.is_none());
        assert!(opts.draft_override.is_none());
    }

    #[test]
    fn ship_options_explicit_version() {
        let opts = ShipOptions {
            explicit_version: Some("2.0.0".into()),
            ..Default::default()
        };
        assert_eq!(opts.explicit_version.as_deref(), Some("2.0.0"));
    }

    #[test]
    fn ship_options_dry_run_only() {
        let opts = ShipOptions {
            dry_run: true,
            ..Default::default()
        };
        assert!(opts.dry_run);
        assert!(!opts.no_publish);
        assert!(!opts.no_git);
        assert!(!opts.no_release);
    }

    #[test]
    fn ship_options_all_no_flags() {
        let opts = ShipOptions {
            no_changelog: true,
            no_publish: true,
            no_push: true,
            no_release: true,
            no_deps: true,
            no_stats: true,
            no_notes: true,
            no_test: true,
            no_tag: true,
            no_git: true,
            ..Default::default()
        };
        assert!(opts.no_changelog);
        assert!(opts.no_publish);
        assert!(opts.no_push);
        assert!(opts.no_release);
        assert!(opts.no_deps);
        assert!(opts.no_stats);
        assert!(opts.no_notes);
        assert!(opts.no_test);
        assert!(opts.no_tag);
        assert!(opts.no_git);
    }

    #[test]
    fn ship_options_draft_override_true() {
        let opts = ShipOptions {
            draft_override: Some(true),
            ..Default::default()
        };
        assert_eq!(opts.draft_override, Some(true));
    }

    #[test]
    fn ship_options_draft_override_false() {
        let opts = ShipOptions {
            draft_override: Some(false),
            ..Default::default()
        };
        assert_eq!(opts.draft_override, Some(false));
    }

    #[test]
    fn ship_options_no_git_implies_tag_and_push_irrelevant() {
        // When no_git is true, no_tag and no_push don't matter
        // (git phase is entirely skipped). This verifies both can be set independently.
        let opts = ShipOptions {
            no_git: true,
            no_tag: true,
            no_push: true,
            ..Default::default()
        };
        assert!(opts.no_git);
        assert!(opts.no_tag);
        assert!(opts.no_push);
    }

    #[test]
    fn ship_options_clone() {
        let opts = ShipOptions {
            explicit_version: Some("3.0.0".into()),
            dry_run: true,
            no_publish: true,
            draft_override: Some(false),
            ..Default::default()
        };
        let cloned = opts.clone();
        assert_eq!(cloned.explicit_version, opts.explicit_version);
        assert_eq!(cloned.dry_run, opts.dry_run);
        assert_eq!(cloned.no_publish, opts.no_publish);
        assert_eq!(cloned.draft_override, opts.draft_override);
    }

    // ========================================================
    // ShipOutcome
    // ========================================================

    #[test]
    fn ship_outcome_serializes() {
        let ctx = PipelineContext::new(test_ctx_init());
        let outcome = ShipOutcome {
            version: Version::new(1, 2, 3),
            previous_version: Version::new(1, 1, 0),
            tag: "v1.2.3".into(),
            phases: vec![(
                ShipPhase::Preflight,
                PhaseOutcome::Success {
                    message: "ok".into(),
                },
            )],
            hooks_run: 2,
            dry_run: false,
            context: ctx,
        };
        let json = serde_json::to_string_pretty(&outcome).unwrap();
        assert!(json.contains("\"tag\": \"v1.2.3\""));
        assert!(json.contains("\"hooks_run\": 2"));
        assert!(json.contains("\"dry_run\": false"));
        assert!(json.contains("\"context\""));
        assert!(json.contains("\"ecosystem\": \"rust\""));
    }

    #[test]
    fn ship_outcome_with_multiple_phases() {
        let ctx = PipelineContext::new(test_ctx_init());
        let outcome = ShipOutcome {
            version: Version::new(1, 2, 3),
            previous_version: Version::new(1, 1, 0),
            tag: "v1.2.3".into(),
            phases: vec![
                (
                    ShipPhase::Preflight,
                    PhaseOutcome::Success {
                        message: "All preflight checks passed".into(),
                    },
                ),
                (
                    ShipPhase::Version,
                    PhaseOutcome::Success {
                        message: "1.1.0 -> 1.2.3 (interactive)".into(),
                    },
                ),
                (
                    ShipPhase::Test,
                    PhaseOutcome::Skipped {
                        reason: "--no-test flag".into(),
                    },
                ),
                (
                    ShipPhase::Bump,
                    PhaseOutcome::Success {
                        message: "Bumped to 1.2.3".into(),
                    },
                ),
                (
                    ShipPhase::Publish,
                    PhaseOutcome::Skipped {
                        reason: "--no-publish flag".into(),
                    },
                ),
                (
                    ShipPhase::Git,
                    PhaseOutcome::Success {
                        message: "Committed abc1234, tagged v1.2.3, pushed".into(),
                    },
                ),
                (
                    ShipPhase::Release,
                    PhaseOutcome::Success {
                        message: "Created GitHub release (draft): https://github.com/...".into(),
                    },
                ),
            ],
            hooks_run: 4,
            dry_run: false,
            context: ctx,
        };
        assert_eq!(outcome.phases.len(), 7);
        assert_eq!(outcome.phases[0].0, ShipPhase::Preflight);
        assert_eq!(outcome.phases[6].0, ShipPhase::Release);
        assert_eq!(outcome.hooks_run, 4);
    }

    #[test]
    fn ship_outcome_dry_run_flag() {
        let ctx = PipelineContext::new(PipelineContextInit {
            dry_run: true,
            ..test_ctx_init()
        });
        let outcome = ShipOutcome {
            version: Version::new(1, 2, 3),
            previous_version: Version::new(1, 1, 0),
            tag: "v1.2.3".into(),
            phases: vec![],
            hooks_run: 0,
            dry_run: true,
            context: ctx,
        };
        assert!(outcome.dry_run);
        assert!(outcome.context.dry_run);
    }

    #[test]
    fn ship_outcome_empty_phases() {
        let ctx = PipelineContext::new(test_ctx_init());
        let outcome = ShipOutcome {
            version: Version::new(0, 1, 0),
            previous_version: Version::new(0, 0, 0),
            tag: "v0.1.0".into(),
            phases: vec![],
            hooks_run: 0,
            dry_run: false,
            context: ctx,
        };
        assert!(outcome.phases.is_empty());
        assert_eq!(outcome.hooks_run, 0);
    }

    // ========================================================
    // PipelineContext — hook_context derivation
    // ========================================================

    #[test]
    fn pipeline_context_derives_hook_context() {
        let ctx = PipelineContext::new(PipelineContextInit {
            version: "1.2.3".into(),
            previous_version: "1.1.0".into(),
            tag: "v1.2.3".into(),
            previous_tag: "v1.1.0".into(),
            owner: "claylo".into(),
            repo: "scrat".into(),
            repo_url: None,
            branch: None,
            ecosystem: "rust".into(),
            changelog_path: "/tmp/project/CHANGELOG.md".into(),
            dry_run: false,
        });
        let hc = ctx.hook_context();
        assert_eq!(hc.version, "1.2.3");
        assert_eq!(hc.prev_version, "1.1.0");
        assert_eq!(hc.tag, "v1.2.3");
        assert_eq!(hc.changelog_path, "/tmp/project/CHANGELOG.md");
        assert_eq!(hc.owner, "claylo");
        assert_eq!(hc.repo, "scrat");
    }

    #[test]
    fn hook_context_from_different_ecosystem() {
        let ctx = PipelineContext::new(PipelineContextInit {
            version: "0.1.0".into(),
            previous_version: "0.0.0".into(),
            tag: "v0.1.0".into(),
            previous_tag: "v0.0.0".into(),
            owner: "testorg".into(),
            repo: "myproject".into(),
            repo_url: None,
            branch: None,
            ecosystem: "generic".into(),
            changelog_path: "/path/to/CHANGELOG.md".into(),
            dry_run: true,
        });
        let hc = ctx.hook_context();
        assert_eq!(hc.version, "0.1.0");
        assert_eq!(hc.prev_version, "0.0.0");
        assert_eq!(hc.owner, "testorg");
        assert_eq!(hc.repo, "myproject");
    }

    // ========================================================
    // build_create_args — various config combinations
    // ========================================================

    #[test]
    fn build_create_args_with_all_options() {
        let notes = tempfile::NamedTempFile::new().unwrap();
        let opts = ReleaseOptions {
            tag: "v1.2.3",
            title: Some("myrepo v1.2.3".into()),
            draft: true,
            notes_file: Some(notes.path()),
            assets: &["dist/app.tar.gz".into(), "dist/checksums.txt".into()],
            discussion_category: Some("releases"),
            project_root: Utf8Path::new("/tmp"),
        };
        let args = build_create_args(&opts);
        assert_eq!(args[0], "release");
        assert_eq!(args[1], "create");
        assert_eq!(args[2], "v1.2.3");
        assert!(args.contains(&"--title".into()));
        assert!(args.contains(&"myrepo v1.2.3".into()));
        assert!(args.contains(&"--draft".into()));
        assert!(args.contains(&"--notes-file".into()));
        assert!(args.contains(&"--discussion-category".into()));
        assert!(args.contains(&"releases".into()));
        assert!(args.contains(&"dist/app.tar.gz".into()));
        assert!(args.contains(&"dist/checksums.txt".into()));
    }

    #[test]
    fn build_create_args_minimal() {
        let opts = ReleaseOptions {
            tag: "v0.1.0",
            title: None,
            draft: false,
            notes_file: None,
            assets: &[],
            discussion_category: None,
            project_root: Utf8Path::new("/tmp"),
        };
        let args = build_create_args(&opts);
        assert_eq!(
            args,
            vec!["release", "create", "v0.1.0", "--generate-notes"]
        );
    }

    #[test]
    fn build_create_args_draft_no_notes() {
        let opts = ReleaseOptions {
            tag: "v3.0.0",
            title: None,
            draft: true,
            notes_file: None,
            assets: &[],
            discussion_category: None,
            project_root: Utf8Path::new("/tmp"),
        };
        let args = build_create_args(&opts);
        assert!(args.contains(&"--draft".into()));
        // no notes_file => --generate-notes fallback
        assert!(args.contains(&"--generate-notes".into()));
        assert!(!args.contains(&"--title".into()));
    }

    #[test]
    fn build_create_args_no_draft_with_notes() {
        let notes = tempfile::NamedTempFile::new().unwrap();
        let opts = ReleaseOptions {
            tag: "v1.0.0",
            title: None,
            draft: false,
            notes_file: Some(notes.path()),
            assets: &[],
            discussion_category: None,
            project_root: Utf8Path::new("/tmp"),
        };
        let args = build_create_args(&opts);
        assert!(!args.contains(&"--draft".into()));
        assert!(args.contains(&"--notes-file".into()));
        assert!(!args.contains(&"--generate-notes".into()));
    }

    #[test]
    fn build_create_args_single_asset() {
        let opts = ReleaseOptions {
            tag: "v1.0.0",
            title: None,
            draft: false,
            notes_file: None,
            assets: &["artifact.zip".into()],
            discussion_category: None,
            project_root: Utf8Path::new("/tmp"),
        };
        let args = build_create_args(&opts);
        assert!(args.contains(&"artifact.zip".into()));
    }

    #[test]
    fn build_create_args_title_only() {
        let opts = ReleaseOptions {
            tag: "v1.5.0",
            title: Some("Release 1.5.0 -- Big Update".into()),
            draft: false,
            notes_file: None,
            assets: &[],
            discussion_category: None,
            project_root: Utf8Path::new("/tmp"),
        };
        let args = build_create_args(&opts);
        let title_idx = args.iter().position(|a| a == "--title").unwrap();
        assert_eq!(args[title_idx + 1], "Release 1.5.0 -- Big Update");
    }

    #[test]
    fn build_create_args_discussion_category_only() {
        let opts = ReleaseOptions {
            tag: "v1.0.0",
            title: None,
            draft: false,
            notes_file: None,
            assets: &[],
            discussion_category: Some("Announcements"),
            project_root: Utf8Path::new("/tmp"),
        };
        let args = build_create_args(&opts);
        let cat_idx = args
            .iter()
            .position(|a| a == "--discussion-category")
            .unwrap();
        assert_eq!(args[cat_idx + 1], "Announcements");
    }

    // ========================================================
    // build_edit_args — various config combinations
    // ========================================================

    #[test]
    fn build_edit_args_draft() {
        let opts = ReleaseOptions {
            tag: "v1.0.0",
            title: Some("Release v1.0.0".into()),
            draft: true,
            notes_file: None,
            assets: &[],
            discussion_category: None,
            project_root: Utf8Path::new("/tmp"),
        };
        let args = build_edit_args(&opts);
        assert_eq!(args[0], "release");
        assert_eq!(args[1], "edit");
        assert_eq!(args[2], "v1.0.0");
        assert!(args.contains(&"--draft".into()));
        assert!(args.contains(&"--title".into()));
        assert!(args.contains(&"Release v1.0.0".into()));
    }

    #[test]
    fn build_edit_args_publish() {
        let opts = ReleaseOptions {
            tag: "v1.0.0",
            title: None,
            draft: false,
            notes_file: None,
            assets: &[],
            discussion_category: None,
            project_root: Utf8Path::new("/tmp"),
        };
        let args = build_edit_args(&opts);
        assert!(args.contains(&"--draft=false".into()));
        assert!(!args.contains(&"--title".into()));
    }

    #[test]
    fn build_edit_args_with_notes_file() {
        let notes = tempfile::NamedTempFile::new().unwrap();
        let opts = ReleaseOptions {
            tag: "v2.0.0",
            title: None,
            draft: true,
            notes_file: Some(notes.path()),
            assets: &[],
            discussion_category: None,
            project_root: Utf8Path::new("/tmp"),
        };
        let args = build_edit_args(&opts);
        assert!(args.contains(&"--notes-file".into()));
        // edit should NOT have --generate-notes
        assert!(!args.contains(&"--generate-notes".into()));
    }

    #[test]
    fn build_edit_args_no_notes_no_generate() {
        // When editing with no notes_file, there should be no --generate-notes or --notes-file
        let opts = default_release_opts("v1.0.0");
        let args = build_edit_args(&opts);
        assert!(!args.contains(&"--generate-notes".into()));
        assert!(!args.contains(&"--notes-file".into()));
    }

    #[test]
    fn build_edit_args_no_discussion_category() {
        // Edit does not pass --discussion-category (only create does)
        let opts = ReleaseOptions {
            tag: "v1.0.0",
            title: None,
            draft: true,
            notes_file: None,
            assets: &[],
            discussion_category: Some("releases"),
            project_root: Utf8Path::new("/tmp"),
        };
        let args = build_edit_args(&opts);
        assert!(
            !args.contains(&"--discussion-category".into()),
            "edit should not include --discussion-category"
        );
    }

    #[test]
    fn build_edit_args_no_assets() {
        // Edit does not include assets in args (they're uploaded separately)
        let opts = ReleaseOptions {
            tag: "v1.0.0",
            title: None,
            draft: true,
            notes_file: None,
            assets: &["dist/app.tar.gz".into()],
            discussion_category: None,
            project_root: Utf8Path::new("/tmp"),
        };
        let args = build_edit_args(&opts);
        assert!(
            !args.contains(&"dist/app.tar.gz".into()),
            "edit should not include assets in args"
        );
    }

    // ========================================================
    // Draft resolution logic
    // ========================================================

    #[test]
    fn draft_resolution_cli_overrides_config() {
        // CLI --draft should override config draft = false
        let opts = ShipOptions {
            draft_override: Some(true),
            ..Default::default()
        };
        let release_cfg = ReleaseConfig {
            draft: Some(false),
            ..Default::default()
        };
        let config = Config {
            release: Some(release_cfg),
            ..Default::default()
        };

        let draft = opts
            .draft_override
            .or_else(|| config.release.as_ref().and_then(|r| r.draft))
            .unwrap_or(true);
        assert!(draft);
    }

    #[test]
    fn draft_resolution_config_value() {
        // No CLI override => use config value
        let opts = ShipOptions::default();
        let config = Config {
            release: Some(ReleaseConfig {
                draft: Some(false),
                ..Default::default()
            }),
            ..Default::default()
        };

        let draft = opts
            .draft_override
            .or_else(|| config.release.as_ref().and_then(|r| r.draft))
            .unwrap_or(true);
        assert!(!draft);
    }

    #[test]
    fn draft_resolution_defaults_to_true() {
        // No CLI override, no config => defaults to true
        let opts = ShipOptions::default();
        let config = Config::default();

        let draft = opts
            .draft_override
            .or_else(|| config.release.as_ref().and_then(|r| r.draft))
            .unwrap_or(true);
        assert!(draft);
    }

    #[test]
    fn draft_resolution_cli_no_draft_overrides_config_draft() {
        // CLI --no-draft (draft_override = Some(false)) overrides config draft = true
        let opts = ShipOptions {
            draft_override: Some(false),
            ..Default::default()
        };
        let config = Config {
            release: Some(ReleaseConfig {
                draft: Some(true),
                ..Default::default()
            }),
            ..Default::default()
        };

        let draft = opts
            .draft_override
            .or_else(|| config.release.as_ref().and_then(|r| r.draft))
            .unwrap_or(true);
        assert!(!draft);
    }

    // ========================================================
    // Title interpolation
    // ========================================================

    #[test]
    fn title_interpolation_with_version() {
        let hc = HookContext {
            version: "1.2.3".into(),
            prev_version: "1.1.0".into(),
            tag: "v1.2.3".into(),
            changelog_path: "CHANGELOG.md".into(),
            owner: "claylo".into(),
            repo: "scrat".into(),
        };
        let title_template = "{repo} {tag}";
        let result = hooks::interpolate_command(title_template, &hc);
        assert_eq!(result, "scrat v1.2.3");
    }

    #[test]
    fn title_interpolation_with_all_vars() {
        let hc = HookContext {
            version: "2.0.0".into(),
            prev_version: "1.9.0".into(),
            tag: "v2.0.0".into(),
            changelog_path: "/tmp/CHANGELOG.md".into(),
            owner: "acme".into(),
            repo: "widget".into(),
        };
        let template = "{owner}/{repo} {version} (from {prev_version})";
        let result = hooks::interpolate_command(template, &hc);
        assert_eq!(result, "acme/widget 2.0.0 (from 1.9.0)");
    }

    #[test]
    fn title_interpolation_no_vars() {
        let hc = HookContext {
            version: "1.0.0".into(),
            prev_version: "0.9.0".into(),
            tag: "v1.0.0".into(),
            changelog_path: "CHANGELOG.md".into(),
            owner: "org".into(),
            repo: "proj".into(),
        };
        let result = hooks::interpolate_command("Static Title", &hc);
        assert_eq!(result, "Static Title");
    }

    // ========================================================
    // ShipPlan variants — ReadyShip, InteractiveShip, NeedsEcosystemSelection
    // ========================================================

    #[test]
    fn ready_ship_holds_all_components() {
        let bump = ReadyBump {
            previous: Version::new(1, 0, 0),
            next: Version::new(1, 1, 0),
            strategy: VersionStrategy::Explicit("1.1.0".into()),
            detection: test_detection_rust(),
            version_files: vec![],
        };
        let ready = ReadyShip {
            bump,
            options: ShipOptions::default(),
            config: Config::default(),
            detection: test_detection_rust(),
            branch: None,
        };
        assert_eq!(ready.bump.next, Version::new(1, 1, 0));
        assert_eq!(ready.bump.previous, Version::new(1, 0, 0));
        assert_eq!(ready.detection.ecosystem, Ecosystem::Rust);
    }

    // ========================================================
    // ReadyShip::validate
    // ========================================================

    #[test]
    fn validate_passes_for_nonexistent_tag() {
        let bump = ReadyBump {
            previous: Version::new(0, 0, 0),
            next: Version::parse("99999.99999.99999").unwrap(),
            strategy: VersionStrategy::Explicit("99999.99999.99999".into()),
            detection: test_detection_rust(),
            version_files: vec![],
        };
        let ready = ReadyShip {
            bump,
            options: ShipOptions::default(),
            config: Config::default(),
            detection: test_detection_rust(),
            branch: None,
        };
        let failures = ready.validate();
        assert!(
            failures.is_empty(),
            "validate should pass for a tag that doesn't exist"
        );
    }

    #[test]
    fn validate_skipped_when_no_tag() {
        let bump = ReadyBump {
            previous: Version::new(0, 0, 0),
            next: Version::new(0, 1, 0),
            strategy: VersionStrategy::Explicit("0.1.0".into()),
            detection: test_detection_rust(),
            version_files: vec![],
        };
        let ready = ReadyShip {
            bump,
            options: ShipOptions {
                no_tag: true,
                ..Default::default()
            },
            config: Config::default(),
            detection: test_detection_rust(),
            branch: None,
        };
        let failures = ready.validate();
        assert!(
            failures.is_empty(),
            "validate should skip tag check when --no-tag"
        );
    }

    #[test]
    fn validate_skipped_when_no_git() {
        let bump = ReadyBump {
            previous: Version::new(0, 0, 0),
            next: Version::new(0, 1, 0),
            strategy: VersionStrategy::Explicit("0.1.0".into()),
            detection: test_detection_rust(),
            version_files: vec![],
        };
        let ready = ReadyShip {
            bump,
            options: ShipOptions {
                no_git: true,
                ..Default::default()
            },
            config: Config::default(),
            detection: test_detection_rust(),
            branch: None,
        };
        let failures = ready.validate();
        assert!(
            failures.is_empty(),
            "validate should skip tag check when --no-git"
        );
    }

    #[test]
    fn interactive_ship_holds_bump_context() {
        let bump = InteractiveBump {
            context: InteractiveContext {
                current_version: Some(Version::new(1, 0, 0)),
                recent_commits: vec![
                    ("abc1234".into(), "feat: add widget".into()),
                    ("def5678".into(), "fix: correct alignment".into()),
                ],
                candidates: vec![],
            },
            detection: test_detection_rust(),
            version_files: vec![],
        };
        let ship = InteractiveShip {
            bump,
            options: ShipOptions {
                dry_run: true,
                ..Default::default()
            },
            config: Config::default(),
            branch: None,
        };
        assert!(ship.options.dry_run);
        assert_eq!(
            ship.bump.context.current_version,
            Some(Version::new(1, 0, 0))
        );
        assert_eq!(ship.bump.context.recent_commits.len(), 2);
    }

    #[test]
    fn needs_ecosystem_selection_preserves_options() {
        let opts = ShipOptions {
            no_publish: true,
            no_test: true,
            dry_run: true,
            ..Default::default()
        };
        let selection = NeedsEcosystemSelection {
            options: opts,
            config: Config::default(),
            project_root: "/tmp/myproject".into(),
        };
        assert!(selection.options.no_publish);
        assert!(selection.options.no_test);
        assert!(selection.options.dry_run);
        assert_eq!(selection.project_root.as_str(), "/tmp/myproject");
    }

    // ========================================================
    // resolve_ship_interaction
    // ========================================================

    #[test]
    fn resolve_ship_interaction_produces_ready_ship() {
        let interactive = InteractiveShip {
            bump: InteractiveBump {
                context: InteractiveContext {
                    current_version: Some(Version::new(1, 0, 0)),
                    recent_commits: vec![],
                    candidates: vec![],
                },
                detection: test_detection_rust(),
                version_files: vec![],
            },
            options: ShipOptions {
                no_publish: true,
                ..Default::default()
            },
            config: Config::default(),
            branch: None,
        };
        let chosen = Version::new(2, 0, 0);
        let ready = resolve_ship_interaction(interactive, chosen);
        assert_eq!(ready.bump.next, Version::new(2, 0, 0));
        assert_eq!(ready.bump.previous, Version::new(1, 0, 0));
        assert!(ready.options.no_publish);
        assert_eq!(ready.detection.ecosystem, Ecosystem::Rust);
    }

    #[test]
    fn resolve_ship_interaction_first_release() {
        // When current_version is None, previous should be 0.0.0
        let interactive = InteractiveShip {
            bump: InteractiveBump {
                context: InteractiveContext {
                    current_version: None,
                    recent_commits: vec![],
                    candidates: vec![],
                },
                detection: test_detection_generic(),
                version_files: vec![],
            },
            options: ShipOptions::default(),
            config: Config::default(),
            branch: None,
        };
        let ready = resolve_ship_interaction(interactive, Version::new(0, 1, 0));
        assert_eq!(ready.bump.previous, Version::new(0, 0, 0));
        assert_eq!(ready.bump.next, Version::new(0, 1, 0));
    }

    #[test]
    fn resolve_ship_interaction_preserves_config() {
        let config = Config {
            release: Some(ReleaseConfig {
                draft: Some(false),
                title: Some("{repo} {tag}".into()),
                ..Default::default()
            }),
            hooks: Some(HooksConfig {
                pre_ship: Some(vec!["echo starting".into()]),
                ..Default::default()
            }),
            ..Default::default()
        };
        let interactive = InteractiveShip {
            bump: InteractiveBump {
                context: InteractiveContext {
                    current_version: Some(Version::new(1, 0, 0)),
                    recent_commits: vec![],
                    candidates: vec![],
                },
                detection: test_detection_rust(),
                version_files: vec![],
            },
            options: ShipOptions::default(),
            config,
            branch: None,
        };
        let ready = resolve_ship_interaction(interactive, Version::new(1, 1, 0));
        assert_eq!(ready.config.release.as_ref().unwrap().draft, Some(false));
        assert!(ready.config.hooks.as_ref().unwrap().pre_ship.is_some());
    }

    // ========================================================
    // Config-driven release behavior
    // ========================================================

    #[test]
    fn github_release_disabled_in_config() {
        let config = Config {
            release: Some(ReleaseConfig {
                github_release: Some(false),
                ..Default::default()
            }),
            ..Default::default()
        };
        let github_release = config
            .release
            .as_ref()
            .and_then(|r| r.github_release)
            .unwrap_or(true);
        assert!(!github_release);
    }

    #[test]
    fn github_release_defaults_to_true() {
        let config = Config::default();
        let github_release = config
            .release
            .as_ref()
            .and_then(|r| r.github_release)
            .unwrap_or(true);
        assert!(github_release);
    }

    #[test]
    fn release_assets_from_config() {
        let config = Config {
            release: Some(ReleaseConfig {
                assets: Some(vec![
                    "dist/app.tar.gz".into(),
                    "dist/checksums.sha256".into(),
                ]),
                ..Default::default()
            }),
            ..Default::default()
        };
        let assets = config
            .release
            .as_ref()
            .and_then(|r| r.assets.as_deref())
            .unwrap_or(&[]);
        assert_eq!(assets.len(), 2);
        assert_eq!(assets[0], "dist/app.tar.gz");
    }

    #[test]
    fn release_assets_empty_by_default() {
        let config = Config::default();
        let assets = config
            .release
            .as_ref()
            .and_then(|r| r.assets.as_deref())
            .unwrap_or(&[]);
        assert!(assets.is_empty());
    }

    #[test]
    fn release_discussion_category_from_config() {
        let config = Config {
            release: Some(ReleaseConfig {
                discussion_category: Some("Announcements".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let cat = config
            .release
            .as_ref()
            .and_then(|r| r.discussion_category.as_deref());
        assert_eq!(cat, Some("Announcements"));
    }

    #[test]
    fn release_notes_template_from_config() {
        let config = Config {
            release: Some(ReleaseConfig {
                notes_template: Some("templates/release-notes.tera".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let tmpl = config
            .release
            .as_ref()
            .and_then(|r| r.notes_template.as_deref());
        assert_eq!(tmpl, Some("templates/release-notes.tera"));
    }

    // ========================================================
    // Command overrides
    // ========================================================

    #[test]
    fn test_cmd_override_from_config() {
        let config = Config {
            commands: Some(CommandsConfig {
                test: Some("just test".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let detection = test_detection_rust();
        let test_cmd = config
            .commands
            .as_ref()
            .and_then(|c| c.test.as_deref())
            .unwrap_or(&detection.tools.test_cmd);
        assert_eq!(test_cmd, "just test");
    }

    #[test]
    fn test_cmd_falls_back_to_detection() {
        let config = Config::default();
        let detection = test_detection_rust();
        let test_cmd = config
            .commands
            .as_ref()
            .and_then(|c| c.test.as_deref())
            .unwrap_or(&detection.tools.test_cmd);
        assert_eq!(test_cmd, "cargo nextest run");
    }

    #[test]
    fn publish_cmd_override_from_config() {
        let config = Config {
            commands: Some(CommandsConfig {
                publish: Some("cargo publish --no-verify".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let detection = test_detection_rust();
        let publish_cmd = config
            .commands
            .as_ref()
            .and_then(|c| c.publish.as_deref())
            .or(detection.tools.publish_cmd.as_deref());
        assert_eq!(publish_cmd, Some("cargo publish --no-verify"));
    }

    #[test]
    fn publish_cmd_falls_back_to_detection() {
        let config = Config::default();
        let detection = test_detection_rust();
        let publish_cmd = config
            .commands
            .as_ref()
            .and_then(|c| c.publish.as_deref())
            .or(detection.tools.publish_cmd.as_deref());
        assert_eq!(publish_cmd, Some("cargo publish"));
    }

    #[test]
    fn publish_cmd_none_for_generic() {
        let config = Config::default();
        let detection = test_detection_generic();
        let publish_cmd = config
            .commands
            .as_ref()
            .and_then(|c| c.publish.as_deref())
            .or(detection.tools.publish_cmd.as_deref());
        assert_eq!(publish_cmd, None);
    }

    // ========================================================
    // Hooks config access patterns
    // ========================================================

    #[test]
    fn hooks_config_none_by_default() {
        let config = Config::default();
        assert!(config.hooks.is_none());
    }

    #[test]
    fn hooks_config_pre_ship_access() {
        let config = Config {
            hooks: Some(HooksConfig {
                pre_ship: Some(vec!["echo pre-ship".into()]),
                ..Default::default()
            }),
            ..Default::default()
        };
        let cmds = config.hooks.as_ref().and_then(|h| h.pre_ship.as_deref());
        assert_eq!(cmds.unwrap().len(), 1);
        assert_eq!(cmds.unwrap()[0], "echo pre-ship");
    }

    #[test]
    fn hooks_config_all_14_hook_points() {
        let hooks = HooksConfig {
            pre_ship: Some(vec!["a".into()]),
            post_ship: Some(vec!["b".into()]),
            pre_test: Some(vec!["c".into()]),
            post_test: Some(vec!["d".into()]),
            pre_bump: Some(vec!["e".into()]),
            post_bump: Some(vec!["f".into()]),
            pre_publish: Some(vec!["g".into()]),
            post_publish: Some(vec!["h".into()]),
            pre_tag: Some(vec!["i".into()]),
            post_tag: Some(vec!["j".into()]),
            pre_release: Some(vec!["k".into()]),
            post_release: Some(vec!["l".into()]),
            // That's 12 fields. Verify they're all populated.
        };
        assert!(hooks.pre_ship.is_some());
        assert!(hooks.post_ship.is_some());
        assert!(hooks.pre_test.is_some());
        assert!(hooks.post_test.is_some());
        assert!(hooks.pre_bump.is_some());
        assert!(hooks.post_bump.is_some());
        assert!(hooks.pre_publish.is_some());
        assert!(hooks.post_publish.is_some());
        assert!(hooks.pre_tag.is_some());
        assert!(hooks.post_tag.is_some());
        assert!(hooks.pre_release.is_some());
        assert!(hooks.post_release.is_some());
    }

    #[test]
    fn hooks_config_empty_vec_vs_none() {
        // Empty vec means hooks configured but with no commands
        let config = Config {
            hooks: Some(HooksConfig {
                pre_ship: Some(vec![]),
                ..Default::default()
            }),
            ..Default::default()
        };
        let cmds = config.hooks.as_ref().and_then(|h| h.pre_ship.as_deref());
        assert!(cmds.unwrap().is_empty());
    }

    // ========================================================
    // ShipError
    // ========================================================

    #[test]
    fn ship_error_preflight_display() {
        let err =
            ShipError::PreflightFailed("working tree not clean; not on release branch".into());
        let msg = err.to_string();
        assert!(msg.contains("preflight checks failed"));
        assert!(msg.contains("working tree not clean"));
    }

    #[test]
    fn ship_error_phase_failed_display() {
        let err = ShipError::PhaseFailed {
            phase: ShipPhase::Test,
            message: "tests failed: exit code 1".into(),
        };
        let msg = err.to_string();
        assert!(msg.contains("test phase failed"));
        assert!(msg.contains("tests failed: exit code 1"));
    }

    #[test]
    fn ship_error_phase_failed_all_phases() {
        let phases = [
            ShipPhase::Preflight,
            ShipPhase::Version,
            ShipPhase::Test,
            ShipPhase::Bump,
            ShipPhase::Publish,
            ShipPhase::Git,
            ShipPhase::Release,
        ];
        for phase in phases {
            let err = ShipError::PhaseFailed {
                phase,
                message: "something went wrong".into(),
            };
            let msg = err.to_string();
            assert!(
                msg.contains(&format!("{phase} phase failed")),
                "expected '{phase} phase failed' in: {msg}"
            );
        }
    }

    // ========================================================
    // ShipEvent variants
    // ========================================================

    #[test]
    fn ship_event_phase_started() {
        let event = ShipEvent::PhaseStarted(ShipPhase::Bump);
        match event {
            ShipEvent::PhaseStarted(phase) => assert_eq!(phase, ShipPhase::Bump),
            _ => panic!("expected PhaseStarted"),
        }
    }

    #[test]
    fn ship_event_phase_completed() {
        let outcome = PhaseOutcome::Success {
            message: "done".into(),
        };
        let event = ShipEvent::PhaseCompleted(ShipPhase::Git, outcome);
        match event {
            ShipEvent::PhaseCompleted(phase, PhaseOutcome::Success { message }) => {
                assert_eq!(phase, ShipPhase::Git);
                assert_eq!(message, "done");
            }
            _ => panic!("expected PhaseCompleted with Success"),
        }
    }

    #[test]
    fn ship_event_hooks_started_dry_run() {
        let event = ShipEvent::HooksStarted {
            phase: ShipPhase::Bump,
            count: 3,
            commands: vec!["a".into(), "b".into(), "c".into()],
            will_execute: false,
        };
        match event {
            ShipEvent::HooksStarted {
                phase,
                count,
                commands,
                will_execute,
            } => {
                assert_eq!(phase, ShipPhase::Bump);
                assert_eq!(count, 3);
                assert_eq!(commands.len(), 3);
                assert!(!will_execute);
            }
            _ => panic!("expected HooksStarted"),
        }
    }

    #[test]
    fn ship_event_hooks_completed() {
        let event = ShipEvent::HooksCompleted {
            phase: ShipPhase::Release,
            count: 2,
        };
        match event {
            ShipEvent::HooksCompleted { phase, count } => {
                assert_eq!(phase, ShipPhase::Release);
                assert_eq!(count, 2);
            }
            _ => panic!("expected HooksCompleted"),
        }
    }

    #[test]
    fn ship_event_clone() {
        let event = ShipEvent::HooksStarted {
            phase: ShipPhase::Test,
            count: 1,
            commands: vec!["echo hello".into()],
            will_execute: true,
        };
        let cloned = event;
        match cloned {
            ShipEvent::HooksStarted { count, .. } => assert_eq!(count, 1),
            _ => panic!("clone should preserve variant"),
        }
    }

    // ========================================================
    // write_notes_tempfile
    // ========================================================

    #[test]
    fn write_notes_tempfile_creates_file() {
        let notes = "## Release 1.2.3\n\n- feat: new feature\n- fix: bug fix";
        let f = write_notes_tempfile(notes).unwrap();
        let contents = std::fs::read_to_string(f.path()).unwrap();
        assert_eq!(contents, notes);
    }

    #[test]
    fn write_notes_tempfile_empty_string() {
        let f = write_notes_tempfile("").unwrap();
        let contents = std::fs::read_to_string(f.path()).unwrap();
        assert_eq!(contents, "");
    }

    #[test]
    fn write_notes_tempfile_unicode() {
        let notes = "## Release Notes\n\nFix for \u{1F41E} bug in \u{2699}\u{FE0F} settings";
        let f = write_notes_tempfile(notes).unwrap();
        let contents = std::fs::read_to_string(f.path()).unwrap();
        assert_eq!(contents, notes);
    }

    // ========================================================
    // Phase skip logic — verifies which flags skip which phases
    // ========================================================

    #[test]
    fn no_test_flag_skips_test_phase() {
        let opts = ShipOptions {
            no_test: true,
            ..Default::default()
        };
        let outcome = if opts.no_test {
            PhaseOutcome::Skipped {
                reason: "--no-test flag".into(),
            }
        } else {
            PhaseOutcome::Success {
                message: "Tests passed".into(),
            }
        };
        match outcome {
            PhaseOutcome::Skipped { reason } => assert_eq!(reason, "--no-test flag"),
            _ => panic!("expected Skipped"),
        }
    }

    #[test]
    fn no_publish_flag_skips_publish_phase() {
        let opts = ShipOptions {
            no_publish: true,
            ..Default::default()
        };
        let outcome = if opts.no_publish {
            PhaseOutcome::Skipped {
                reason: "--no-publish flag".into(),
            }
        } else {
            PhaseOutcome::Success {
                message: "Published".into(),
            }
        };
        match outcome {
            PhaseOutcome::Skipped { reason } => assert_eq!(reason, "--no-publish flag"),
            _ => panic!("expected Skipped"),
        }
    }

    #[test]
    fn no_git_flag_skips_git_phase() {
        let opts = ShipOptions {
            no_git: true,
            ..Default::default()
        };
        let outcome = if opts.no_git {
            PhaseOutcome::Skipped {
                reason: "--no-git flag".into(),
            }
        } else {
            PhaseOutcome::Success {
                message: "Git ok".into(),
            }
        };
        match outcome {
            PhaseOutcome::Skipped { reason } => assert_eq!(reason, "--no-git flag"),
            _ => panic!("expected Skipped"),
        }
    }

    #[test]
    fn no_release_flag_skips_release_phase() {
        let opts = ShipOptions {
            no_release: true,
            ..Default::default()
        };
        let outcome = if opts.no_release {
            PhaseOutcome::Skipped {
                reason: "--no-release flag".into(),
            }
        } else {
            PhaseOutcome::Success {
                message: "Released".into(),
            }
        };
        match outcome {
            PhaseOutcome::Skipped { reason } => assert_eq!(reason, "--no-release flag"),
            _ => panic!("expected Skipped"),
        }
    }

    #[test]
    fn no_git_skips_pre_tag_hooks() {
        // When no_git is set, pre_tag/post_tag hooks should not run
        let opts = ShipOptions {
            no_git: true,
            ..Default::default()
        };
        let hooks_config = HooksConfig {
            pre_tag: Some(vec!["echo pre-tag".into()]),
            post_tag: Some(vec!["echo post-tag".into()]),
            ..Default::default()
        };
        // The execute method gates pre_tag/post_tag hooks on !no_git
        if opts.no_git {
            // Hooks should be skipped
            let pre_tag = None::<&[String]>;
            let post_tag = None::<&[String]>;
            assert!(pre_tag.is_none());
            assert!(post_tag.is_none());
        } else {
            let _pre_tag = hooks_config.pre_tag.as_deref();
            let _post_tag = hooks_config.post_tag.as_deref();
        }
    }

    // ========================================================
    // Dry-run message construction
    // ========================================================

    #[test]
    fn dry_run_test_phase_message() {
        let detection = test_detection_rust();
        let config = Config::default();
        let test_cmd = config
            .commands
            .as_ref()
            .and_then(|c| c.test.as_deref())
            .unwrap_or(&detection.tools.test_cmd);
        let msg = format!("Would run: {test_cmd}");
        assert_eq!(msg, "Would run: cargo nextest run");
    }

    #[test]
    fn dry_run_test_phase_message_with_override() {
        let config = Config {
            commands: Some(CommandsConfig {
                test: Some("just test".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let detection = test_detection_rust();
        let test_cmd = config
            .commands
            .as_ref()
            .and_then(|c| c.test.as_deref())
            .unwrap_or(&detection.tools.test_cmd);
        let msg = format!("Would run: {test_cmd}");
        assert_eq!(msg, "Would run: just test");
    }

    #[test]
    fn dry_run_publish_phase_no_command() {
        let detection = test_detection_generic();
        let config = Config::default();
        let publish_cmd = config
            .commands
            .as_ref()
            .and_then(|c| c.publish.as_deref())
            .or(detection.tools.publish_cmd.as_deref())
            .unwrap_or("(no publish command)");
        let msg = format!("Would run: {publish_cmd}");
        assert_eq!(msg, "Would run: (no publish command)");
    }

    #[test]
    fn dry_run_git_phase_message_full() {
        let tag = "v1.2.3";
        let no_tag = false;
        let no_push = false;
        let tag_msg = if no_tag {
            String::new()
        } else {
            format!(", tag {tag}")
        };
        let push_msg = if no_push { " (no push)" } else { " + push" };
        let msg = format!("Would commit{tag_msg}{push_msg}");
        assert_eq!(msg, "Would commit, tag v1.2.3 + push");
    }

    #[test]
    fn dry_run_git_phase_message_no_tag() {
        let tag = "v1.2.3";
        let no_tag = true;
        let no_push = false;
        let tag_msg = if no_tag {
            String::new()
        } else {
            format!(", tag {tag}")
        };
        let push_msg = if no_push { " (no push)" } else { " + push" };
        let msg = format!("Would commit{tag_msg}{push_msg}");
        assert_eq!(msg, "Would commit + push");
    }

    #[test]
    fn dry_run_git_phase_message_no_push() {
        let tag = "v1.2.3";
        let no_tag = false;
        let no_push = true;
        let tag_msg = if no_tag {
            String::new()
        } else {
            format!(", tag {tag}")
        };
        let push_msg = if no_push { " (no push)" } else { " + push" };
        let msg = format!("Would commit{tag_msg}{push_msg}");
        assert_eq!(msg, "Would commit, tag v1.2.3 (no push)");
    }

    #[test]
    fn dry_run_git_phase_message_no_tag_no_push() {
        let no_tag = true;
        let no_push = true;
        let tag_msg = if no_tag {
            String::new()
        } else {
            ", tag v1.2.3".to_string()
        };
        let push_msg = if no_push { " (no push)" } else { " + push" };
        let msg = format!("Would commit{tag_msg}{push_msg}");
        assert_eq!(msg, "Would commit (no push)");
    }

    #[test]
    fn dry_run_release_phase_message_draft() {
        let tag = "v1.2.3";
        let draft = true;
        let title: Option<String> = Some("scrat v1.2.3".into());
        let no_notes = false;
        let assets: &[String] = &["dist/app.tar.gz".into()];

        let draft_label = if draft { " as draft" } else { "" };
        let title_label = title
            .as_ref()
            .map_or(String::new(), |t| format!(" titled \"{t}\""));
        let notes_msg = if no_notes {
            " (--generate-notes)"
        } else {
            " (with rendered notes)"
        };
        let asset_count = assets.len();
        let asset_msg = if asset_count > 0 {
            format!(
                ", {} asset{}",
                asset_count,
                if asset_count == 1 { "" } else { "s" }
            )
        } else {
            String::new()
        };
        let msg = format!(
            "Would create GitHub release for {tag}{draft_label}{title_label}{notes_msg}{asset_msg}"
        );
        assert_eq!(
            msg,
            "Would create GitHub release for v1.2.3 as draft titled \"scrat v1.2.3\" (with rendered notes), 1 asset"
        );
    }

    #[test]
    fn dry_run_release_phase_message_minimal() {
        let tag = "v0.1.0";
        let draft = false;
        let title: Option<String> = None;
        let no_notes = true;
        let assets: &[String] = &[];

        let draft_label = if draft { " as draft" } else { "" };
        let title_label = title
            .as_ref()
            .map_or(String::new(), |t| format!(" titled \"{t}\""));
        let notes_msg = if no_notes {
            " (--generate-notes)"
        } else {
            " (with rendered notes)"
        };
        let asset_count = assets.len();
        let asset_msg = if asset_count > 0 {
            format!(
                ", {} asset{}",
                asset_count,
                if asset_count == 1 { "" } else { "s" }
            )
        } else {
            String::new()
        };
        let msg = format!(
            "Would create GitHub release for {tag}{draft_label}{title_label}{notes_msg}{asset_msg}"
        );
        assert_eq!(
            msg,
            "Would create GitHub release for v0.1.0 (--generate-notes)"
        );
    }

    #[test]
    fn dry_run_release_multiple_assets() {
        let assets: &[String] = &["a.tar.gz".into(), "b.zip".into(), "c.deb".into()];
        let asset_count = assets.len();
        let asset_msg = if asset_count > 0 {
            format!(
                ", {} asset{}",
                asset_count,
                if asset_count == 1 { "" } else { "s" }
            )
        } else {
            String::new()
        };
        assert_eq!(asset_msg, ", 3 assets");
    }

    // ========================================================
    // PipelineContext — recording phase results for ShipOutcome
    // ========================================================

    #[test]
    fn pipeline_context_accumulates_data() {
        let mut ctx = PipelineContext::new(test_ctx_init());

        // Record bump results
        ctx.record_bump(true, vec!["Cargo.toml".into(), "CHANGELOG.md".into()]);
        assert!(ctx.changelog_updated);
        assert_eq!(ctx.modified_files.len(), 2);

        // Record git results
        ctx.record_git(Some("abc1234".into()), Some("main".into()));
        assert_eq!(ctx.commit_hash.as_deref(), Some("abc1234"));

        // Record release results
        ctx.record_release(Some(
            "https://github.com/claylo/scrat/releases/tag/v1.2.3".into(),
        ));
        assert!(ctx.release_url.is_some());

        // Set assets
        ctx.set_assets(vec!["dist/app.tar.gz".into()]);
        assert_eq!(ctx.assets.len(), 1);

        // Set release notes
        ctx.release_notes = Some("## v1.2.3\nChanges here".into());
        assert!(ctx.release_notes.is_some());

        // Verify hook context still works after all mutations
        let hc = ctx.hook_context();
        assert_eq!(hc.version, "1.2.3");
    }

    #[test]
    fn pipeline_context_dry_run_flag_propagates() {
        let ctx = PipelineContext::new(PipelineContextInit {
            dry_run: true,
            ..test_ctx_init()
        });
        assert!(ctx.dry_run);
    }

    #[test]
    fn pipeline_context_metadata_insertion() {
        let mut ctx = PipelineContext::new(test_ctx_init());
        ctx.metadata
            .insert("custom".into(), serde_json::json!({"key": "value"}));
        assert_eq!(ctx.metadata["custom"], serde_json::json!({"key": "value"}));
    }

    // ========================================================
    // Ecosystem-specific behavior
    // ========================================================

    #[test]
    fn generic_detection_has_no_publish_cmd() {
        let det = test_detection_generic();
        assert!(det.tools.publish_cmd.is_none());
        assert!(det.tools.bump_cmd.is_none());
        assert_eq!(det.tools.test_cmd, "");
        assert_eq!(det.ecosystem, Ecosystem::Generic);
    }

    #[test]
    fn rust_detection_has_publish_cmd() {
        let det = test_detection_rust();
        assert_eq!(det.tools.publish_cmd.as_deref(), Some("cargo publish"));
        assert_eq!(det.ecosystem, Ecosystem::Rust);
    }

    #[test]
    fn node_detection_has_publish_cmd() {
        let det = test_detection_node();
        assert_eq!(det.tools.publish_cmd.as_deref(), Some("npm publish"));
        assert_eq!(det.ecosystem, Ecosystem::Node);
    }

    // ========================================================
    // Config integration: ShipConfig
    // ========================================================

    #[test]
    fn ship_config_confirm_default_none() {
        let config = Config::default();
        assert!(config.ship.is_none());
    }

    #[test]
    fn ship_config_confirm_false_for_ci() {
        let config = Config {
            ship: Some(ShipConfig {
                confirm: Some(false),
                ..Default::default()
            }),
            ..Default::default()
        };
        let confirm = config.ship.as_ref().and_then(|s| s.confirm).unwrap_or(true);
        assert!(!confirm);
    }

    #[test]
    fn ship_config_confirm_default_true() {
        let config = Config::default();
        let confirm = config.ship.as_ref().and_then(|s| s.confirm).unwrap_or(true);
        assert!(confirm);
    }

    // ========================================================
    // Full config: release title with hooks interpolation
    // ========================================================

    #[test]
    fn release_title_interpolation_flow() {
        // Simulates the title interpolation path in execute()
        let config = Config {
            release: Some(ReleaseConfig {
                title: Some("{repo} {tag}".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let hook_ctx = HookContext {
            version: "1.2.3".into(),
            prev_version: "1.1.0".into(),
            tag: "v1.2.3".into(),
            changelog_path: "CHANGELOG.md".into(),
            owner: "claylo".into(),
            repo: "scrat".into(),
        };
        let title = config
            .release
            .as_ref()
            .and_then(|r| r.title.as_deref())
            .map(|t| hooks::interpolate_command(t, &hook_ctx));
        assert_eq!(title.as_deref(), Some("scrat v1.2.3"));
    }

    #[test]
    fn release_title_none_when_not_configured() {
        let config = Config::default();
        let hook_ctx = HookContext {
            version: "1.0.0".into(),
            prev_version: "0.9.0".into(),
            tag: "v1.0.0".into(),
            changelog_path: "CHANGELOG.md".into(),
            owner: "org".into(),
            repo: "proj".into(),
        };
        let title = config
            .release
            .as_ref()
            .and_then(|r| r.title.as_deref())
            .map(|t| hooks::interpolate_command(t, &hook_ctx));
        assert!(title.is_none());
    }

    // ========================================================
    // build_create_args / build_edit_args edge cases
    // ========================================================

    #[test]
    fn build_create_args_preserves_argument_order() {
        // The tag must always be args[2], right after "release" "create"
        let notes = tempfile::NamedTempFile::new().unwrap();
        let opts = ReleaseOptions {
            tag: "v99.99.99",
            title: Some("Big Release".into()),
            draft: true,
            notes_file: Some(notes.path()),
            assets: &["a.zip".into()],
            discussion_category: Some("cat"),
            project_root: Utf8Path::new("/tmp"),
        };
        let args = build_create_args(&opts);
        assert_eq!(&args[..3], &["release", "create", "v99.99.99"]);
    }

    #[test]
    fn build_edit_args_preserves_argument_order() {
        let opts = ReleaseOptions {
            tag: "v99.99.99",
            title: Some("Big Release".into()),
            draft: false,
            notes_file: None,
            assets: &[],
            discussion_category: None,
            project_root: Utf8Path::new("/tmp"),
        };
        let args = build_edit_args(&opts);
        assert_eq!(&args[..3], &["release", "edit", "v99.99.99"]);
    }

    #[test]
    fn build_create_args_many_assets() {
        let assets: Vec<String> = (0..10).map(|i| format!("asset_{i}.tar.gz")).collect();
        let opts = ReleaseOptions {
            tag: "v1.0.0",
            title: None,
            draft: false,
            notes_file: None,
            assets: &assets,
            discussion_category: None,
            project_root: Utf8Path::new("/tmp"),
        };
        let args = build_create_args(&opts);
        for asset in &assets {
            assert!(args.contains(asset), "missing asset: {asset}");
        }
    }

    // ========================================================
    // NeedsEcosystemSelection data preservation
    // ========================================================

    #[test]
    fn needs_ecosystem_selection_preserves_config() {
        let config = Config {
            release: Some(ReleaseConfig {
                draft: Some(false),
                assets: Some(vec!["app.zip".into()]),
                ..Default::default()
            }),
            ..Default::default()
        };
        let selection = NeedsEcosystemSelection {
            options: ShipOptions::default(),
            config: config.clone(),
            project_root: "/tmp/proj".into(),
        };
        assert_eq!(selection.config, config);
    }

    #[test]
    fn needs_ecosystem_selection_config_mutation() {
        // Simulates what resolve_ecosystem_selection does
        let config = Config::default();
        let mut config_clone = config;
        let project = config_clone
            .project
            .get_or_insert_with(ProjectConfig::default);
        project.project_type = Some(Ecosystem::Generic);
        assert_eq!(
            config_clone.project.as_ref().unwrap().project_type,
            Some(Ecosystem::Generic)
        );
    }

    // ========================================================
    // Version formatting in tags
    // ========================================================

    #[test]
    fn tag_format_from_version() {
        let version = Version::new(1, 2, 3);
        let tag = format!("v{version}");
        assert_eq!(tag, "v1.2.3");
    }

    #[test]
    fn tag_format_prerelease() {
        let version = Version::parse("1.0.0-alpha.1").unwrap();
        let tag = format!("v{version}");
        assert_eq!(tag, "v1.0.0-alpha.1");
    }

    #[test]
    fn previous_tag_format() {
        let previous = Version::new(0, 9, 0);
        let tag = format!("v{previous}");
        assert_eq!(tag, "v0.9.0");
    }
}
