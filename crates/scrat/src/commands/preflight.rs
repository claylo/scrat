//! Preflight command — validate release readiness.

use clap::Args;
use owo_colors::OwoColorize;
use tracing::{debug, instrument};

use scrat_core::config::Config;
use scrat_core::preflight;

/// Arguments for the `preflight` subcommand.
#[derive(Args, Debug, Default)]
pub struct PreflightArgs {
    // Uses global --json flag for structured output
}

/// Run preflight checks and display results.
#[instrument(name = "cmd_preflight", skip_all, fields(json_output))]
pub fn cmd_preflight(
    _args: PreflightArgs,
    global_json: bool,
    config: &Config,
    cwd: &camino::Utf8Path,
) -> anyhow::Result<()> {
    debug!(json_output = global_json, "executing preflight command");

    let report = preflight::run_preflight(cwd, config);

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
