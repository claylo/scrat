//! Command implementations

pub mod bump;

pub mod doctor;

pub mod info;

pub mod notes;

pub mod preflight;

pub mod ship;

use anyhow::Context;
use inquire::Select;
use owo_colors::OwoColorize;
use scrat_core::ecosystem::Ecosystem;

/// Prompt the user to select an ecosystem when auto-detection fails.
///
/// Shared across commands that need ecosystem detection (ship, bump, preflight).
pub fn prompt_ecosystem_selection() -> anyhow::Result<Ecosystem> {
    println!(
        "\n{}",
        "Could not auto-detect project type.".yellow().bold()
    );
    println!(
        "{}",
        "No Cargo.toml, package.json, or other marker file found.".dimmed()
    );
    println!();

    let options = vec![
        "Generic (no ecosystem-specific behavior)".to_string(),
        "Rust".to_string(),
        "Node".to_string(),
        "Exit".to_string(),
    ];

    let selection = Select::new("Select project ecosystem:", options)
        .with_starting_cursor(0)
        .prompt()
        .context("ecosystem selection cancelled")?;

    match selection.as_str() {
        s if s.starts_with("Generic") => Ok(Ecosystem::Generic),
        "Rust" => Ok(Ecosystem::Rust),
        "Node" => Ok(Ecosystem::Node),
        "Exit" => {
            println!("{}", "Cancelled.".yellow());
            std::process::exit(0);
        }
        _ => anyhow::bail!("unexpected selection: {selection}"),
    }
}
