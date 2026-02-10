# scrat

[![CI](https://github.com/claylo/scrat/actions/workflows/ci.yml/badge.svg)](https://github.com/claylo/scrat/actions/workflows/ci.yml)
[![Crates.io](https://img.shields.io/crates/v/scrat.svg)](https://crates.io/crates/scrat)
[![docs.rs](https://docs.rs/scrat/badge.svg)](https://docs.rs/scrat)
[![MSRV](https://img.shields.io/badge/MSRV-1.88.0-blue.svg)](https://github.com/claylo/scrat)

Release management tooling focused on sanity retention


## Features


- **Hierarchical Configuration** - Automatic config discovery from project directories up to user home

- **Structured Logging** - Daily-rotated JSONL log files
- **JSON Output** - Machine-readable output for scripting and automation
- **Shell Completions** - Tab completion for Bash, Zsh, Fish, and PowerShell
- **Man Pages** - Unix-style documentation

## Installation

### Homebrew (macOS and Linux)

```bash
brew install claylo/brew/scrat
```

### Pre-built Binaries

Download the latest release for your platform from the [releases page](https://github.com/claylo/scrat/releases).

Binaries are available for:
- macOS (Apple Silicon and Intel)
- Linux (x86_64 and ARM64, glibc and musl)
- Windows (x86_64 and ARM64)

### From Source

```bash
cargo install scrat
```

Or build from source:

```bash
git clone https://github.com/claylo/scrat.git
cd scrat
cargo install --path crates/scrat
```

### Shell Completions

Shell completions are included in release archives and Homebrew installs. For manual installation, see [Shell Completions](#shell-completions) below.

## Usage

```bash
# Show version and build information
scrat info

# JSON output for scripting
scrat info --json

# Enable verbose output
scrat --verbose <command>
```


## Configuration

Configuration files are discovered automatically in order of precedence (highest first):

1. `.scrat.<ext>` in current directory or any parent
2. `scrat.<ext>` in current directory or any parent
3. `~/.config/scrat/config.<ext>` (user config)

**Supported formats:** TOML, YAML, JSON (extensions: `.toml`, `.yaml`, `.yml`, `.json`)

Values from higher-precedence files override lower ones. Missing files are silently ignored.

See the example configurations in the repository root for templates.

### Example Configuration

**TOML** (`~/.config/scrat/config.toml`):
```toml
log_level = "info"
```

**YAML** (`~/.config/scrat/config.yaml`):
```yaml
log_level: info
```

**JSON** (`~/.config/scrat/config.json`):
```json
{
  "log_level": "info"
}
```

### Configuration Options

| Option | Values | Default | Description |
|--------|--------|---------|-------------|
| `log_level` | `debug`, `info`, `warn`, `error` | `info` | Minimum log level to display |
| `log_dir` | path | platform default | Directory for JSONL log files |


## Logging

Logs are written as **JSONL** to a daily-rotated file.
Rotation is date-suffixed (e.g. `scrat.jsonl.2026-01-06`).

> **Note**: Logs never write to stdout, which is reserved for application output
> (important for tools like MCP servers that use stdout for communication).

Default log path (first writable wins):

1. `/var/log/scrat.jsonl` (Unix only, requires write access)
2. OS user data directory (e.g. `~/.local/share/scrat/logs/scrat.jsonl`)
3. Falls back to stderr if no writable directory is found

Overrides:

- `SCRAT_LOG_PATH` — full file path
- `SCRAT_LOG_DIR` — directory (file name defaults to `scrat.jsonl`)
- `SCRAT_ENV` — environment tag (default: `dev`)
- Config file key: `log_dir`


## Shell Completions

Shell completions are included in the release archives. To install manually:

**Bash**
```bash
scrat completions bash > ~/.local/share/bash-completion/completions/scrat
```

**Zsh**
```bash
scrat completions zsh > ~/.zfunc/_scrat
```

**Fish**
```bash
scrat completions fish > ~/.config/fish/completions/scrat.fish
```

**PowerShell**
```powershell
scrat completions powershell > $PROFILE.CurrentUserAllHosts
```

## Development

This project uses a workspace layout with multiple crates:

```
crates/
├── scrat/       # CLI binary
└── scrat-core/  # Core library (config, errors)
```

### Prerequisites

- Rust 1.88.0+ (2024 edition)
- [just](https://github.com/casey/just) (task runner)
- [cargo-nextest](https://nexte.st/) (test runner)

### Quick Start

```bash
# List available tasks
just --list

# Run full check suite (format, lint, test)
just check

# Run tests only
just test

# Run with coverage
just cov
```

### Build Tasks

| Command | Description |
|---------|-------------|
| `just check` | Format, lint, and test |
| `just fmt` | Format code with rustfmt |
| `just clippy` | Run clippy lints |
| `just test` | Run tests with nextest |
| `just doc-test` | Run documentation tests |
| `just cov` | Generate coverage report |


### xtask Commands

The project includes an xtask crate for build automation:

```bash
# Generate man pages
cargo xtask man

# Generate shell completions
cargo xtask completions

# Generate for specific shell
cargo xtask completions --shell zsh
```

## Architecture


### Crate Organization


- **scrat** - The CLI binary. Handles argument parsing, command dispatch, and user interaction.
- **scrat-core** - The core library. Contains configuration loading, error types, and shared functionality.

### Error Handling

- Libraries use `thiserror` for structured error types
- Binaries use `anyhow` for flexible error propagation
- All errors include context for debugging


### Configuration System

The `ConfigLoader` provides flexible configuration discovery:

```rust
use scrat_core::config::{Config, ConfigLoader};

let config = ConfigLoader::new()
    .with_project_search(std::env::current_dir()?)
    .with_user_config(true)
    .load()?;
```

Features:
- Walks up directory tree looking for config files
- Stops at repository boundaries (`.git` by default)
- Merges multiple config sources with clear precedence
- Supports explicit file paths for testing

## CI/CD

This project uses GitHub Actions for continuous integration:

- **Build & Test** - Runs on every push and PR
- **MSRV Check** - Verifies minimum supported Rust version
- **Clippy** - Enforces lint rules
- **Coverage** - Tracks test coverage

### Dependabot

This project uses Dependabot for security monitoring, but **not** for automatic pull requests. Instead:

1. Dependabot scans for vulnerabilities in dependencies
2. A weekly GitHub Actions workflow converts alerts into **issues**
3. Maintainers review and address updates manually

This approach provides:
- Full control over when and how dependencies are updated
- Opportunity to batch related updates together
- Time to test updates before merging
- Cleaner git history without automated PR noise

Security alerts appear as issues labeled `dependabot-alert`.

## Contributing

Contributions welcome! Please see [AGENTS.md](AGENTS.md) for development conventions.

### Commit Messages

This project uses [Conventional Commits](https://www.conventionalcommits.org/):

- `feat:` - New features
- `fix:` - Bug fixes
- `docs:` - Documentation changes
- `perf:` - Performance improvements
- `chore:` - Maintenance tasks

### Code Style

- Rust 2024 edition
- `#![deny(unsafe_code)]` - Safe Rust only
- Follow `rustfmt` defaults
- Keep clippy clean

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

