//! Preflight command — validate release readiness.

use clap::Args;
use owo_colors::OwoColorize;
use tracing::{debug, instrument};

use scrat_core::config::{Config, ProjectConfig};
use scrat_core::preflight;
use scrat_core::ship::ShipOptions;

/// Arguments for the `preflight` subcommand.
#[derive(Args, Debug, Default)]
pub struct PreflightArgs {
    /// Skip `git fetch` (faster startup, may miss recent remote changes)
    #[arg(long)]
    pub no_fetch: bool,
    // Other output options inherit from the global --json flag.
}

/// Run preflight checks and display results.
#[instrument(name = "cmd_preflight", skip_all, fields(json_output))]
pub fn cmd_preflight(
    args: PreflightArgs,
    global_json: bool,
    config: &Config,
    cwd: &camino::Utf8Path,
) -> anyhow::Result<()> {
    debug!(json_output = global_json, "executing preflight command");

    let mut config = config.clone();
    let opts = ShipOptions {
        no_fetch: args.no_fetch,
        ..ShipOptions::default()
    };
    // Standalone preflight passes a ShipOptions skeleton purely to honor
    // the network-side flags (--no-fetch). Phase-skip flags stay false so
    // every credential check still runs.
    let mut report = preflight::run_preflight(cwd, &config, Some(&opts));

    // If no ecosystem detected and not in JSON mode, prompt the user
    if report.detection.is_none() && !global_json {
        match super::prompt_ecosystem_selection() {
            Ok(ecosystem) => {
                // Re-run preflight with the user's ecosystem choice
                let project = config.project.get_or_insert_with(ProjectConfig::default);
                project.project_type = Some(ecosystem);
                report = preflight::run_preflight(cwd, &config, Some(&opts));
            }
            Err(_) => {
                // User cancelled — show the original report
            }
        }
    }

    if global_json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("{}", "Preflight Checks".bold().underline());
        println!();

        for check in &report.checks {
            let icon = if check.passed {
                "✓".green().to_string()
            } else {
                "✗".red().to_string()
            };
            if let Some(ref flag) = check.skip_flag
                && !check.passed
            {
                println!(
                    "  {icon} {}: {} {}",
                    check.name.bold(),
                    check.message,
                    format!("(skip with {flag})").dimmed(),
                );
                continue;
            }
            println!("  {icon} {}: {}", check.name.bold(), check.message);
        }

        println!();
        if report.all_passed {
            println!("  {} 🚀", "Ready to release!".green().bold());
        } else {
            let failed = report.checks.iter().filter(|c| !c.passed).count();
            println!(
                "  {} — fix issues above before releasing",
                format!("{failed} check(s) failed").red().bold(),
            );
        }
    }

    if report.all_passed {
        Ok(())
    } else {
        Err(anyhow::anyhow!("preflight checks failed"))
    }
}
