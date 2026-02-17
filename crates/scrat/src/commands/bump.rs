//! Bump command — thin CLI layer over `scrat_core::bump`.

use anyhow::{Context, bail};
use clap::Args;
use inquire::Select;
use owo_colors::OwoColorize;
use tracing::{debug, instrument};

use scrat_core::bump::{self, BumpError, BumpPlan, InteractiveBump};
use scrat_core::config::{Config, ProjectConfig};

/// Arguments for the `bump` subcommand.
#[derive(Args, Debug, Default)]
pub struct BumpArgs {
    /// Set the version explicitly (e.g., "1.2.3" or "v1.2.3")
    #[arg(long, value_name = "VERSION")]
    pub version: Option<String>,

    /// Skip changelog generation
    #[arg(long)]
    pub no_changelog: bool,

    /// Run without making changes (show what would happen)
    #[arg(long)]
    pub dry_run: bool,
}

/// Execute the bump command.
#[instrument(name = "cmd_bump", skip_all, fields(json_output))]
pub fn cmd_bump(
    args: BumpArgs,
    global_json: bool,
    config: &Config,
    cwd: &camino::Utf8Path,
) -> anyhow::Result<()> {
    debug!(json_output = global_json, "executing bump command");

    // Plan the bump (all logic in core)
    // If ecosystem detection fails, prompt the user to select one
    let mut config = config.clone();
    let plan = match bump::plan_bump(cwd, &config, args.version.as_deref()) {
        Ok(plan) => plan,
        Err(BumpError::Detection(_)) => {
            let ecosystem =
                super::prompt_ecosystem_selection().context("ecosystem selection failed")?;
            let project = config.project.get_or_insert_with(ProjectConfig::default);
            project.project_type = Some(ecosystem);
            bump::plan_bump(cwd, &config, args.version.as_deref())
                .context("bump planning failed")?
        }
        Err(e) => return Err(e).context("bump planning failed"),
    };

    // Resolve interactive prompt if needed
    let ready = match plan {
        BumpPlan::Ready(r) => r,
        BumpPlan::NeedsInteraction(interactive) => {
            let chosen = prompt_interactive_version(&interactive)
                .context("interactive version selection failed")?;
            bump::resolve_interactive(interactive, chosen)
        }
    };

    // Display the plan
    if global_json {
        let plan_json = serde_json::json!({
            "previous": ready.previous.to_string(),
            "next": ready.next.to_string(),
            "strategy": ready.strategy.to_string(),
            "ecosystem": ready.detection.ecosystem.to_string(),
            "dry_run": args.dry_run,
        });
        if args.dry_run {
            println!("{}", serde_json::to_string_pretty(&plan_json)?);
            return Ok(());
        }
    } else {
        println!(
            "{}: {} → {}",
            "Version".bold(),
            ready.previous.to_string().dimmed(),
            ready.next.to_string().green().bold()
        );
        println!("{}: {}", "Strategy".dimmed(), ready.strategy);
        println!("{}: {}", "Ecosystem".dimmed(), ready.detection.ecosystem);

        if args.dry_run {
            println!();
            println!("{}", "Dry run — no changes made.".yellow());
            return Ok(());
        }
    }

    // Execute the bump
    let outcome = ready
        .execute(cwd, !args.no_changelog)
        .context("bump failed")?;

    // Display result
    if global_json {
        println!("{}", serde_json::to_string_pretty(&outcome)?);
    } else {
        println!();
        println!(
            "  {} Version updated to {}",
            "✓".green(),
            outcome.new.to_string().green().bold()
        );
        if outcome.changelog_updated {
            println!("  {} Changelog updated", "✓".green());
        }
        for file in &outcome.modified_files {
            println!("  {} {}", "→".dimmed(), file.cyan());
        }
    }

    Ok(())
}

/// Display interactive context and prompt the user to pick a version.
fn prompt_interactive_version(
    plan: &InteractiveBump,
) -> anyhow::Result<scrat_core::semver::Version> {
    let ctx = &plan.context;

    // Show recent commits
    if ctx.recent_commits.is_empty() {
        println!("{}", "No commits since last tag.".yellow());
    } else {
        println!("{}", "Recent commits:".bold().underline());
        let display_count = ctx.recent_commits.len().min(10);
        for (hash, subject) in ctx.recent_commits.iter().take(display_count) {
            println!("  {} {}", hash.dimmed(), subject);
        }
        let remaining = ctx.recent_commits.len().saturating_sub(display_count);
        if remaining > 0 {
            println!("  {} ... and {remaining} more", "".dimmed());
        }
        println!();
    }

    // Show current version
    if let Some(ref v) = ctx.current_version {
        println!("{}: {}", "Current version".dimmed(), v);
    } else {
        println!(
            "{}: {}",
            "Current version".dimmed(),
            "none (first release)".yellow()
        );
    }

    // Build selection options
    let options: Vec<String> = ctx
        .candidates
        .iter()
        .map(|c| format!("{} ({})", c.version, c.level))
        .collect();

    if options.is_empty() {
        bail!("no version candidates available");
    }

    let selection = Select::new("Select version:", options)
        .prompt()
        .context("version selection cancelled")?;

    // Parse the version back from the selection
    let version_str = selection
        .split_once(' ')
        .map(|(v, _)| v)
        .unwrap_or(&selection);

    scrat_core::version::parse_version(version_str).context("failed to parse selected version")
}
