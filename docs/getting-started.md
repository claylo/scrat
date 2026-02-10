# Getting Started with scrat

You're about to release your first version of something. Maybe it's a Rust crate, maybe it's a CLI tool you've been building for weeks. Either way, the feeling is the same: *what if I screw this up?*

That's why scrat exists. It won't let you shoot yourself in the foot.

Before it touches a single file, scrat checks your git state, verifies you're on the right branch, confirms your remote is in sync, detects your ecosystem, and validates that every tool it needs is installed. If anything's off, it stops and tells you exactly what's wrong. No partial releases. No "oops, I tagged from a feature branch." No silent failures.

## Install

```bash
cargo install scrat
```

scrat also needs a couple of companion tools for Rust projects:

| Tool | What it does | Install |
|------|-------------|---------|
| `cargo-nextest` | Faster test runner (optional — falls back to `cargo test`) | `cargo install cargo-nextest` |
| `cargo-edit` | Bumps version in Cargo.toml | `cargo install cargo-edit` |
| `git-cliff` | Generates changelogs from conventional commits | `cargo install git-cliff` |

scrat auto-detects which of these you have. If something's missing, `scrat preflight` will tell you.

## The 30-Second Version

```bash
# See what scrat knows about your project
scrat info

# Check if you're ready to release
scrat preflight

# Ship it — scrat shows you the plan and asks before executing
scrat ship
```

That's it. scrat confirms before doing anything destructive. Everything below is details.

## What scrat Detects Automatically

Run `scrat info` in your project directory. scrat walks your working directory for marker files and probes your `PATH` for available tools:

- **Ecosystem**: `Cargo.toml` → Rust, `package.json` → Node
- **Version strategy**: `cliff.toml` → conventional commits via git-cliff, `cog.toml` → cocogitto, neither → interactive picker
- **Test command**: `cargo nextest run` if nextest is installed, `cargo test` otherwise
- **Build command**: `cargo build --release`
- **Publish command**: `cargo publish`
- **Bump command**: `cargo set-version` (from cargo-edit)
- **Changelog tool**: git-cliff or cog, if their config files exist

You can override any of these in a config file, but you probably don't need to. The defaults are solid.

## Preflight: The Safety Net

`scrat preflight` runs six checks before it'll let you release anything:

```
  ✓ Git repository: Inside a git repository
  ✓ Working tree: Clean working tree
  ✓ Release branch: On release branch 'main'
  ✓ Remote sync: Local branch is in sync with remote
  ✓ Project detection: Detected rust project
  ✓ Required tools: All required tools are installed
```

If any check fails, scrat tells you exactly what's wrong:

```
  ✓ Git repository: Inside a git repository
  ✗ Working tree: Uncommitted changes in working tree
  ✗ Release branch: On 'feat/my-thing', expected 'main'
  ✓ Remote sync: Local branch is in sync with remote
  ✓ Project detection: Detected rust project
  ✗ Required tools: Missing tools: cargo-set-version

  3 check(s) failed — fix issues above before releasing
```

These same checks run automatically at the start of `scrat ship`. You can't skip them. That's the point.

## The Ship Workflow

`scrat ship` runs the full release pipeline in order:

```
preflight → test → bump → publish → git → release
```

Each phase does exactly one thing:

| Phase | What happens |
|-------|-------------|
| **preflight** | All six safety checks pass (or scrat stops) |
| **test** | Runs your test suite (`cargo nextest run` by default) |
| **bump** | Updates version in Cargo.toml, generates CHANGELOG.md |
| **publish** | Runs `cargo publish` to push to crates.io |
| **git** | Commits changes, creates annotated tag, pushes branch + tags |
| **release** | Creates a GitHub release with auto-generated notes via `gh` |

Tests run **before** the version bump. If tests fail, nothing has been modified. Your working tree is exactly how you left it.

### Confirmation: Look Before You Leap

By default, `scrat ship` shows you the plan and asks before executing:

```
Ship: 0.1.0 → 0.2.0
Strategy: conventional-commits (git-cliff) | Ecosystem: rust

  Phases: test, bump, publish, git, release
  Hooks: 3 hook commands

? Proceed with release? (Y/n)
```

Decline and nothing happens. This is the safe default — you always see what's about to happen before committing to it.

