# `scrat init` Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a `scrat init` command that detects project context, walks users through ~6 prompts, and generates a documented or minimal config file (TOML or YAML).

**Architecture:** Core (`scrat-core/src/init.rs`) runs detection and generates config strings. CLI (`scrat/src/commands/init.rs`) handles prompts and file I/O. Config templates are embedded as `include_str!` — 4 templates total (toml/yaml x documented/minimal). Follows the existing thin-CLI/fat-core pattern.

**Tech Stack:** Rust, clap (derive), inquire (Select/Confirm), scrat-core detection and config modules.

---

## Task 1: Core Types and `plan_init()`

**Files:**

- Create: `crates/scrat-core/src/init.rs`
- Modify: `crates/scrat-core/src/lib.rs:37-63` (add `pub mod init;`)

**Step 1: Write the failing test**

Add to the bottom of `crates/scrat-core/src/init.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn plan_init_detects_rust_ecosystem() {
        let tmp = TempDir::new().unwrap();
        let root = camino::Utf8Path::from_path(tmp.path()).unwrap();

        // Create a Cargo.toml marker
        std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"test\"\nversion = \"0.1.0\"\n").unwrap();

        let plan = plan_init(root);
        assert_eq!(plan.ecosystem, Some(crate::ecosystem::Ecosystem::Rust));
    }

    #[test]
    fn plan_init_detects_node_ecosystem() {
        let tmp = TempDir::new().unwrap();
        let root = camino::Utf8Path::from_path(tmp.path()).unwrap();

        std::fs::write(root.join("package.json"), r#"{"name":"test","version":"1.0.0"}"#).unwrap();

        let plan = plan_init(root);
        assert_eq!(plan.ecosystem, Some(crate::ecosystem::Ecosystem::Node));
    }

    #[test]
    fn plan_init_no_ecosystem_detected() {
        let tmp = TempDir::new().unwrap();
        let root = camino::Utf8Path::from_path(tmp.path()).unwrap();

        let plan = plan_init(root);
        assert_eq!(plan.ecosystem, None);
    }

    #[test]
    fn plan_init_detects_cliff_toml() {
        let tmp = TempDir::new().unwrap();
        let root = camino::Utf8Path::from_path(tmp.path()).unwrap();

        std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"test\"\nversion = \"0.1.0\"\n").unwrap();
        std::fs::write(root.join("cliff.toml"), "[changelog]\n").unwrap();

        let plan = plan_init(root);
        assert!(matches!(
            plan.version_strategy,
            crate::ecosystem::VersionStrategy::ConventionalCommits { .. }
        ));
    }

    #[test]
    fn plan_init_finds_existing_config() {
        let tmp = TempDir::new().unwrap();
        let root = camino::Utf8Path::from_path(tmp.path()).unwrap();

        std::fs::write(root.join("scrat.toml"), "log_level = \"info\"\n").unwrap();

        let plan = plan_init(root);
        assert!(plan.existing_config.is_some());
    }

    #[test]
    fn plan_init_no_existing_config() {
        let tmp = TempDir::new().unwrap();
        let root = camino::Utf8Path::from_path(tmp.path()).unwrap();

        let plan = plan_init(root);
        assert!(plan.existing_config.is_none());
    }
}
```

**Step 2: Run test to verify it fails**

Run: `just test -E 'test(init)' 2>&1 | tail -20`
Expected: FAIL — `plan_init` doesn't exist yet.

**Step 3: Write minimal implementation**

Create `crates/scrat-core/src/init.rs`:

