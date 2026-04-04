//! scrat CLI
#![deny(unsafe_code)]

use anyhow::Context;
use clap::FromArgMatches;
use scrat::{Cli, Commands, commands};
use scrat_core::config::ConfigLoader;
use tracing::debug;

use scrat_core::observability;

fn main() {
    if let Err(err) = run() {
        eprintln!("Error: {err}");
        err.chain().skip(1).for_each(|cause| {
            eprintln!("\nCaused by:\n    {cause}");
        });
        std::process::exit(1);
    }
}

fn run() -> anyhow::Result<()> {
    // Use scrat::command() which adds the custom -h/--help flag back
    // (Cli derive has disable_help_flag = true so we can use HelpShort).
    // Intercept --version to render the :shipit: squirrel before the version string.
    let cli = match scrat::command().try_get_matches() {
        Ok(matches) => {
            Cli::from_arg_matches(&matches).expect("clap mismatch between Cli derive and command()")
        }
        Err(e) if e.kind() == clap::error::ErrorKind::DisplayVersion => {
            scrat::terminal::render_shipit();
            e.print().expect("failed to write version");
            return Ok(());
        }
        Err(e) => e.exit(),
    };

    cli.color.apply();

    if cli.version_only {
        println!("{}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    // arg_required_else_help ensures we have --version-only or a subcommand
    let Some(command) = cli.command else {
        return Ok(());
    };

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
    let (config, config_sources) = loader.load().context("failed to load configuration")?;

    let obs_config = observability::ObservabilityConfig::from_env_with_overrides(
        env!("CARGO_PKG_NAME"),
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
    let result = match command {
        Commands::Doctor(args) => {
            commands::doctor::cmd_doctor(args, cli.json, &config_sources, &cwd)
        }
        Commands::Init(args) => commands::init::cmd_init(args, cli.json, &cwd),
        Commands::Info(args) => {
            commands::info::cmd_info(args, cli.json, &config, &config_sources, &cwd)
        }
        Commands::Preflight(args) => {
            commands::preflight::cmd_preflight(args, cli.json, &config, &cwd)
        }
        Commands::Bump(args) => commands::bump::cmd_bump(args, cli.json, &config, &cwd),
        Commands::Notes(args) => commands::notes::cmd_notes(args, cli.json, &config, &cwd),
        Commands::Ship(args) => commands::ship::cmd_ship(args, cli.json, &config, &cwd),
    };
    if let Err(ref err) = result {
        tracing::error!(error = %err, "fatal error");
    }
    result
}
