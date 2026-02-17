//! Ship command — thin CLI layer over `scrat_core::ship`.

use anyhow::{Context, bail};
use clap::Args;
use indicatif::{ProgressBar, ProgressStyle};
use inquire::{Confirm, Select};
use owo_colors::OwoColorize;
use tracing::{debug, instrument};

use scrat_core::config::Config;
use scrat_core::ship::{self, PhaseOutcome, ShipEvent, ShipOptions, ShipPlan};

/// Arguments for the `ship` subcommand.
#[derive(Args, Debug, Default)]
pub struct ShipArgs {
    /// Set version explicitly (e.g., "1.2.3" or "v1.2.3")
    #[arg(long, value_name = "VERSION")]
    pub version: Option<String>,

    /// Skip changelog generation
    #[arg(long)]
    pub no_changelog: bool,

    /// Skip publishing to registry
    #[arg(long)]
    pub no_publish: bool,

    /// Skip git push (still commits and tags locally)
    #[arg(long)]
    pub no_push: bool,

    /// Skip GitHub release creation
    #[arg(long)]
    pub no_release: bool,

    /// Skip dependency diff
    #[arg(long)]
    pub no_deps: bool,

    /// Skip release statistics collection
    #[arg(long)]
    pub no_stats: bool,

    /// Skip release notes rendering (uses GitHub auto-generated notes)
    #[arg(long)]
    pub no_notes: bool,

    /// Skip running tests
    #[arg(long)]
    pub no_test: bool,

    /// Skip git tag creation (still commits and pushes)
    #[arg(long)]
    pub no_tag: bool,

    /// Skip entire git phase (commit, tag, push)
    #[arg(long)]
    pub no_git: bool,

    /// Create release as draft (overrides config)
    #[arg(long, conflicts_with = "no_draft")]
    pub draft: bool,

    /// Create release as published, not draft (overrides config)
    #[arg(long, conflicts_with = "draft")]
    pub no_draft: bool,

    /// Preview what would happen without making changes
    #[arg(long)]
    pub dry_run: bool,

    /// Skip confirmation prompt
    #[arg(long, short = 'y')]
    pub yes: bool,
}

/// Execute the ship command.
#[instrument(name = "cmd_ship", skip_all)]
pub fn cmd_ship(
    args: ShipArgs,
    global_json: bool,
    config: &Config,
    cwd: &camino::Utf8Path,
) -> anyhow::Result<()> {
    debug!(
        json_output = global_json,
        dry_run = args.dry_run,
        "executing ship command"
    );

    let skip_confirm = args.yes;

    let draft_override = if args.draft {
        Some(true)
    } else if args.no_draft {
        Some(false)
    } else {
        None
    };

    let options = ShipOptions {
        explicit_version: args.version,
        no_changelog: args.no_changelog,
        no_publish: args.no_publish,
        no_push: args.no_push,
        no_release: args.no_release,
        no_deps: args.no_deps,
        no_stats: args.no_stats,
        no_notes: args.no_notes,
        dry_run: args.dry_run,
        no_test: args.no_test,
        no_tag: args.no_tag,
        no_git: args.no_git,
        draft_override,
    };

    let is_dry = options.dry_run;

    // Plan the ship (preflight + version resolution)
    let plan = ship::plan_ship(cwd, config, options).context("ship planning failed")?;

    // Resolve interactive prompt if needed
    let ready = match plan {
        ShipPlan::Ready(r) => r,
        ShipPlan::NeedsInteraction(interactive) => {
            let chosen = prompt_interactive_version(&interactive)
                .context("interactive version selection failed")?;
            ship::resolve_ship_interaction(interactive, chosen)
        }
    };

    // Display the plan header
    if !global_json {
        if is_dry {
            println!("\n{}", "DRY RUN — no changes will be made".yellow().bold());
        }
        println!(
            "\n{}: {} → {}",
            "Ship".bold(),
            ready.bump.previous.to_string().dimmed(),
            ready.bump.next.to_string().green().bold(),
        );
        println!(
            "{}: {} | {}: {}",
            "Strategy".dimmed(),
            ready.bump.strategy,
            "Ecosystem".dimmed(),
            ready.detection.ecosystem,
        );
        println!();
    }

    // Confirm before executing (unless dry-run, --yes, or config says no)
    if !is_dry && !global_json {
        let config_confirm = config.ship.as_ref().and_then(|s| s.confirm).unwrap_or(true);

        if config_confirm && !skip_confirm {
            print_phase_summary(&ready.options, config);
            let confirmed = Confirm::new("Proceed with release?")
                .with_default(true)
                .prompt()
                .context("confirmation prompt failed")?;
            if !confirmed {
                println!("{}", "Ship cancelled.".yellow());
                return Ok(());
            }
            println!();
        }
    }

    // Execute with progress display
    let outcome = ready
        .execute(cwd, |event| {
            if !global_json {
                handle_event(event, is_dry);
            }
        })
        .context("ship failed")?;

    // Display final summary
    if global_json {
        println!("{}", serde_json::to_string_pretty(&outcome)?);
    } else {
        println!();
        if is_dry {
            println!(
                "{} Dry run complete — {} phases previewed, {} hooks would run",
                "✓".green(),
                outcome.phases.len(),
                outcome.hooks_run,
            );
        } else {
            print_shipit_squirrel();
            println!(
                "{} Shipped {} ({} phases, {} hooks)",
                "✓".green().bold(),
                outcome.tag.green().bold(),
                outcome.phases.len(),
                outcome.hooks_run,
            );
        }
    }

    Ok(())
}