```rust
//! Init command — project discovery and config file generation.
//!
//! This module provides the core logic for `scrat init`:
//! - [`plan_init`] runs detection and returns an [`InitPlan`]
//! - [`generate_config`] renders a config file from [`InitSelections`]

use camino::Utf8Path;
use serde::Serialize;
use tracing::debug;

use crate::config;
use crate::detect;
use crate::ecosystem::{ChangelogTool, Ecosystem, VersionStrategy};
use crate::git;

/// The result of project discovery — detected defaults for the init prompts.
///
/// No user interaction happens here. The CLI uses this to populate prompt
/// defaults, then builds [`InitSelections`] from the user's answers.
#[derive(Debug, Clone, Serialize)]
pub struct InitPlan {
    /// Detected ecosystem (Rust, Node), or `None` if no marker file found.
    pub ecosystem: Option<Ecosystem>,
    /// Detected release branch (`main` or `master`), or `None`.
    pub release_branch: Option<String>,
    /// Detected version strategy (from cliff.toml / cog.toml).
    pub version_strategy: VersionStrategy,
    /// Detected changelog tool, if any.
    pub changelog_tool: Option<ChangelogTool>,
    /// Path to an existing scrat config file, if found.
    pub existing_config: Option<String>,
}

/// Config file format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, clap::ValueEnum)]
pub enum ConfigFormat {
    /// TOML format (recommended).
    #[default]
    Toml,
    /// YAML format.
    Yaml,
}

/// Config file style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, clap::ValueEnum)]
pub enum ConfigStyle {
    /// All options with comments and examples (recommended).
    #[default]
    Documented,
    /// Only non-default values, no comments.
    Minimal,
}

/// The user's confirmed selections from the interactive prompts.
///
/// Built by the CLI after walking through prompts with [`InitPlan`] defaults.
#[derive(Debug, Clone, Serialize)]
pub struct InitSelections {
    /// Config file format.
    pub format: ConfigFormat,
    /// Config file style.
    pub style: ConfigStyle,
    /// Chosen ecosystem (`None` = omit, let auto-detection handle it).
    pub ecosystem: Option<Ecosystem>,
    /// Chosen release branch (`None` = omit, let auto-detection handle it).
    pub release_branch: Option<String>,
    /// Whether to create GitHub releases.
    pub github_release: bool,
    /// Whether to create releases as drafts.
    pub draft: bool,
}

/// Run project discovery and return an [`InitPlan`] with detected defaults.
///
/// This is the "plan" half of the plan/execute pattern. No files are written.
/// The CLI uses the plan to populate interactive prompts.
pub fn plan_init(project_root: &Utf8Path) -> InitPlan {
    debug!(%project_root, "running init discovery");

    // Detect ecosystem
    let ecosystem = detect::detect_project(project_root).map(|d| d.ecosystem);

    // Detect version strategy
    let version_strategy = detect::detect_version_strategy(project_root);

    // Extract changelog tool from strategy
    let changelog_tool = match &version_strategy {
        VersionStrategy::ConventionalCommits { tool } => Some(*tool),
        _ => None,
    };

    // Detect release branch (may fail if not in a git repo)
    let release_branch = git::detect_release_branch()
        .ok()
        .flatten();

    // Check for existing config file
    let existing_config = config::find_project_config(project_root)
        .map(|p| p.to_string());

    InitPlan {
        ecosystem,
        release_branch,
        version_strategy,
        changelog_tool,
        existing_config,
    }
}
```

Then add to `crates/scrat-core/src/lib.rs` (after `pub mod hooks;`):

```rust
pub mod init;
```

**Step 4: Run tests to verify they pass**

Run: `just test -E 'test(init)' 2>&1 | tail -20`
Expected: All 6 tests PASS.

**Step 5: Commit**

```bash
git add crates/scrat-core/src/init.rs crates/scrat-core/src/lib.rs
git commit -m "feat(init): add plan_init() with project discovery"
```

---

## Task 2: Config Generation — Templates

**Files:**

- Create: `crates/scrat-core/templates/init-documented.toml`
- Create: `crates/scrat-core/templates/init-documented.yaml`
- Create: `crates/scrat-core/templates/init-minimal.toml`
- Create: `crates/scrat-core/templates/init-minimal.yaml`

These are plain text templates with `{{placeholder}}` markers for string replacement. Not Tera — just simple `str::replace()` since the substitutions are few and fixed.

**Step 1: Create the documented TOML template**

Create `crates/scrat-core/templates/init-documented.toml`:

