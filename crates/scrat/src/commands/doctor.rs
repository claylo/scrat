//! Doctor command — diagnose configuration and environment.

use clap::Args;
use owo_colors::OwoColorize;
use serde::Serialize;
use tracing::{debug, instrument};
use scrat_core::config;
use indicatif::{ProgressBar, ProgressStyle};
use inquire::Confirm;
/// Arguments for the `doctor` subcommand.
#[derive(Args, Debug, Default)]
pub struct DoctorArgs {
    // No subcommand-specific arguments; uses global --json flag
}

#[derive(Serialize)]
struct DoctorReport {
    directories: DirectoryPaths,
    config: ConfigStatus,
    environment: EnvironmentInfo,
}

#[derive(Serialize)]
struct DirectoryPaths {
    config: Option<String>,
    cache: Option<String>,
    data: Option<String>,
    data_local: Option<String>,
}

#[derive(Serialize)]
struct ConfigStatus {
    /// Path to loaded config file, if any
    file: Option<String>,
    /// Whether a config file was found
    found: bool,
}

#[derive(Serialize)]
struct EnvironmentInfo {
    /// Current working directory
    cwd: Option<String>,
    /// Relevant environment variables
    env_vars: Vec<EnvVar>,
}

#[derive(Serialize)]
struct EnvVar {
    name: &'static str,
    value: Option<String>,
    description: &'static str,
}

impl DoctorReport {
    fn gather(cwd: &camino::Utf8Path) -> Self {
        let config_file = config::find_project_config(cwd);

        Self {
            directories: DirectoryPaths {
                config: config::user_config_dir().map(|p| p.to_string()),
                cache: config::user_cache_dir().map(|p| p.to_string()),
                data: config::user_data_dir().map(|p| p.to_string()),
                data_local: config::user_data_local_dir().map(|p| p.to_string()),
            },
            config: ConfigStatus {
                found: config_file.is_some(),
                file: config_file.map(|p| p.to_string()),
            },
            environment: EnvironmentInfo {
                cwd: Some(cwd.to_string()),
                env_vars: vec![
                    EnvVar {
                        name: "XDG_CONFIG_HOME",
                        value: std::env::var("XDG_CONFIG_HOME").ok(),
                        description: "Override config directory",
                    },
                    EnvVar {
                        name: "XDG_CACHE_HOME",
                        value: std::env::var("XDG_CACHE_HOME").ok(),
                        description: "Override cache directory",
                    },
                    EnvVar {
                        name: "XDG_DATA_HOME",
                        value: std::env::var("XDG_DATA_HOME").ok(),
                        description: "Override data directory",
                    },
                    EnvVar {
                        name: "RUST_LOG",
                        value: std::env::var("RUST_LOG").ok(),
                        description: "Log filter directive",
                    },

                ],
            },
        }
    }
}

/// Run diagnostics and report configuration status.
///
/// # Arguments
/// * `global_json` - Global `--json` flag from CLI
/// * `cwd` - Current working directory
#[instrument(name = "cmd_doctor", skip_all, fields(json_output))]
pub fn cmd_doctor(_args: DoctorArgs, global_json: bool, cwd: &camino::Utf8Path) -> anyhow::Result<()> {
    debug!(
        json_output = global_json,
        "executing doctor command"
    );

let spinner = ProgressBar::new_spinner();
    spinner.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.cyan} {msg}")
            .expect("valid template")
    );
    spinner.set_message("Gathering diagnostics...");
    spinner.enable_steady_tick(std::time::Duration::from_millis(80));

    let report = DoctorReport::gather(cwd);
    spinner.finish_and_clear();
if global_json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        // Config status
        println!("{}", "Configuration".bold().underline());
        if report.config.found {
            println!("  {} Config file: {}", "✓".green(), report.config.file.as_deref().unwrap_or("").cyan());
        } else {
            println!("  {} No config file found", "○".yellow());
offer_config_creation()?;
}
        println!();

        // Directories
        println!("{}", "Directories".bold().underline());
        print_dir("  Config", &report.directories.config);
        print_dir("  Cache", &report.directories.cache);
        print_dir("  Data", &report.directories.data);
        print_dir("  Data (local)", &report.directories.data_local);
        println!();

        // Environment
        println!("{}", "Environment".bold().underline());
        println!("  {}: {}", "Working directory".dimmed(), cwd.cyan());

        let set_vars: Vec<_> = report.environment.env_vars.iter()
            .filter(|v| v.value.is_some())
            .collect();

        if set_vars.is_empty() {
            println!("  {} No XDG/logging overrides set", "○".dimmed());
        } else {
            for var in set_vars {
                println!("  {}: {}", var.name.dimmed(), var.value.as_deref().unwrap_or("").cyan());
            }
        }
    }

    Ok(())
}

fn print_dir(label: &str, path: &Option<String>) {
    print!("{}: ", label.dimmed());
    match path {
        Some(p) => println!("{}", p.cyan()),
        None => println!("{}", "(unavailable)".yellow()),
    }
}

/// Offer to create a default config file when none exists.
fn offer_config_creation() -> anyhow::Result<()> {
    let Some(config_dir) = config::user_config_dir() else {
        return Ok(());
    };

    let config_path = config_dir.join("config.yaml");

    // Don't prompt if running non-interactively
    if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        return Ok(());
    }

    let create = Confirm::new("Create a default config file?")
        .with_default(false)
        .with_help_message(&format!("Will create {config_path}"))
        .prompt();

    match create {
        Ok(true) => {
            // Ensure directory exists
            if let Some(parent) = config_path.parent() {
                std::fs::create_dir_all(parent)?;
            }

            // Write default config as YAML
let default_config = scrat_core::config::Config::default();
let yaml = serde_saphyr::to_string(&default_config)?;
            std::fs::write(&config_path, yaml)?;

            println!("  {} Created {}", "✓".green(), config_path.cyan());
        }
        Ok(false) => {
            // User declined
        }
        Err(_) => {
            // Prompt interrupted (Ctrl+C, etc.)
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_cwd() -> camino::Utf8PathBuf {
        camino::Utf8PathBuf::from("/tmp")
    }

    #[test]
    fn test_cmd_doctor_text_succeeds() {
        assert!(cmd_doctor(DoctorArgs::default(), false, &test_cwd()).is_ok());
    }

    #[test]
    fn test_cmd_doctor_json_succeeds() {
        assert!(cmd_doctor(DoctorArgs::default(), true, &test_cwd()).is_ok());
    }

    #[test]
    fn test_doctor_report_gathers() {
        let report = DoctorReport::gather(&test_cwd());
        // On most systems, at least config dir should resolve
        assert!(report.directories.config.is_some() || report.directories.cache.is_some());
    }
}

