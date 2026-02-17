//! Init command — generate a scrat config file with interactive prompts.

use clap::Args;

use scrat_core::init::{ConfigFormat, ConfigStyle};

/// Arguments for the `init` subcommand.
#[derive(Args, Debug, Default)]
pub struct InitArgs {
    /// Config format to generate
    #[arg(long, value_enum)]
    pub format: Option<ConfigFormat>,

    /// Config style: documented (all options with comments) or minimal
    #[arg(long, value_enum)]
    pub style: Option<ConfigStyle>,

    /// Write config without prompting (uses detected defaults)
    #[arg(long, short = 'y')]
    pub yes: bool,

    /// Output path (default: ./scrat.toml or ./scrat.yaml)
    #[arg(long, short = 'o')]
    pub output: Option<String>,
}

/// Execute the init command.
pub fn cmd_init(
    _args: InitArgs,
    _global_json: bool,
    _cwd: &camino::Utf8Path,
) -> anyhow::Result<()> {
    println!("not yet implemented");
    Ok(())
}