```toml
# scrat configuration
# Generated by `scrat init`
#
# Docs: https://github.com/claylo/scrat

# ─── Logging ─────────────────────────────────────────────────────────
# Values: debug, info, warn, error
# Override at runtime with --verbose (-v) or --quiet (-q)
# log_level = "info"

# ─── Project ─────────────────────────────────────────────────────────
# Auto-detected from Cargo.toml / package.json. Override if needed.
{{project_section}}

# ─── Version Strategy ────────────────────────────────────────────────
# Auto-detected from cliff.toml / cog.toml presence.
# Values: conventional-commits, interactive, explicit
# [version]
# strategy = "conventional-commits"

# ─── Command Overrides ───────────────────────────────────────────────
# Defaults are set per ecosystem. Override individual commands here.
{{commands_section}}

# ─── Release ─────────────────────────────────────────────────────────
{{release_section}}

# ─── Hooks ────────────────────────────────────────────────────────────
# Shell commands that run at each phase of `scrat ship`.
# Variables: {version}, {prev_version}, {tag}, {changelog_path},
#            {owner}, {repo}
#
# Commands run in parallel by default.
#   sync:   prefix = barrier (wait for prior, run alone, then continue)
#   filter: prefix = sync + JSON stdin/stdout for pipeline mutation
#
# Phase order: pre_ship → test → bump → publish → git → release → post_ship
#
# Example:
#
# [hooks]
# post_bump = [
#     "generate-release-card --version {version} -o release-card.png",
# ]
# post_release = [
#     "sync: gh release edit {tag} --draft=false",
#     "notify-slack 'Released {owner}/{repo} {tag}'",
# ]

# ─── Ship Behavior ───────────────────────────────────────────────────
# [ship]
# confirm = true    # Prompt before executing. Use --yes/-y to skip.
```

**Step 2: Create the documented YAML template**

Create `crates/scrat-core/templates/init-documented.yaml`:

```yaml
# scrat configuration
# Generated by `scrat init`
#
# Docs: https://github.com/claylo/scrat

# ─── Logging ─────────────────────────────────────────────────────────
# Values: debug, info, warn, error
# Override at runtime with --verbose (-v) or --quiet (-q)
# log_level: info

# ─── Project ─────────────────────────────────────────────────────────
# Auto-detected from Cargo.toml / package.json. Override if needed.
{{project_section}}

# ─── Version Strategy ────────────────────────────────────────────────
# Auto-detected from cliff.toml / cog.toml presence.
# Values: conventional-commits, interactive, explicit
# version:
#   strategy: conventional-commits

# ─── Command Overrides ───────────────────────────────────────────────
# Defaults are set per ecosystem. Override individual commands here.
{{commands_section}}

# ─── Release ─────────────────────────────────────────────────────────
{{release_section}}

# ─── Hooks ────────────────────────────────────────────────────────────
# Shell commands that run at each phase of `scrat ship`.
# Variables: {version}, {prev_version}, {tag}, {changelog_path},
#            {owner}, {repo}
#
# Commands run in parallel by default.
#   sync:   prefix = barrier (wait for prior, run alone, then continue)
#   filter: prefix = sync + JSON stdin/stdout for pipeline mutation
#
# Phase order: pre_ship → test → bump → publish → git → release → post_ship
#
# Example:
#
# hooks:
#   post_bump:
#     - "generate-release-card --version {version} -o release-card.png"
#   post_release:
#     - "sync: gh release edit {tag} --draft=false"
#     - "notify-slack 'Released {owner}/{repo} {tag}'"

# ─── Ship Behavior ───────────────────────────────────────────────────
# ship:
#   confirm: true    # Prompt before executing. Use --yes/-y to skip.
```

**Step 3: Create the minimal TOML template**

Create `crates/scrat-core/templates/init-minimal.toml`:

```toml
{{project_section}}
{{release_section}}
```

**Step 4: Create the minimal YAML template**

Create `crates/scrat-core/templates/init-minimal.yaml`:

```yaml
{{project_section}}
{{release_section}}
```

**Step 5: Commit**

```bash
git add crates/scrat-core/templates/init-*.toml crates/scrat-core/templates/init-*.yaml
git commit -m "feat(init): add config file templates (toml/yaml x documented/minimal)"
```

---

## Task 3: `generate_config()` Implementation

**Files:**

- Modify: `crates/scrat-core/src/init.rs` (add `generate_config` + section builders)

**Step 1: Write the failing tests**

Add to the `tests` module in `crates/scrat-core/src/init.rs`:

