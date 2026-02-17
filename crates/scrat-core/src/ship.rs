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
    /// Interactive mode — the CLI must prompt the user.
    NeedsInteraction(InteractiveShip),
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
    // Phase 1: Preflight
    let report = preflight::run_preflight(project_root, config);

    if !report.all_passed {
        let failures: Vec<&str> = report
            .checks
            .iter()
            .filter(|c| !c.passed)
            .map(|c| c.message.as_str())
            .collect();
        return Err(ShipError::PreflightFailed(failures.join("; ")));
    }

    // Phase 2: Version resolution (delegates to bump::plan_bump)
    let bump_plan = bump::plan_bump(project_root, config, options.explicit_version.as_deref())?;

    match bump_plan {
        bump::BumpPlan::Ready(ready_bump) => {
            let detection = ready_bump.detection.clone();
            Ok(ShipPlan::Ready(ReadyShip {
                bump: ready_bump,
                options,
                config: config.clone(),
                detection,
            }))
        }
        bump::BumpPlan::NeedsInteraction(interactive_bump) => {
            Ok(ShipPlan::NeedsInteraction(InteractiveShip {
                bump: interactive_bump,
                options,
                config: config.clone(),
            }))
        }
    }
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
    }
}

// ──────────────────────────────────────────────
// Execute
// ──────────────────────────────────────────────

impl ReadyShip {
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
            branch: git::current_branch().ok().flatten(),
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

        // Render release notes (before creating the release)
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

        // Resolve release config for both dry-run and real execution
        let release_cfg = self.config.release.as_ref();
        let github_release = release_cfg
            .and_then(|r| r.github_release)
            .unwrap_or(true);
        let draft = self
            .options
            .draft_override
            .or_else(|| release_cfg.and_then(|r| r.draft))
            .unwrap_or(true);
        let title = release_cfg
            .and_then(|r| r.title.as_deref())
            .map(|t| hooks::interpolate_command(t, &hook_ctx));
        let discussion_category = release_cfg.and_then(|r| r.discussion_category.as_deref());
        let assets = release_cfg
            .and_then(|r| r.assets.as_deref())
            .unwrap_or(&[]);

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
                assets,
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
        let pipeline_json =
            serde_json::to_string(pipeline_ctx).map_err(|e| ShipError::PhaseFailed {
                phase,
                message: format!("failed to serialize pipeline context: {e}"),
            })?;
        let output = hooks::run_hooks(cmds, context, project_root, Some(&pipeline_json))?;

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
        let branch = git::current_branch()?.unwrap_or_else(|| "HEAD".into());
        git::push("origin", &branch, !no_tag)?;
        Ok(GitPhaseResult {
            hash,
            branch: Some(branch),
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
fn upload_release_assets(
    tag: &str,
    assets: &[String],
    project_root: &Utf8Path,
) -> ShipResult<()> {
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
        let url = if raw_url.is_empty() { None } else { Some(raw_url) };
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
        let url = if raw_url.is_empty() { None } else { Some(raw_url) };
        Ok(ReleasePhaseResult { url, edited: false })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn ship_outcome_serializes() {
        let ctx = PipelineContext::new(PipelineContextInit {
            version: "1.2.3".into(),
            previous_version: "1.1.0".into(),
            tag: "v1.2.3".into(),
            previous_tag: "v1.1.0".into(),
            owner: "claylo".into(),
            repo: "scrat".into(),
            repo_url: None,
            branch: Some("main".into()),
            ecosystem: "rust".into(),
            changelog_path: "CHANGELOG.md".into(),
            dry_run: false,
        });
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
}
