//! Info command — show package, config, and detected project information.

use clap::Args;
use owo_colors::OwoColorize;
use serde::Serialize;
use tracing::{debug, instrument};

use scrat_core::config::{self, Config};
use scrat_core::detect;
use scrat_core::ecosystem::ProjectDetection;

/// Arguments for the `info` subcommand.
#[derive(Args, Debug, Default)]
pub struct InfoArgs {
    // No subcommand-specific arguments; uses global --json flag
}

#[derive(Serialize)]
struct PackageInfo {
    name: &'static str,
    version: &'static str,
    #[serde(skip_serializing_if = "str::is_empty")]
    description: &'static str,
    #[serde(skip_serializing_if = "str::is_empty")]
    repository: &'static str,
    #[serde(skip_serializing_if = "str::is_empty")]
    homepage: &'static str,
    #[serde(skip_serializing_if = "str::is_empty")]
    license: &'static str,
}

impl PackageInfo {
    const fn new() -> Self {
        Self {
            name: env!("CARGO_PKG_NAME"),
            version: env!("CARGO_PKG_VERSION"),
            description: env!("CARGO_PKG_DESCRIPTION"),
            repository: env!("CARGO_PKG_REPOSITORY"),
            homepage: env!("CARGO_PKG_HOMEPAGE"),
            license: env!("CARGO_PKG_LICENSE"),
        }
    }
}

#[derive(Serialize)]
struct ConfigInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    config_file: Option<String>,
    log_level: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    log_dir: Option<String>,
}

impl ConfigInfo {
    fn from_config(config: &Config, cwd: &camino::Utf8Path) -> Self {
        Self {
            config_file: config::find_project_config(cwd).map(|p| p.to_string()),
            log_level: config.log_level.as_str().to_string(),
            log_dir: config.log_dir.as_ref().map(|p| p.to_string()),
        }
    }
}

#[derive(Serialize)]
struct FullInfo {
    #[serde(flatten)]
    package: PackageInfo,
    config: ConfigInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    detection: Option<ProjectDetection>,
}

/// Print package information.
///
/// # Arguments
/// * `global_json` - Global `--json` flag from CLI
/// * `config` - Loaded configuration
/// * `cwd` - Current working directory for config discovery and detection
#[instrument(name = "cmd_info", skip_all, fields(json_output))]
pub fn cmd_info(
    _args: InfoArgs,
    global_json: bool,
    config: &Config,
    cwd: &camino::Utf8Path,
) -> anyhow::Result<()> {
    let info = PackageInfo::new();

    debug!(json_output = global_json, "executing info command");

    let config_info = ConfigInfo::from_config(config, cwd);
    let detection = detect::resolve_detection(cwd, config);

    let full_info = FullInfo {
        package: info,
        config: config_info,
        detection: detection.clone(),
    };

    if global_json {
        println!("{}", serde_json::to_string_pretty(&full_info)?);
    } else {
        if !crate::terminal::render_shipit() {
            println!("  {}", ":shipit:".bold());
        }
        println!(
            "{} {}",
            full_info.package.name.bold(),
            full_info.package.version.green()
        );
        if !full_info.package.description.is_empty() {
            println!("{}", full_info.package.description);
        }
        if !full_info.package.license.is_empty() {
            println!("{}: {}", "License".dimmed(), full_info.package.license);
        }
        if !full_info.package.repository.is_empty() {
            println!(
                "{}: {}",
                "Repository".dimmed(),
                full_info.package.repository.cyan()
            );
        }
        if !full_info.package.homepage.is_empty() {
            println!(
                "{}: {}",
                "Homepage".dimmed(),
                full_info.package.homepage.cyan()
            );
        }

        // Configuration section
        println!();
        println!("{}", "Configuration".bold().underline());
        if let Some(ref path) = full_info.config.config_file {
            println!("{}: {}", "Config file".dimmed(), path.cyan());
        } else {
            println!("{}: {}", "Config file".dimmed(), "none loaded".yellow());
        }
        println!("{}: {}", "Log level".dimmed(), full_info.config.log_level);
        if let Some(ref dir) = full_info.config.log_dir {
            println!("{}: {}", "Log directory".dimmed(), dir);
        }

        // Detection section
        println!();
        println!("{}", "Project Detection".bold().underline());
        if let Some(ref det) = detection {
            println!(
                "{}: {}",
                "Ecosystem".dimmed(),
                det.ecosystem.to_string().cyan()
            );
            println!(
                "{}: {}",
                "Version strategy".dimmed(),
                det.version_strategy.to_string().cyan()
            );
            println!("{}: {}", "Test command".dimmed(), det.tools.test_cmd.cyan());
            println!(
                "{}: {}",
                "Build command".dimmed(),
                det.tools.build_cmd.cyan()
            );
            if let Some(ref cmd) = det.tools.publish_cmd {
                println!("{}: {}", "Publish command".dimmed(), cmd.cyan());
            }
            if let Some(ref cmd) = det.tools.bump_cmd {
                println!("{}: {}", "Bump command".dimmed(), cmd.cyan());
            }
            if let Some(ref tool) = det.tools.changelog_tool {
                println!("{}: {}", "Changelog tool".dimmed(), tool.to_string().cyan());
            }
        } else {
            println!(
                "  {} {}",
                "○".yellow(),
                "No recognized project detected".yellow()
            );
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> Config {
        Config::default()
    }

    fn test_cwd() -> camino::Utf8PathBuf {
        camino::Utf8PathBuf::from("/tmp")
    }

    #[test]
    fn test_cmd_info_text_succeeds() {
        assert!(cmd_info(InfoArgs::default(), false, &test_config(), &test_cwd()).is_ok());
    }

    #[test]
    fn test_cmd_info_json_via_global() {
        assert!(cmd_info(InfoArgs::default(), true, &test_config(), &test_cwd()).is_ok());
    }

    #[test]
    fn test_config_info_no_file() {
        let config = Config::default();
        let cwd = camino::Utf8PathBuf::from("/nonexistent");
        let info = ConfigInfo::from_config(&config, &cwd);
        assert!(info.config_file.is_none());
        assert_eq!(info.log_level, "info");
    }
}