```rust
    fn default_selections() -> InitSelections {
        InitSelections {
            format: ConfigFormat::Toml,
            style: ConfigStyle::Documented,
            ecosystem: Some(Ecosystem::Rust),
            release_branch: None,
            github_release: true,
            draft: true,
        }
    }

    #[test]
    fn generate_config_documented_toml() {
        let sel = default_selections();
        let output = generate_config(&sel);
        assert!(output.contains("# scrat configuration"));
        assert!(output.contains(r#"type = "rust""#));
        assert!(output.contains("github_release = true"));
        assert!(output.contains("draft = true"));
        assert!(output.contains("# [hooks]"));
    }

    #[test]
    fn generate_config_documented_yaml() {
        let mut sel = default_selections();
        sel.format = ConfigFormat::Yaml;
        let output = generate_config(&sel);
        assert!(output.contains("# scrat configuration"));
        assert!(output.contains("type: rust"));
        assert!(output.contains("github_release: true"));
        assert!(output.contains("# hooks:"));
    }

    #[test]
    fn generate_config_minimal_toml() {
        let mut sel = default_selections();
        sel.style = ConfigStyle::Minimal;
        let output = generate_config(&sel);
        assert!(output.contains(r#"type = "rust""#));
        assert!(output.contains("github_release = true"));
        assert!(!output.contains("# ───"));
    }

    #[test]
    fn generate_config_minimal_yaml() {
        let mut sel = default_selections();
        sel.format = ConfigFormat::Yaml;
        sel.style = ConfigStyle::Minimal;
        let output = generate_config(&sel);
        assert!(output.contains("type: rust"));
        assert!(output.contains("github_release: true"));
        assert!(!output.contains("# ───"));
    }

    #[test]
    fn generate_config_no_ecosystem_omits_project() {
        let mut sel = default_selections();
        sel.ecosystem = None;
        sel.style = ConfigStyle::Minimal;
        let output = generate_config(&sel);
        assert!(!output.contains("[project]"));
        assert!(!output.contains("type ="));
    }

    #[test]
    fn generate_config_no_github_release() {
        let mut sel = default_selections();
        sel.github_release = false;
        sel.style = ConfigStyle::Minimal;
        let output = generate_config(&sel);
        assert!(output.contains("github_release = false"));
    }

    #[test]
    fn generate_config_with_release_branch() {
        let mut sel = default_selections();
        sel.release_branch = Some("release".into());
        let output = generate_config(&sel);
        assert!(output.contains(r#"release_branch = "release""#));
    }

    #[test]
    fn generate_config_generic_ecosystem() {
        let mut sel = default_selections();
        sel.ecosystem = Some(Ecosystem::Generic);
        let output = generate_config(&sel);
        assert!(output.contains(r#"type = "generic""#));
    }
```

**Step 2: Run tests to verify they fail**

Run: `just test -E 'test(init)' 2>&1 | tail -20`
Expected: FAIL — `generate_config` doesn't exist.

**Step 3: Write the implementation**

Add to `crates/scrat-core/src/init.rs`, after `plan_init`:

