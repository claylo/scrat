# scrat

[![CI](https://github.com/claylo/scrat/actions/workflows/ci.yml/badge.svg)](https://github.com/claylo/scrat/actions/workflows/ci.yml)
[![Crates.io](https://img.shields.io/crates/v/scrat.svg)](https://crates.io/crates/scrat)
[![docs.rs](https://docs.rs/scrat/badge.svg)](https://docs.rs/scrat)
[![MSRV](https://img.shields.io/badge/MSRV-1.88.0-blue.svg)](https://github.com/claylo/scrat)

**Release management tooling focused on sanity retention.**

scrat is a batteries-included release pipeline for projects that use git tags and GitHub releases.
It detects your ecosystem,
diffs your dependencies,
collects stats,
renders release notes,
bumps versions,
commits, tags, pushes,
and creates a GitHub release—all in one command.
Every step is on by default, every step is skippable, and hooks let you bolt on anything custom.

Think of it as [np](https://github.com/sindresorhus/np) for any ecosystem,
with built-in release notes via [git-cliff](https://git-cliff.org/).


## Table of Contents

- [Why](#why)
- [Quick Start](#quick-start)
- [The Pipeline](#the-pipeline)
- [Commands](#commands)
- [Configuration](#configuration)
- [Hooks](#hooks)
- [CLI Reference](#cli-reference)
- [Installation](#installation)
- [Development](#development)
- [License](#license)


## Why

Every release I've ever done by hand has the same steps:
run tests, bump versions, update the changelog, commit, tag, push, create a GitHub release,
attach assets, and try not to forget something.
Most tools automate one or two of those steps.
scrat automates all of them—and the ones it doesn't know about, you wire up with hooks.

**Design principles:**

- **Batteries included, everything optional.**
  Every built-in step is on by default.
  Pass `--no-<step>` to skip it.
  Set a value in config to override the built-in behavior.
  Or disable it entirely.
- **Thin CLI, fat core.**
  The binary is a thin UI layer.
  All orchestration lives in `scrat-core` so other tools can embed it.
- **Hooks over built-ins for custom stuff.**
  scrat doesn't know about your postcard generator or your quote corpus.
  Declare shell commands as hooks—they run at every phase boundary with variable interpolation.
- **Zero config works. Override what you want.**
  Auto-detection handles ecosystem, version strategy, test commands, and publish commands.
  Config files only exist to override the defaults.


## Quick Start

```bash
# Install
cargo install scrat

# See what scrat detects about your project
scrat info

# Check if you're ready to release
scrat preflight

# Preview release notes without shipping
scrat notes

# Dry run the full pipeline
scrat ship --dry-run

# Ship it
scrat ship
```

`scrat ship` is the main event.
It runs every stage below, shows you the plan, asks for confirmation, and executes.


## The Pipeline

`scrat ship` runs these stages in order.
Each stage feeds structured data into a `PipelineContext` that flows through the whole pipeline.
Hooks can read and mutate this context at every phase boundary.

### 1. Preflight

Checks release readiness before anything else runs.

- Clean working directory (no uncommitted changes)
- On the correct release branch (`main` or `master`, configurable)
- git-cliff installed (required for release notes)

If any check fails, the pipeline stops.
Run `scrat preflight` standalone to diagnose issues.

### 2. Version Resolution

Determines the next version.
Three strategies, auto-detected:

| Strategy | When | How |
|----------|------|-----|
| **Conventional Commits** | `cliff.toml` present | Analyzes commit messages to determine major/minor/patch |
| **Explicit** | `--version 1.2.3` passed | Uses exactly what you give it |
| **Interactive** | Fallback | Shows recent commits, offers version candidates, you pick |

scrat reads the current version from your project files
(`Cargo.toml`, `package.json`, etc.)
and computes candidates from there.

### 3. Test

Runs your test suite.
The command is auto-detected per ecosystem:

| Ecosystem | Default Command |
|-----------|----------------|
| Rust | `cargo test` |
| Node | `npm test` |
| PHP (Composer) | `composer test` |
| Python | `pytest` |
| Go | `go test ./...` |

Override with `commands.test` in config.
Skip with `--no-test`.

### 4. Bump

Updates version numbers in project files and generates the changelog.

- Writes the new version to `Cargo.toml`, `package.json`, etc.
- Runs `git-cliff` to update `CHANGELOG.md`
- Reports which files were modified

Skip changelog generation with `--no-changelog`.
Run `scrat bump` standalone to bump without shipping.

### 5. Publish

Publishes to a package registry.
Auto-detected:

| Ecosystem | Default Command |
|-----------|----------------|
| Rust | `cargo publish` |
| Node | `npm publish` |

Skip with `--no-publish`.
Override with `commands.publish` in config.

### 6. Dependency Diff

Diffs lockfiles between the previous tag and HEAD to find what changed.
Supports:

- `Cargo.lock`
- `package-lock.json`
- `composer.lock`
- `Gemfile.lock`
- `go.sum`
- `requirements.txt` / `poetry.lock`

The diff parses `git diff` output—not the full lockfile format—so it's fast
and doesn't need ecosystem-specific parsers.
Results feed into release notes automatically.

Skip with `--no-deps`.

### 7. Stats Collection

Gathers release statistics from git:

- Commit count
- Files changed, insertions, deletions
- Contributors and their commit counts

Uses `git diff --shortstat` and `git shortlog`.
Results feed into release notes.

Skip with `--no-stats`.

### 8. Release Notes

Renders release notes using a two-pass git-cliff pattern:

1. `git-cliff --unreleased --context` produces JSON with commits grouped by type
2. scrat injects extra data (deps, stats, metadata) into the context's `extra` field
3. `git-cliff --from-context - --body <template>` renders the final markdown

scrat ships a built-in template with:
breaking changes, grouped commits with emoji,
dependency changes (updated/added/removed),
a stats table, and a "nerd drawer" with contributor details.

Point to your own template with `release.notes_template` in config
or `--template` on `scrat notes`.

Skip with `--no-notes`.
Falls back to `--generate-notes` (GitHub's auto-generated notes) if rendering fails.

### 9. Git

Commits, tags, and pushes.

- `git add . && git commit -m "chore: release {version}"`
- `git tag -a v{version} -m "Release {version}"`
- `git push origin {branch} && git push origin --tags`

Fine-grained control:

| Flag | Effect |
|------|--------|
| `--no-git` | Skip entire phase (commit, tag, push) |
| `--no-tag` | Commit and push, but don't create a tag |
| `--no-push` | Commit and tag locally, don't push |

### 10. GitHub Release

Creates (or updates) a GitHub release using `gh`.

- **Auto-detects edit vs. create:**
  if a release already exists for the tag, it edits and re-uploads assets instead of failing.
  This makes `scrat ship` safe to re-run after a partial failure.
- **Draft by default:**
  releases are created as drafts so you can review before publishing.
  Publish with `gh release edit <tag> --draft=false`.
- **Configurable title:**
  `release.title = "{repo} {tag}"` with hook-style variable interpolation.
- **Assets:**
  declare `release.assets = ["dist/app.tar.gz", "checksums.txt"]` in config.
  Hook commands produce these files; scrat attaches them.

Skip with `--no-release`.
Override draft behavior with `--draft` / `--no-draft`.


## Commands

### `scrat ship`

The full release pipeline.
Runs all stages above, with confirmation prompt.

```bash
scrat ship                    # interactive — asks for confirmation
scrat ship --dry-run          # preview without changes
scrat ship --version 2.0.0    # explicit version
scrat ship --no-publish -y    # skip publish, skip confirmation
scrat ship --draft            # force draft mode (overrides config)
```

### `scrat notes`

Renders release notes without shipping.
Useful for previewing what the notes will look like.

```bash
scrat notes                          # preview notes for current version
scrat notes --from v1.0.0            # diff against specific tag
scrat notes --version 2.0.0          # render as if releasing 2.0.0
scrat notes --template my-notes.tera # use custom template
scrat notes --json                   # output raw context as JSON
```

### `scrat bump`

Bumps version and generates changelog without shipping.

```bash
scrat bump                    # interactive version selection
scrat bump --version 1.2.3    # explicit version
scrat bump --dry-run          # preview without changes
scrat bump --no-changelog     # skip changelog generation
```

### `scrat preflight`

Checks release readiness.

```bash
scrat preflight               # run all checks
scrat preflight --json        # machine-readable output
```

### `scrat info`

Shows project information: detected ecosystem, version, tools, config paths.

```bash
scrat info                    # human-readable
scrat info --json             # machine-readable
```

### `scrat doctor`

Diagnoses configuration and environment issues.


## Configuration

Config files are discovered automatically.
Precedence (highest first):

1. Explicit file via `--config <path>`
2. `.scrat.toml` / `scrat.toml` in current directory (walks up to `.git` boundary)
3. `~/.config/scrat/config.toml` (user config)
4. Built-in defaults

**Supported formats:** TOML, YAML, JSON.

Zero config works.
Everything below is optional—only set what you want to override.

### Full Reference

```toml
# Log level: debug, info, warn, error
log_level = "info"

# Directory for JSONL log files (default: platform-specific)
# log_dir = "/var/log/scrat"

[project]
# Override detected ecosystem: rust, node, php, python, go
# type = "rust"
# Override release branch (default: auto-detect main/master)
# release_branch = "main"

[version]
# Override version strategy: conventional-commits, interactive, explicit
# strategy = "conventional-commits"

[commands]
# Override per-phase commands (default: auto-detected per ecosystem)
# test = "just test"
# build = "cargo build --release"
# publish = "cargo publish"
# clean = "cargo clean"

[release]
# Create GitHub releases (default: true)
# github_release = true

# Create as draft — review before publishing (default: true)
# draft = true

# Title format with variable interpolation (default: tag name)
# title = "{repo} {tag}"

# GitHub Discussions category (only for new releases)
# discussion_category = "releases"

# Custom git-cliff template for release notes
# notes_template = "templates/my-notes.tera"

# Files to attach to the GitHub release
# assets = ["dist/release-card.png", "dist/checksums.txt"]

[hooks]
# Shell commands at each phase boundary.
# See the Hooks section for details.
# post_bump = ["ll-graphics generate --version {version} --output dist/release-card.png"]

[ship]
# Prompt for confirmation before executing (default: true)
# Set to false for CI/scripted use. --yes/-y flag also skips.
# confirm = true
```


## Hooks

Hooks are shell commands that run at phase boundaries during the ship workflow.
Declare them in config as lists of strings.

### Hook Points

14 hook points across 7 phases:

| Hook | When |
|------|------|
| `pre_ship` / `post_ship` | Before/after the entire workflow |
| `pre_test` / `post_test` | Before/after the test phase |
| `pre_bump` / `post_bump` | Before/after version bump + changelog |
| `pre_publish` / `post_publish` | Before/after registry publish |
| `pre_tag` / `post_tag` | Before/after git commit + tag + push |
| `pre_release` / `post_release` | Before/after GitHub release creation |

### Variable Interpolation

Commands support `{var}` placeholders:

| Variable | Example Value |
|----------|---------------|
| `{version}` | `1.2.3` |
| `{prev_version}` | `1.1.0` |
| `{tag}` | `v1.2.3` |
| `{changelog_path}` | `CHANGELOG.md` |
| `{owner}` | `claylo` |
| `{repo}` | `scrat` |

### Execution Model

Commands run **in parallel** by default.
Two prefixes alter execution:

**`sync:` — barrier.**
All prior commands finish, the sync command runs alone, then subsequent commands resume in parallel.

```toml
[hooks]
post_bump = [
    "generate-image --version {version}",
    "generate-checksums",
    "sync: validate-artifacts",  # waits for both above, runs alone
    "upload-to-cdn",             # resumes parallel
]
```

**`filter:` — barrier + JSON piping.**
Like `sync:`, but the command also receives the full `PipelineContext` as JSON on stdin
and must return valid JSON on stdout.
The output replaces the pipeline context.
This lets you mutate built-in step output without replacing the whole step.

```toml
[hooks]
post_bump = [
    "filter: jq '[.dependencies[] | select(.name != \"dev-dep\")]'",
]
```

### Example: Full Hook Setup

```toml
[hooks]
post_bump = [
    "ll-graphics release-postcard --tag {tag} --output dist/release-card.png",
]
pre_release = [
    "sync: test -f dist/release-card.png",
]

[release]
assets = ["dist/release-card.png"]
```


## CLI Reference

### Global Options

| Flag | Description |
|------|-------------|
| `-c, --config <FILE>` | Explicit config file path |
| `-C, --chdir <DIR>` | Run as if started in DIR |
| `-v, --verbose` | More detail (repeatable: `-vv`) |
| `-q, --quiet` | Only print errors |
| `--json` | Machine-readable JSON output |
| `--color <auto\|always\|never>` | Colorize output |

### `scrat ship` Flags

**Step control** — every pipeline step is skippable:

| Flag | Skips |
|------|-------|
| `--no-test` | Test phase |
| `--no-changelog` | Changelog generation (during bump) |
| `--no-publish` | Registry publish |
| `--no-deps` | Dependency diff |
| `--no-stats` | Stats collection |
| `--no-notes` | Release notes rendering |
| `--no-tag` | Git tag (still commits and pushes) |
| `--no-push` | Git push (still commits and tags locally) |
| `--no-git` | Entire git phase (commit, tag, push) |
| `--no-release` | GitHub release creation |

**Other options:**

| Flag | Description |
|------|-------------|
| `--version <VERSION>` | Set version explicitly |
| `--draft` | Force draft mode (overrides config) |
| `--no-draft` | Force published mode (overrides config) |
| `--dry-run` | Preview without making changes |
| `-y, --yes` | Skip confirmation prompt |


## Installation

### Homebrew (macOS and Linux)

```bash
brew install claylo/brew/scrat
```

### Pre-built Binaries

Download from the [releases page](https://github.com/claylo/scrat/releases).

Binaries available for:

- macOS (Apple Silicon and Intel)
- Linux (x86_64 and ARM64, glibc and musl)
- Windows (x86_64 and ARM64)

### From Source

```bash
cargo install scrat
```

### Shell Completions

Included in release archives and Homebrew installs. For manual setup:

```bash
# Bash
scrat completions bash > ~/.local/share/bash-completion/completions/scrat

# Zsh
scrat completions zsh > ~/.zfunc/_scrat

# Fish
scrat completions fish > ~/.config/fish/completions/scrat.fish
```

### Prerequisites

scrat shells out to these tools when their features are used:

| Tool | Required For | Install |
|------|-------------|---------|
| [git-cliff](https://git-cliff.org/) | Changelog + release notes | `cargo install git-cliff` |
| [gh](https://cli.github.com/) | GitHub release creation | `brew install gh` |


## Development

Workspace layout:

```
crates/
  scrat/       # CLI binary — thin UI layer
  scrat-core/  # Core library — all orchestration and logic
xtask/         # Build automation (man pages, completions, install)
```

### Requirements

- Rust 1.88.0+ (edition 2024)
- [just](https://github.com/casey/just)
- [cargo-nextest](https://nexte.st/)

### Build Tasks

| Command | Description |
|---------|-------------|
| `just check` | Format + clippy + deny + test + doc-test |
| `just test` | Run tests with nextest |
| `just clippy` | Run clippy lints |
| `just fmt` | Format with rustfmt |
| `just cov` | Coverage report |
| `just deny` | Security/license audit |

### Architecture

- **Thin CLI, fat core:**
  the binary parses args, calls core, maybe prompts the user, displays results.
  If you're importing `deps`, `stats`, `detect`, `git`, or `pipeline` in the CLI crate,
  it belongs in core.
- **Plan/execute pattern:**
  `plan_ship()` returns `Ready` or `NeedsInteraction`.
  The CLI only prompts on the latter.
  `ReadyShip::execute()` runs the pipeline with event callbacks for progress display.
- **Error handling:**
  `thiserror` in the library, `anyhow` in the binary.
- **Safe Rust only:**
  `#![deny(unsafe_code)]` workspace-wide.

See [AGENTS.md](AGENTS.md) for full development conventions.


## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.
