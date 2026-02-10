//! Build automation tasks for scrat.
//!
//! This crate provides development utilities:
//! - `completions` - Generate shell completions
//! - `man` - Generate man pages
//! - `install` - Install binary to ~/.bin
//!
//! Run `cargo xtask --help` to see available commands.

#![deny(unsafe_code)]

mod commands;

use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "xtask")]
#[command(about = "Project maintenance tasks")]
struct Xtask {
    #[command(subcommand)]
    command: Task,
}

#[derive(Subcommand, Debug)]
enum Task {
    /// Generate shell completions for the scrat CLI.
    Completions(commands::completions::CompletionsArgs),

    /// Generate manpages for the scrat CLI.
    Man(commands::man::ManArgs),

    /// Build and install the scrat CLI into ~/.bin for local testing.
    Install(commands::install::InstallArgs),
}

fn main() -> Result<(), String> {
    let task = Xtask::parse();
    match task.command {
        Task::Completions(args) => commands::completions::cmd_completions(args),
        Task::Man(args) => commands::man::cmd_man(args),
        Task::Install(args) => commands::install::cmd_install(args),
    }
}

pub fn workspace_root() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir.parent().unwrap_or(&manifest_dir).to_path_buf()
}