```rust
// ──────────────────────────────────────────────
// Config generation
// ──────────────────────────────────────────────

/// Documented TOML template, shipped with scrat.
const TEMPLATE_DOC_TOML: &str = include_str!("../templates/init-documented.toml");
/// Documented YAML template, shipped with scrat.
const TEMPLATE_DOC_YAML: &str = include_str!("../templates/init-documented.yaml");
/// Minimal TOML template, shipped with scrat.
const TEMPLATE_MIN_TOML: &str = include_str!("../templates/init-minimal.toml");
/// Minimal YAML template, shipped with scrat.
const TEMPLATE_MIN_YAML: &str = include_str!("../templates/init-minimal.yaml");

/// Generate a config file string from the user's selections.
///
/// Uses embedded templates with placeholder replacement. The documented
/// style includes all options as comments with explanations; the minimal
/// style includes only non-default values.
pub fn generate_config(selections: &InitSelections) -> String {
    let template = match (selections.style, selections.format) {
        (ConfigStyle::Documented, ConfigFormat::Toml) => TEMPLATE_DOC_TOML,
        (ConfigStyle::Documented, ConfigFormat::Yaml) => TEMPLATE_DOC_YAML,
        (ConfigStyle::Minimal, ConfigFormat::Toml) => TEMPLATE_MIN_TOML,
        (ConfigStyle::Minimal, ConfigFormat::Yaml) => TEMPLATE_MIN_YAML,
    };

    let project_section = build_project_section(selections);
    let commands_section = build_commands_section(selections);
    let release_section = build_release_section(selections);

    let output = template
        .replace("{{project_section}}", &project_section)
        .replace("{{commands_section}}", &commands_section)
        .replace("{{release_section}}", &release_section);

    // Clean up any double blank lines from empty sections
    cleanup_blank_lines(&output)
}

/// Build the [project] section based on format and selections.
fn build_project_section(sel: &InitSelections) -> String {
    let eco = match sel.ecosystem {
        Some(e) => e,
        None => return String::new(),
    };

    match sel.format {
        ConfigFormat::Toml => {
            let mut lines = vec!["[project]".to_string()];
            lines.push(format!("type = \"{}\"", eco));
            if let Some(ref branch) = sel.release_branch {
                lines.push(format!("release_branch = \"{}\"", branch));
            } else if matches!(sel.style, ConfigStyle::Documented) {
                lines.push("# release_branch = \"main\"".to_string());
            }
            lines.join("\n")
        }
        ConfigFormat::Yaml => {
            let mut lines = vec!["project:".to_string()];
            lines.push(format!("  type: {}", eco));
            if let Some(ref branch) = sel.release_branch {
                lines.push(format!("  release_branch: {}", branch));
            } else if matches!(sel.style, ConfigStyle::Documented) {
                lines.push("  # release_branch: main".to_string());
            }
            lines.join("\n")
        }
    }
}

/// Build the [commands] section (commented out in documented, omitted in minimal).
fn build_commands_section(sel: &InitSelections) -> String {
    if matches!(sel.style, ConfigStyle::Minimal) {
        return String::new();
    }

    let eco = sel.ecosystem.unwrap_or(Ecosystem::Generic);

    match sel.format {
        ConfigFormat::Toml => {
            let mut lines = vec!["# [commands]".to_string()];
            match eco {
                Ecosystem::Rust => {
                    lines.push("# test = \"cargo nextest run\"".to_string());
                    lines.push("# build = \"cargo build --release\"".to_string());
                    lines.push("# publish = \"cargo publish\"".to_string());
                }
                Ecosystem::Node => {
                    lines.push("# test = \"npm test\"".to_string());
                    lines.push("# build = \"npm run build\"".to_string());
                    lines.push("# publish = \"npm publish\"".to_string());
                }
                Ecosystem::Generic => {
                    lines.push("# test = \"make test\"".to_string());
                    lines.push("# build = \"make build\"".to_string());
                }
            }
            lines.join("\n")
        }
        ConfigFormat::Yaml => {
            let mut lines = vec!["# commands:".to_string()];
            match eco {
                Ecosystem::Rust => {
                    lines.push("#   test: \"cargo nextest run\"".to_string());
                    lines.push("#   build: \"cargo build --release\"".to_string());
                    lines.push("#   publish: \"cargo publish\"".to_string());
                }
                Ecosystem::Node => {
                    lines.push("#   test: \"npm test\"".to_string());
                    lines.push("#   build: \"npm run build\"".to_string());
                    lines.push("#   publish: \"npm publish\"".to_string());
                }
                Ecosystem::Generic => {
                    lines.push("#   test: \"make test\"".to_string());
                    lines.push("#   build: \"make build\"".to_string());
                }
            }
            lines.join("\n")
        }
    }
}

/// Build the [release] section.
fn build_release_section(sel: &InitSelections) -> String {
    match sel.format {
        ConfigFormat::Toml => {
            let mut lines = vec!["[release]".to_string()];
            lines.push(format!("github_release = {}", sel.github_release));
            if sel.github_release {
                lines.push(format!("draft = {}", sel.draft));
            }
            if matches!(sel.style, ConfigStyle::Documented) {
                lines.push("# title = \"{tag}\"".to_string());
                lines.push("# assets = [\"release-card.png\", \"checksums.txt\"]".to_string());
                lines.push("# notes_template = \"templates/release-notes.tera\"".to_string());
                lines.push("# discussion_category = \"Announcements\"".to_string());
            }
            lines.join("\n")
        }
        ConfigFormat::Yaml => {
            let mut lines = vec!["release:".to_string()];
            lines.push(format!("  github_release: {}", sel.github_release));
            if sel.github_release {
                lines.push(format!("  draft: {}", sel.draft));
            }
            if matches!(sel.style, ConfigStyle::Documented) {
                lines.push("  # title: \"{tag}\"".to_string());
                lines.push("  # assets:".to_string());
                lines.push("  #   - release-card.png".to_string());
                lines.push("  #   - checksums.txt".to_string());
                lines.push("  # notes_template: templates/release-notes.tera".to_string());
                lines.push("  # discussion_category: Announcements".to_string());
            }
            lines.join("\n")
        }
    }
}

/// Collapse runs of 3+ blank lines down to 2.
fn cleanup_blank_lines(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut blank_count = 0;
    for line in s.lines() {
        if line.trim().is_empty() {
            blank_count += 1;
            if blank_count <= 2 {
                result.push('\n');
            }
        } else {
            blank_count = 0;
            result.push_str(line);
            result.push('\n');
        }
    }
    result
}
```

