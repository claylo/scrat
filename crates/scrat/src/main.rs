//! scrat CLI
#![deny(unsafe_code)]

use anyhow::Context;
use clap::Parser;
use scrat::{Cli, Commands, commands};
use scrat_core::config::ConfigLoader;
use tracing::debug;

mod observability;

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    cli.color.apply();

    if let Some(ref dir) = cli.chdir {
        std::env::set_current_dir(dir)
            .with_context(|| format!("failed to change directory to {}", dir.display()))?;
    }

    let cwd = std::env::current_dir().context("failed to determine current directory")?;
    let cwd = camino::Utf8PathBuf::try_from(cwd).map_err(|e| {
        anyhow::anyhow!(
            "current directory is not valid UTF-8: {}",
            e.into_path_buf().display()
        )
    })?;
    let mut loader = ConfigLoader::new().with_project_search(&cwd);
    if let Some(ref config_path) = cli.config {
        let config_path = camino::Utf8PathBuf::try_from(config_path.clone()).map_err(|e| {
            anyhow::anyhow!(
                "config path is not valid UTF-8: {}",
                e.into_path_buf().display()
            )
        })?;
        loader = loader.with_file(&config_path);
    }
    let config = loader.load().context("failed to load configuration")?;

    let obs_config = observability::ObservabilityConfig::from_env_with_overrides(
        config
            .log_dir
            .as_ref()
            .map(|dir| dir.as_std_path().to_path_buf()),
    );
    let env_filter = observability::env_filter(cli.quiet, cli.verbose, config.log_level.as_str());
    let _guard = observability::init_observability(&obs_config, env_filter)
        .context("failed to initialize logging/tracing")?;

    debug!(
        verbose = cli.verbose,
        quiet = cli.quiet,
        json = cli.json,
        color = ?cli.color,
        chdir = ?cli.chdir,
        "CLI initialized"
    );

    // Execute command
    let result = match cli.command {
        Commands::Doctor(args) => commands::doctor::cmd_doctor(args, cli.json, &cwd),
        Commands::Info(args) => commands::info::cmd_info(args, cli.json, &config, &cwd),
    };
    if let Err(ref err) = result {
        tracing::error!(error = %err, "fatal error");
    }
    result
}
