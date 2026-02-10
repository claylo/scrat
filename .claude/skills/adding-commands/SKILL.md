---
name: adding-commands
description: Guides agents through adding new CLI subcommands or xtask commands to this project. Use when the user asks to add a new command, subcommand, or xtask, or when implementing new CLI functionality.
---

# Adding Commands to scrat

Use this skill when adding new CLI subcommands or xtask development commands.

## Architecture Overview

This project uses a structured command pattern:

| Layer | Location | Purpose |
|-------|----------|---------|
| CLI parsing | `crates/scrat/src/lib.rs` | `Cli` struct with clap derive |
| Command dispatch | `crates/scrat/src/main.rs` | Match on `Commands` enum |
| Command impl | `crates/scrat/src/commands/*.rs` | Actual logic |
| Core library | `crates/scrat-core/` | Shared logic, config, errors |
| Build tasks | `xtask/` | Development/maintenance commands |

---

## Adding a CLI Command

### Step 1: Define the command args

Create `crates/scrat/src/commands/<name>.rs`:

```rust
//! <Name> command implementation

use clap::Args;
use tracing::{debug, info, instrument};
use scrat_core::Config;



/// Arguments for the `<name>` subcommand.
#[derive(Args)]
pub struct <Name>Args {
    /// Example flag
    #[arg(long)]
    pub verbose: bool,

    /// Example positional argument
    pub input: Option<String>,
}

#[instrument(skip(config), fields(command = "<name>"))]
pub fn cmd_<name>(args: <Name>Args, config: &Config) -> anyhow::Result<()> {
    debug!(?config.log_level, "executing <name> command");
info!(input = ?args.input, "processing");

    // Your implementation here

    Ok(())
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cmd_<name>_succeeds() {
        let args = <Name>Args { verbose: false, input: None };
let config = Config::default();
        assert!(cmd_<name>(args, &config).is_ok());
}
}
```

### Step 2: Export from mod.rs

Add to `crates/scrat/src/commands/mod.rs`:

```rust
pub mod <name>;
```

### Step 3: Add to Commands enum

In `crates/scrat/src/lib.rs`, add the variant:

```rust
use crate::commands::<name>::<Name>Args;

pub enum Commands {
    // ... existing commands ...

    /// Description of what <name> does
    <Name>(<Name>Args),
}
```

### Step 4: Wire up in main.rs

In `crates/scrat/src/main.rs`, add the match arm:

```rust
match cli.command {
    // ... existing arms ...
Commands::<Name>(args) => commands::<name>::cmd_<name>(args, &config),
}
```

### Step 5: Add CLI integration tests

Add to `crates/scrat/tests/cli.rs`:

```rust
#[test]
fn <name>_basic_usage() {
    Command::cargo_bin(env!("CARGO_PKG_NAME"))
        .unwrap()
        .arg("<name>")
        .assert()
        .success();
}

#[test]
fn <name>_with_flags() {
    Command::cargo_bin(env!("CARGO_PKG_NAME"))
        .unwrap()
        .args(["<name>", "--verbose"])
        .assert()
        .success();
}
```

---

## Adding an xtask Command

xtask commands are for development/maintenance tasks, not runtime commands. They don't use the app's config or logging.

### Step 1: Create the command file

Create `xtask/src/commands/<name>.rs`:

```rust
//! <Name> xtask command

use clap::Args;
use std::process::Command;

use crate::workspace_root;

#[derive(Args, Debug)]
pub struct <Name>Args {
    /// Example option
    #[arg(long)]
    pub dry_run: bool,
}

pub fn cmd_<name>(args: <Name>Args) -> Result<(), String> {
    let root = workspace_root();

    if args.dry_run {
        println!("Would run <name> in {:?}", root);
        return Ok(());
    }

    // Run external command example:
    let status = Command::new("cargo")
        .current_dir(&root)
        .args(["check", "--all-targets"])
        .status()
        .map_err(|e| format!("Failed to run cargo: {}", e))?;

    if !status.success() {
        return Err("Command failed".into());
    }

    Ok(())
}
```

### Step 2: Export and wire up

In `xtask/src/commands/mod.rs`:
```rust
pub mod <name>;
```

In `xtask/src/main.rs`, add to `Task` enum:
```rust
/// Description of what <name> does
<Name>(<Name>Args),
```