**Step 4: Run tests to verify they pass**

Run: `just test -E 'test(init)' 2>&1 | tail -20`
Expected: All tests PASS.

**Step 5: Commit**

```bash
git add crates/scrat-core/src/init.rs
git commit -m "feat(init): add generate_config() with template rendering"
```

---

## Task 4: CLI Command — Registration and Args

**Files:**

- Create: `crates/scrat/src/commands/init.rs`
- Modify: `crates/scrat/src/commands/mod.rs:1-14` (add `pub mod init;`)
- Modify: `crates/scrat/src/lib.rs:91-111` (add `Init` variant to `Commands`)
- Modify: `crates/scrat/src/main.rs:69-78` (add `Commands::Init` match arm)

**Step 1: Create the init command file with args only**

Create `crates/scrat/src/commands/init.rs`:

```rust
//! Init command — generate a scrat config file interactively.

use clap::Args;
use tracing::{debug, instrument};

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
#[instrument(name = "cmd_init", skip_all)]
pub fn cmd_init(
    args: InitArgs,
    global_json: bool,
    cwd: &camino::Utf8Path,
) -> anyhow::Result<()> {
    debug!(json = global_json, yes = args.yes, "executing init command");

    // TODO: implement in next task
    anyhow::bail!("init not yet implemented")
}
```

**Step 2: Register the command**

Add to `crates/scrat/src/commands/mod.rs` after the existing module declarations:

```rust
pub mod init;
```

Add to `crates/scrat/src/lib.rs` in the `Commands` enum (after `Doctor`):

```rust
    /// Generate a scrat config file
    Init(commands::init::InitArgs),
```

Add to `crates/scrat/src/main.rs` in the match block (after `Commands::Doctor`):

```rust
        Commands::Init(args) => commands::init::cmd_init(args, cli.json, &cwd),
```

Note: `Init` does NOT take `&config` — it generates the config, it doesn't read one.

**Step 3: Verify it compiles and shows in help**

Run: `cargo build 2>&1 | tail -5`
Expected: Compiles without errors.

Run: `cargo run -- --help 2>&1 | grep -i init`
Expected: Shows `init` in the command list.

Run: `cargo run -- init --help 2>&1`
Expected: Shows init-specific flags (`--format`, `--style`, `--yes`, `--output`).

**Step 4: Commit**

```bash
git add crates/scrat/src/commands/init.rs crates/scrat/src/commands/mod.rs crates/scrat/src/lib.rs crates/scrat/src/main.rs
git commit -m "feat(init): register init command with CLI args"
```

---

## Task 5: CLI Command — Interactive Prompt Flow

**Files:**

- Modify: `crates/scrat/src/commands/init.rs` (replace TODO with full implementation)

**Step 1: Implement `cmd_init`**

Replace the entire `cmd_init` function and add helpers:

```rust
//! Init command — generate a scrat config file interactively.

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

/// Execute the init command.
#[instrument(name = "cmd_init", skip_all)]
pub fn cmd_init(
    args: InitArgs,
    global_json: bool,
    cwd: &camino::Utf8Path,
) -> anyhow::Result<()> {
    debug!(json = global_json, yes = args.yes, "executing init command");

    let plan = init::plan_init(cwd);

    // ── Handle existing config ──────────────────────────────────
    let existing_config = if let Some(ref path) = plan.existing_config {
        if args.yes {
            // --yes overwrites silently
            None
        } else {
            let action = prompt_existing_config(path)?;
            match action {
                ExistingAction::Merge => {
                    // Load existing config to use as defaults
                    Some(config::ConfigLoader::new()
                        .with_project_search(cwd)
                        .load()
                        .ok())
                }
                ExistingAction::Overwrite => None,
                ExistingAction::Exit => {
                    println!("{}", "Init cancelled.".yellow());
                    return Ok(());
                }
            }
        }
    } else {
        None
    };

    // ── Build selections from prompts or --yes defaults ─────────
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
        prompt_selections(&plan, &args, &existing_config)?
    };

    // ── Generate the config content ─────────────────────────────
    let content = init::generate_config(&selections);

    // ── Determine output path ───────────────────────────────────
    let ext = match selections.format {
        ConfigFormat::Toml => "toml",
        ConfigFormat::Yaml => "yaml",
    };
    let output_path = match args.output {
        Some(ref p) => camino::Utf8PathBuf::from(p),
        None => cwd.join(format!("scrat.{ext}")),
    };

    // ── Confirm write ───────────────────────────────────────────
    if !args.yes {
        let confirmed = Confirm::new(&format!("Write to {}?", output_path))
            .with_default(true)
            .prompt()
            .context("confirmation cancelled")?;
        if !confirmed {
            println!("{}", "Init cancelled.".yellow());
            return Ok(());
        }
    }

    // ── Write the file ──────────────────────────────────────────
    std::fs::write(&output_path, &content)
        .with_context(|| format!("failed to write {output_path}"))?;

    println!(
        "  {} Wrote {}",
        "✓".green(),
        output_path.cyan(),
    );
    println!(
        "  {} Run {} to verify your setup",
        "→".dimmed(),
        "scrat preflight".bold(),
    );

    Ok(())
}

enum ExistingAction {
    Merge,
    Overwrite,
    Exit,
}

fn prompt_existing_config(path: &str) -> anyhow::Result<ExistingAction> {
    println!(
        "\n{}",
        format!("Found existing config at {path}").yellow().bold()
    );

    let options = vec![
        "Merge/update (use existing values as defaults)".to_string(),
        "Overwrite (start fresh)".to_string(),
        "Exit".to_string(),
    ];

    let selection = Select::new("What do you want to do?", options)
        .prompt()
        .context("selection cancelled")?;

    Ok(match selection.as_str() {
        s if s.starts_with("Merge") => ExistingAction::Merge,
        s if s.starts_with("Overwrite") => ExistingAction::Overwrite,
        _ => ExistingAction::Exit,
    })
}

fn prompt_selections(
    plan: &init::InitPlan,
    args: &InitArgs,
    _existing_config: &Option<Option<scrat_core::config::Config>>,
) -> anyhow::Result<InitSelections> {
    // 1. Format
    let format = match args.format {
        Some(f) => f,
        None => {
            let options = vec!["TOML (recommended)".to_string(), "YAML".to_string()];
            let sel = Select::new("Config format?", options)
                .prompt()
                .context("format selection cancelled")?;
            if sel.starts_with("TOML") {
                ConfigFormat::Toml
            } else {
                ConfigFormat::Yaml
            }
        }
    };

    // 2. Style
    let style = match args.style {
        Some(s) => s,
        None => {
            let options = vec![
                "Documented — all options with comments (recommended)".to_string(),
                "Minimal — only active values, no comments".to_string(),
            ];
            let sel = Select::new("Config style?", options)
                .prompt()
                .context("style selection cancelled")?;
            if sel.starts_with("Documented") {
                ConfigStyle::Documented
            } else {
                ConfigStyle::Minimal
            }
        }
    };

    // 3. Ecosystem
    let ecosystem = match plan.ecosystem {
        Some(detected) => {
            let confirmed = Confirm::new(&format!(
                "Detected ecosystem: {}. Correct?",
                detected.to_string().cyan()
            ))
            .with_default(true)
            .prompt()
            .context("ecosystem confirmation cancelled")?;

            if confirmed {
                Some(detected)
            } else {
                Some(super::prompt_ecosystem_selection()
                    .context("ecosystem selection failed")?)
            }
        }
        None => {
            println!(
                "\n{}",
                "Could not auto-detect project type.".yellow().bold()
            );
            let options = vec![
                "Generic (no ecosystem-specific behavior)".to_string(),
                "Rust".to_string(),
                "Node".to_string(),
                "Skip (omit from config)".to_string(),
            ];
            let sel = Select::new("Select project ecosystem:", options)
                .prompt()
                .context("ecosystem selection cancelled")?;

            match sel.as_str() {
                s if s.starts_with("Generic") => Some(Ecosystem::Generic),
                "Rust" => Some(Ecosystem::Rust),
                "Node" => Some(Ecosystem::Node),
                _ => None,
            }
        }
    };

    // 4. GitHub releases
    let options = vec![
        "Yes, as drafts (recommended)".to_string(),
        "Yes, published immediately".to_string(),
        "No".to_string(),
    ];
    let gh_sel = Select::new("Create GitHub releases?", options)
        .prompt()
        .context("GitHub release selection cancelled")?;

    let (github_release, draft) = match gh_sel.as_str() {
        s if s.contains("drafts") => (true, true),
        s if s.contains("published") => (true, false),
        _ => (false, false),
    };

    Ok(InitSelections {
        format,
        style,
        ecosystem,
        release_branch: None, // auto-detect is usually fine
        github_release,
        draft,
    })
}
```

