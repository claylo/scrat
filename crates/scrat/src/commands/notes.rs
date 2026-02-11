//! Notes command — thin CLI layer over `scrat_core::notes::preview_notes`.

use anyhow::{Context, bail};
use clap::Args;
use tracing::{debug, instrument};

use scrat_core::config::Config;
use scrat_core::notes::{self, PreviewNotesOptions};

/// Arguments for the `notes` subcommand.
#[derive(Args, Debug, Default)]
pub struct NotesArgs {
    /// Previous version tag to diff against (default: latest semver tag)
    #[arg(long, value_name = "TAG")]
    pub from: Option<String>,

    /// Version to render notes for (default: current version from project files)
    #[arg(long, value_name = "VERSION")]
    pub version: Option<String>,

    /// Path to a custom git-cliff template (overrides config and built-in)
    #[arg(long, value_name = "FILE")]
    pub template: Option<String>,

    /// Skip dependency diff in rendered notes
    #[arg(long)]
    pub no_deps: bool,

    /// Skip stats collection in rendered notes
    #[arg(long)]
    pub no_stats: bool,
}

/// Execute the notes command.
#[instrument(name = "cmd_notes", skip_all)]
pub fn cmd_notes(
    args: NotesArgs,
    global_json: bool,
    config: &Config,
    cwd: &camino::Utf8Path,
) -> anyhow::Result<()> {
    debug!("rendering release notes preview");

    let options = PreviewNotesOptions {
        from: args.from,
        version: args.version,
        template: args.template,
        no_deps: args.no_deps,
        no_stats: args.no_stats,
    };

    let result =
        notes::preview_notes(cwd, config, options).context("failed to render release notes")?;

    if result.notes.trim().is_empty() {
        bail!("git-cliff produced empty output — check that there are unreleased commits");
    }

    if global_json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        print!("{}", result.notes);
    }

    Ok(())
}