To skip the prompt (CI pipelines, scripted use):

```bash
# Flag override — skip confirmation for this run
scrat ship --yes
scrat ship -y

# Config override — skip confirmation permanently
# In .scrat.toml:
# [ship]
# confirm = false
```

### Dry Run: Full Preview Without Executing

```bash
scrat ship --dry-run
```

Walks through every phase, shows what *would* happen (including interpolated hook commands), and changes nothing. The confirmation prompt is skipped automatically since there's nothing to confirm. Useful for CI readiness checks and scripting.

### Skip What You Don't Need

Every phase has a flag to skip it:

```bash
# Don't publish to crates.io (maybe it's a private tool)
scrat ship --no-publish

# Don't push to remote (commit and tag locally only)
scrat ship --no-push

# Don't create a GitHub release
scrat ship --no-release

# Skip tests (you just ran them, or you're feeling lucky)
scrat ship --skip-tests

# Skip changelog generation
scrat ship --no-changelog
```

Combine freely. `scrat ship --no-publish --no-release` does a local-only release: bump, changelog, commit, tag, push.

### Set the Version Explicitly

If you don't want conventional commits to decide the version:

```bash
scrat ship --version 1.0.0
```

The `v` prefix is optional — `scrat ship --version v1.0.0` works too.

## Version Strategies

scrat supports three ways to determine the next version:

### Conventional Commits (automatic)

If you have a `cliff.toml` or `cog.toml` in your project root, scrat uses your commit messages to compute the next version:

- `fix:` commits → patch bump (0.0.X)
- `feat:` commits → minor bump (0.X.0)
- `feat!:` or `BREAKING CHANGE:` → major bump (X.0.0)

This is the zero-config path. Write conventional commits, and scrat figures out the rest.

### Interactive

If there's no `cliff.toml` or `cog.toml`, scrat shows you your recent commits and offers version candidates:

```
Recent commits:
  a1b2c3d feat: add ship command
  d4e5f6g fix: handle empty config
  h7i8j9k chore: update deps

Current version: 0.1.0

? Select version:
  0.2.0 (minor)
  0.1.1 (patch)
  1.0.0 (major)
```

### Explicit

Pass `--version` on the command line. Overrides everything.

## Bump Without Shipping

Sometimes you want to bump the version and generate a changelog without the full release pipeline:

```bash
# Preview
scrat bump --dry-run

# Do it
scrat bump

# Set version explicitly
scrat bump --version 2.0.0

# Skip changelog
scrat bump --no-changelog
```

`scrat bump` updates your project files but doesn't commit, tag, push, publish, or create a release. Useful when you want to review the changes before committing.

## Configuration

scrat works with zero configuration. But if you want to override defaults, create a `.scrat.toml` in your project root:

```toml
# Override the release branch (default: auto-detects main/master)
[project]
release_branch = "main"

# Override version strategy
[version]
strategy = "conventional-commits"  # or "interactive", "explicit"

# Override commands
[commands]
test = "cargo nextest run --all-features"
publish = "cargo publish --no-verify"

# Release settings
[release]
changelog_tool = "git-cliff"
github_release = true
assets = ["release-card.png", "checksums.txt"]

# Ship command behavior
[ship]
confirm = true  # default — set to false for CI/scripted use
```

Config files are discovered automatically:
1. `.scrat.toml` or `scrat.toml` in your project directory (searched up to the repo root)
2. `~/.config/scrat/config.toml` for user-wide defaults

Supported formats: TOML, YAML, JSON. Use whichever you prefer.

### Run `scrat doctor` to verify your setup

```bash
scrat doctor
```

Shows your config file location, directory paths, and environment variables. If no config file exists, it offers to create one.

## Hooks

Hooks let you run custom commands at phase boundaries during `scrat ship`. They're defined in your config file:

```toml
[hooks]
# Before/after the entire workflow
pre_ship = ["echo 'Starting release...'"]
post_ship = ["echo 'Release complete!'"]

# Before/after tests
pre_test = ["echo 'Warming up...'"]
post_test = ["echo 'Tests passed.'"]

# Before/after version bump + changelog
pre_bump = ["echo 'Pre-bump tasks'"]
post_bump = ["generate-release-card --version {version} --output release-card.png"]

# Before/after publishing to registry
pre_publish = ["cargo build --release"]
post_publish = ["echo 'Published {version}'"]

# Before/after git commit + tag + push
pre_tag = ["echo 'About to tag {tag}'"]
post_tag = ["echo 'Pushed {tag}'"]

# Before/after GitHub release creation
pre_release = ["echo 'Creating release...'"]
post_release = ["echo 'Released {owner}/{repo}@{tag}'"]
```

### Variables

Hook commands support these interpolation variables:

| Variable | Example value | Description |
|----------|---------------|-------------|
| `{version}` | `1.2.3` | The new version being released |
| `{prev_version}` | `1.1.0` | The previous version |
| `{tag}` | `v1.2.3` | The git tag |
| `{changelog_path}` | `/path/to/CHANGELOG.md` | Absolute path to the changelog |
| `{owner}` | `claylo` | Repository owner (from git remote) |
| `{repo}` | `scrat` | Repository name (from git remote) |

### Parallel Execution and Barriers

Hook commands in a phase run in parallel by default. If you need a command to run alone—waiting for everything before it and blocking everything after it—prefix it with `sync:`:

```toml
[hooks]
post_bump = [
    "generate-release-card --version {version}",
    "generate-checksums",
    "sync:sign-artifacts --version {version}",
    "upload-to-cdn",
    "notify-team",
]
```

In this example:
1. `generate-release-card` and `generate-checksums` run in parallel
2. Both finish, then `sign-artifacts` runs alone
3. After signing completes, `upload-to-cdn` and `notify-team` run in parallel

If any hook command fails, the remaining hooks and phases are skipped.

### Dry-Run Shows Your Hooks

`scrat ship --dry-run` previews hook commands with variables interpolated, so you can see exactly what would run:

```
  ○ bump Would bump 0.1.0 → 0.2.0
    hook → generate-release-card --version 0.2.0 --output release-card.png
```

## JSON Output

Every command supports `--json` for scripting:

```bash
scrat info --json
scrat preflight --json
scrat ship --dry-run --json
```

## Global Flags

These work with any subcommand:

| Flag | Short | Description |
|------|-------|-------------|
| `--config FILE` | `-c` | Use a specific config file |
| `--chdir DIR` | `-C` | Run as if started in DIR |
| `--quiet` | `-q` | Only print errors |
| `--verbose` | `-v` | More detail (repeatable: `-vv`) |
| `--color` | | `auto`, `always`, or `never` |
| `--json` | | Output as JSON |

## Your First Release: A Walkthrough

Here's the path from "I've never released this" to "it's on crates.io":

**1. Make sure your project is ready**

```bash
scrat doctor     # check your setup
scrat info       # see what scrat detected
scrat preflight  # verify release readiness
```

Fix anything that comes up. Common issues: uncommitted changes, not on main, missing `cargo-edit`.

**2. Ship it**

```bash
scrat ship
```

scrat shows you the version, which phases will run, and how many hooks are configured, then asks for confirmation. Say yes and it will:
- Run your tests (if they fail, it stops — nothing has changed)
- Bump the version in Cargo.toml
- Generate CHANGELOG.md
- Commit the changes with `chore: release X.Y.Z`
- Create an annotated tag `vX.Y.Z`
- Push the branch and tags
- Create a GitHub release

**4. If something goes wrong**

If a phase fails, scrat stops immediately. No partial state. Here's what to do:

- **Tests failed**: Fix the tests and try again. Nothing was modified.
- **Publish failed**: The version was bumped and committed locally but not pushed. Fix the publish issue (auth, network, etc.) and either retry or push manually.
- **Push failed**: Everything is committed and tagged locally. Fix the remote issue and `git push && git push --tags`.

The worst case is always "some things happened locally that you can inspect and fix." scrat never leaves you in a state where remote and local are inconsistent without telling you.

## What's Next

Once you've done your first release, you'll probably want to:

- Set up hooks in `.scrat.toml` for your specific workflow
- Add `scrat ship --dry-run` to your CI as a release readiness check
- Add `[ship] confirm = false` or use `scrat ship -y` in CI pipelines
- Configure release assets for things like release cards or checksums