And add the match arm:
```rust
Task::<Name>(args) => commands::<name>::cmd_<name>(args),
```

---


## Observability Patterns

### Tracing Macros

Use these tracing macros for structured logging:

```rust
use tracing::{trace, debug, info, warn, error, instrument, span, Level};

// Simple messages
trace!("very detailed info");
debug!("debugging info");
info!("normal operation");
warn!("something unexpected");
error!("something failed");

// Structured fields
info!(user_id = %user.id, action = "login", "user logged in");
debug!(count = items.len(), "processing items");
error!(error = ?err, path = %file_path, "failed to read file");

// Field formatting:
// %value  - Display formatting
// ?value  - Debug formatting
// value   - direct (must impl Value trait)
```

### Instrumenting Functions

Use `#[instrument]` to automatically create spans:

```rust
#[instrument(skip(config), fields(command = "my_cmd"))]
pub fn cmd_my_command(args: MyArgs, config: &Config) -> anyhow::Result<()> {
    // Function body is automatically wrapped in a span
    // Arguments are recorded as span fields (except `config` which we skip)

    debug!("inside the span");

    Ok(())
}
```

Skip large or sensitive arguments:
```rust
#[instrument(skip(password, large_data), fields(user = %username))]
```



---

## Config Integration

### Accessing Config in Commands

Config is loaded in `main.rs` and passed to commands:

```rust
// In main.rs - config is already loaded
let config = ConfigLoader::new()
    .with_project_search(std::env::current_dir()?)
    .load()?;

// Pass to your command
Commands::MyCmd(args) => commands::my_cmd::cmd_my_cmd(args, &config),
```

### Using Config Values

```rust
pub fn cmd_my_cmd(args: MyArgs, config: &Config) -> anyhow::Result<()> {
    // Access config fields
    let log_level = config.log_level.as_str();
// Config fields are defined in:
// crates/scrat-core/src/config.rs
Ok(())
}
```

### Adding New Config Fields

1. Add field to `Config` struct in `config.rs`
2. Add default in `impl Default for Config`
3. Update example configs in `config/scrat.{toml,yaml}.example`


---

## Error Handling Patterns

### Use anyhow for Application Errors

```rust
use anyhow::{Context, Result, bail, ensure};

pub fn cmd_example(args: ExampleArgs) -> Result<()> {
    // Add context to errors
    let content = std::fs::read_to_string(&args.path)
        .with_context(|| format!("Failed to read {}", args.path.display()))?;

    // Early return with error
    if content.is_empty() {
        bail!("File is empty: {}", args.path.display());
    }

    // Assert with error
    ensure!(!content.is_empty(), "File must not be empty");

    Ok(())
}
```


### Use thiserror for Library Errors

In `crates/scrat-core/src/error.rs`:

```rust
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("configuration error: {message}")]
    Config { message: String },

    #[error("failed to process {path}")]
    Processing {
        path: String,
        #[source]
        source: std::io::Error,
    },
}
```


---

## JSON Output Pattern

Commands that produce data should support `--json` for scripting:

```rust
use serde::Serialize;

#[derive(Serialize)]
struct MyOutput {
    field: String,
    count: usize,
}

/// Arguments for `mycommand` subcommand
#[derive(Args)]
pub struct MyArgs {
    /// Output as JSON (command-level flag)
    #[arg(long)]
    pub json: bool,
}

pub fn cmd_my(args: MyArgs, global_json: bool) -> anyhow::Result<()> {
    let output = MyOutput { field: "value".into(), count: 42 };

    // Either --json flag works (global or command-specific)
    if args.json || global_json {
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("Field: {}", output.field);
        println!("Count: {}", output.count);
    }
    Ok(())
}
```

Update main.rs to pass `cli.json`:
```rust
Commands::My(args) => commands::my::cmd_my(args, cli.json),
```

---

## Checklist for New Commands

- [ ] Command file created in `src/commands/`
- [ ] Exported in `src/commands/mod.rs`
- [ ] Args struct added to `lib.rs` Commands enum
- [ ] Match arm added in `main.rs`
- [ ] Config passed if needed
- [ ] `#[instrument]` added for tracing
- [ ] Appropriate log levels used (debug for dev, info for user-visible)
- [ ] Unit tests in command file
- [ ] Integration tests in `tests/cli.rs`
- [ ] Help text is clear and useful
- [ ] JSON output supported if command produces data (accept `global_json` param)
