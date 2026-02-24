//! Init command — generate a scrat config file with interactive prompts.

use anyhow::{Context, bail};
use clap::Args;
use inquire::{Confirm, Select};
use owo_colors::OwoColorize;
use tracing::{debug, instrument};

use scrat_core::config;
use scrat_core::ecosystem::Ecosystem;
use scrat_core::init::{self, ConfigFormat, ConfigStyle, InitSelections};

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

/// What to do when an existing config is found.
enum ExistingAction {
    Merge,
    Overwrite,
    Exit,
}

/// Execute the init command.
#[instrument(name = "cmd_init", skip_all)]
pub fn cmd_init(args: InitArgs, global_json: bool, cwd: &camino::Utf8Path) -> anyhow::Result<()> {
    debug!(
        json_output = global_json,
        yes = args.yes,
        format = ?args.format,
        style = ?args.style,
        "executing init command"
    );

    // 1. Discover the project
    let plan = init::plan_init(cwd);

    // 2. Handle existing config
    if let Some(ref existing_path) = plan.existing_config {
        if args.yes {
            debug!(%existing_path, "overwriting existing config (--yes)");
        } else {
            match prompt_existing_action(existing_path)? {
                ExistingAction::Merge => {
                    debug!(%existing_path, "merge: loading existing config");
                    // Load existing config for reference. The existing values aren't
                    // wired into prompt defaults yet (future enhancement). For now
                    // we just proceed with the same prompts.
                    let _existing = config::ConfigLoader::new()
                        .with_project_search(cwd)
                        .load()
                        .ok();
                }
                ExistingAction::Overwrite => {
                    debug!(%existing_path, "overwriting existing config");
                }
                ExistingAction::Exit => {
                    println!("{}", "Init cancelled.".yellow());
                    return Ok(());
                }
            }
        }
    }

    // 3. Build selections from prompts (or --yes defaults)
    let selections = if args.yes {
        InitSelections {
            format: args.format.unwrap_or_default(),
            style: args.style.unwrap_or_default(),
            ecosystem: plan.ecosystem,
            release_branch: None,
            github_release: true,
            draft: true,
        }
    } else {
        prompt_selections(&args, &plan)?
    };

    // 4. Generate config
    let config_content = init::generate_config(&selections);

    // 5. Determine output path
    let ext = match selections.format {
        ConfigFormat::Toml => "toml",
        ConfigFormat::Yaml => "yaml",
    };
    let output_path = args.output.as_ref().map_or_else(
        || cwd.join(format!("scrat.{ext}")),
        camino::Utf8PathBuf::from,
    );

    // 6. Confirm write (unless --yes)
    if !args.yes {
        println!();
        let confirmed = Confirm::new(&format!("Write config to {output_path}?"))
            .with_default(true)
            .prompt()
            .context("confirmation prompt failed")?;
        if !confirmed {
            println!("{}", "Init cancelled.".yellow());
            return Ok(());
        }
    }

    // 7. Write the file
    std::fs::write(output_path.as_std_path(), &config_content)
        .with_context(|| format!("failed to write {output_path}"))?;

    // 8. Success output
    if global_json {
        let result = serde_json::json!({
            "path": output_path.as_str(),
            "format": ext,
            "style": format!("{:?}", selections.style).to_lowercase(),
        });
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        println!("{} Wrote {}", "\u{2713}".green(), output_path.bold());
        println!(
            "{} Run {} to verify",
            "\u{2192}".dimmed(),
            "scrat preflight".cyan()
        );
    }

    Ok(())
}

/// Prompt the user when an existing config file is found.
fn prompt_existing_action(existing_path: &str) -> anyhow::Result<ExistingAction> {
    println!(
        "\n{} {}",
        "Found existing config at".yellow().bold(),
        existing_path.bold(),
    );

    let options = vec![
        "Merge/update (use existing values as defaults)".to_string(),
        "Overwrite (start fresh)".to_string(),
        "Exit".to_string(),
    ];

    let selection = Select::new("What do you want to do?", options)
        .with_starting_cursor(0)
        .prompt()
        .context("selection cancelled")?;

    match selection.as_str() {
        s if s.starts_with("Merge") => Ok(ExistingAction::Merge),
        s if s.starts_with("Overwrite") => Ok(ExistingAction::Overwrite),
        "Exit" => Ok(ExistingAction::Exit),
        _ => bail!("unexpected selection: {selection}"),
    }
}