**Step 2: Verify it compiles**

Run: `cargo build 2>&1 | tail -5`
Expected: Compiles without errors.

**Step 3: Commit**

```bash
git add crates/scrat/src/commands/init.rs
git commit -m "feat(init): implement interactive prompt flow and file writing"
```

---

## Task 6: Full Check, Clippy, and Integration Smoke Test

**Files:**

- No new files — verification only.

**Step 1: Run the full check suite**

Run: `just check 2>&1 | tail -30`
Expected: fmt clean, clippy clean, deny clean, all tests pass, doc-tests pass.

**Step 2: Fix any clippy warnings or test failures**

Address each issue individually. Common things to watch for:

- Unused imports in `init.rs`
- Clippy `collapsible_if` or `needless_borrow` warnings
- Missing `use` statements

**Step 3: Install and smoke test**

Run: `cargo xtask install 2>&1`
Expected: Installs to `~/.bin/scrat`.

Run: `cd /tmp && mkdir test-init && cd test-init && git init && scrat init --yes`
Expected: Creates `scrat.toml` with documented style, Generic ecosystem (no marker files), github_release=true, draft=true.

Run: `cat /tmp/test-init/scrat.toml`
Expected: Well-formed documented TOML config.

Run: `cd /tmp && rm -rf test-init`

**Step 4: Commit any fixes**

```bash
git add -A
git commit -m "fix(init): address clippy and integration issues"
```

---

## Task 7: Create PR

**Step 1: Create branch and push**

All work should be on a feature branch:

```bash
git checkout -b feat/init-command
# (if not already on a branch — tasks 1-6 should have been committed to this branch)
git push -u origin feat/init-command
```

**Step 2: Create PR**

```bash
gh pr create --title "feat: add init command for interactive config generation" --body "$(cat <<'EOF'
## Summary

Adds `scrat init` — an interactive command that detects project context and
generates a scrat config file (TOML or YAML, documented or minimal).

- Core: `plan_init()` runs ecosystem/branch/strategy detection, returns `InitPlan`
- Core: `generate_config()` renders config from templates with user selections
- CLI: ~6 interactive prompts with detected values as defaults
- Handles existing config: merge/overwrite/exit
- `--yes` flag for non-interactive use (CI/scripting)

## Test plan

- [ ] `just check` passes (fmt, clippy, deny, tests, doc-tests)
- [ ] `scrat init` in Rust project → detects Rust, generates correct config
- [ ] `scrat init` in empty dir → prompts for ecosystem
- [ ] `scrat init --yes --format yaml --style minimal` → non-interactive YAML
- [ ] `scrat init` with existing config → offers merge/overwrite/exit
EOF
)"
```