/// Handle a ship event for terminal progress display.
fn handle_event(event: ShipEvent, is_dry: bool) {
    match event {
        ShipEvent::PhaseStarted(phase) => {
            let spinner = ProgressBar::new_spinner();
            spinner.set_style(
                ProgressStyle::with_template("  {spinner:.cyan} {msg}")
                    .unwrap()
                    .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
            );
            spinner.set_message(format!("{phase}..."));
            // For now we finish immediately since phases are synchronous.
            // The spinner shows briefly to indicate activity.
            spinner.finish_and_clear();
        }
        ShipEvent::PhaseCompleted(phase, outcome) => match outcome {
            PhaseOutcome::Success { message } => {
                let prefix = if is_dry { "○" } else { "✓" };
                println!(
                    "  {} {} {}",
                    prefix.green(),
                    format!("{phase}").bold(),
                    message.dimmed(),
                );
            }
            PhaseOutcome::Skipped { reason } => {
                println!(
                    "  {} {} {}",
                    "–".yellow(),
                    format!("{phase}").bold(),
                    format!("skipped: {reason}").dimmed(),
                );
            }
        },
        ShipEvent::HooksStarted {
            phase,
            count,
            commands,
            will_execute,
        } => {
            if will_execute {
                debug!(%phase, count, "running hooks");
            } else {
                // Dry-run: show what hooks would run
                for cmd in &commands {
                    println!("    {} {}", "hook →".dimmed(), cmd.cyan(),);
                }
            }
        }
        ShipEvent::HooksCompleted { phase, count } => {
            debug!(%phase, count, "hooks completed");
        }
    }
}

/// Display interactive context and prompt the user to pick a version.
fn prompt_interactive_version(
    plan: &ship::InteractiveShip,
) -> anyhow::Result<scrat_core::semver::Version> {
    let ctx = &plan.bump.context;

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

/// Print a summary of phases and hooks before the confirmation prompt.
fn print_phase_summary(options: &ShipOptions, config: &Config) {
    let phases: &[(&str, bool)] = &[
        ("test", !options.no_test),
        ("bump", true),
        ("publish", !options.no_publish),
        ("git", !options.no_git),
        ("release", !options.no_release),
    ];

    let active: Vec<&str> = phases
        .iter()
        .filter(|(_, on)| *on)
        .map(|(n, _)| *n)
        .collect();
    let skipped: Vec<&str> = phases
        .iter()
        .filter(|(_, on)| !*on)
        .map(|(n, _)| *n)
        .collect();

    print!("  {}: {}", "Phases".dimmed(), active.join(", ").bold());
    if !skipped.is_empty() {
        print!(" {}", format!("(skip: {})", skipped.join(", ")).dimmed());
    }
    println!();

    let hook_count = count_hooks(config);
    if hook_count > 0 {
        println!(
            "  {}: {} hook command{}",
            "Hooks".dimmed(),
            hook_count,
            if hook_count == 1 { "" } else { "s" }
        );
    }

    println!();
}

/// Count total hook commands configured.
fn count_hooks(config: &Config) -> usize {
    let Some(hooks) = config.hooks.as_ref() else {
        return 0;
    };
    [
        hooks.pre_ship.as_ref(),
        hooks.post_ship.as_ref(),
        hooks.pre_test.as_ref(),
        hooks.post_test.as_ref(),
        hooks.pre_bump.as_ref(),
        hooks.post_bump.as_ref(),
        hooks.pre_publish.as_ref(),
        hooks.post_publish.as_ref(),
        hooks.pre_tag.as_ref(),
        hooks.post_tag.as_ref(),
        hooks.pre_release.as_ref(),
        hooks.post_release.as_ref(),
    ]
    .iter()
    .filter_map(|h| h.as_ref())
    .map(|cmds| cmds.len())
    .sum()
}

/// Print the :shipit: squirrel — a scrat tradition.
fn print_shipit_squirrel() {
    println!();

    if !crate::terminal::render_shipit() {
        println!("  {}", ":shipit:".bold());
    }

    println!("  {}", "SHIP IT!".bold());
    println!();
}