/// Run interactive prompts to build `InitSelections`.
fn prompt_selections(args: &InitArgs, plan: &init::InitPlan) -> anyhow::Result<InitSelections> {
    // Format
    let format = if let Some(f) = args.format {
        f
    } else {
        prompt_format()?
    };

    // Style
    let style = if let Some(s) = args.style {
        s
    } else {
        prompt_style()?
    };

    // Ecosystem
    let ecosystem = prompt_ecosystem(plan)?;

    // GitHub releases
    let (github_release, draft) = prompt_github_release()?;

    Ok(InitSelections {
        format,
        style,
        ecosystem,
        release_branch: None,
        github_release,
        draft,
    })
}

/// Prompt for config format (TOML or YAML).
fn prompt_format() -> anyhow::Result<ConfigFormat> {
    let options = vec!["TOML (recommended)".to_string(), "YAML".to_string()];

    let selection = Select::new("Config format:", options)
        .with_starting_cursor(0)
        .prompt()
        .context("format selection cancelled")?;

    if selection.starts_with("TOML") {
        Ok(ConfigFormat::Toml)
    } else {
        Ok(ConfigFormat::Yaml)
    }
}

/// Prompt for config style (documented or minimal).
fn prompt_style() -> anyhow::Result<ConfigStyle> {
    let options = vec![
        "Documented \u{2014} all options with comments (recommended)".to_string(),
        "Minimal \u{2014} only active values".to_string(),
    ];

    let selection = Select::new("Config style:", options)
        .with_starting_cursor(0)
        .prompt()
        .context("style selection cancelled")?;

    if selection.starts_with("Documented") {
        Ok(ConfigStyle::Documented)
    } else {
        Ok(ConfigStyle::Minimal)
    }
}

/// Prompt for ecosystem, using detection results when available.
fn prompt_ecosystem(plan: &init::InitPlan) -> anyhow::Result<Option<Ecosystem>> {
    if let Some(detected) = plan.ecosystem {
        let confirmed = Confirm::new(&format!("Detected ecosystem: {detected}. Correct?"))
            .with_default(true)
            .prompt()
            .context("ecosystem confirmation cancelled")?;

        if confirmed {
            return Ok(Some(detected));
        }

        // User said no — let them pick
        let eco = super::prompt_ecosystem_selection()?;
        return Ok(Some(eco));
    }

    // No detection — offer manual selection
    let options = vec![
        "Generic (no ecosystem-specific behavior)".to_string(),
        "Rust".to_string(),
        "Node".to_string(),
        "Skip (omit from config)".to_string(),
    ];

    let selection = Select::new("Project ecosystem:", options)
        .with_starting_cursor(0)
        .prompt()
        .context("ecosystem selection cancelled")?;

    match selection.as_str() {
        s if s.starts_with("Generic") => Ok(Some(Ecosystem::Generic)),
        "Rust" => Ok(Some(Ecosystem::Rust)),
        "Node" => Ok(Some(Ecosystem::Node)),
        s if s.starts_with("Skip") => Ok(None),
        _ => bail!("unexpected selection: {selection}"),
    }
}

/// Prompt for GitHub release preferences.
fn prompt_github_release() -> anyhow::Result<(bool, bool)> {
    let options = vec![
        "Yes, as drafts (recommended)".to_string(),
        "Yes, published immediately".to_string(),
        "No".to_string(),
    ];

    let selection = Select::new("Create GitHub releases?", options)
        .with_starting_cursor(0)
        .prompt()
        .context("GitHub release selection cancelled")?;

    match selection.as_str() {
        s if s.starts_with("Yes, as drafts") => Ok((true, true)),
        s if s.starts_with("Yes, published") => Ok((true, false)),
        "No" => Ok((false, false)),
        _ => bail!("unexpected selection: {selection}"),
    }
}
