# Agent Guide: scrat

Reference for AI agents helping users configure scrat release workflows.

scrat is a release automation CLI that orchestrates versioning, changelog generation, git operations, GitHub releases, and package publishing. It detects project ecosystems automatically, runs a phased release pipeline with safety checks, and supports hooks for custom automation at every stage.

## Commands

| Command | Purpose |
|---------|---------|
| `scrat info` | Show detected ecosystem, version strategy, tools, config location |
| `scrat preflight` | Validate release readiness (git state, branch, tools, auth) |
| `scrat bump` | Determine next version and update project files (without shipping) |
| `scrat notes` | Preview rendered release notes without shipping |
| `scrat ship` | Full release pipeline: preflight → version → test → bump → git → release → publish |
| `scrat init` | Generate a config file interactively or with `--yes` for defaults |
| `scrat doctor` | Show XDG directories, loaded config, environment |

All commands accept `--json` for machine-readable output. `scrat ship` and `scrat bump` accept `--dry-run`.

## Ship Phases

Phases execute in this order. Irreversibility increases left to right.

```
preflight → version → test → bump → git → release → publish
```

| Phase | What it does | Skip flag |
|-------|-------------|-----------|
| Preflight | Git repo, clean tree, release branch, remote sync, tools, auth | (always runs) |
| Version | Resolve next version via conventional commits, interactive, or explicit | (always runs) |
| Test | Run test suite | `--no-test` |
| Bump | Update version in project files, generate/update CHANGELOG | `--no-changelog` (changelog only) |
| Git | Stage, commit, tag, push | `--no-git` (all), `--no-tag`, `--no-push` |
| Release | Create/edit GitHub release with notes and assets | `--no-release` |
| Publish | Publish to package registry (crates.io, npm, etc.) | `--no-publish` |

Additional flags: `--no-deps` (skip dependency diff), `--no-stats` (skip commit statistics), `--no-notes` (use GitHub auto-generated notes instead of template), `--draft` / `--no-draft` (override config), `--yes` / `-y` (skip confirmation).

## Configuration

### File Discovery (highest to lowest precedence)

1. `--config <path>` (explicit)
2. `.config/scrat.toml` (or `.yaml`, `.json`) — project-level, searched upward to `.git` boundary
3. `.scrat.toml` — project-level alternate
4. `scrat.toml` — project-level alternate
5. User config — platform-dependent:
   - macOS: `~/Library/Application Support/scrat/config.toml`
   - Linux: `~/.config/scrat/config.toml` (XDG)

Project and user configs are deep-merged via Figment. Arrays (like hook lists) are **replaced**, not appended — a project-level hook list overrides the user-level list entirely.

### Full Config Schema

```toml
log_level = "info"  # debug | info | warn | error

[project]
type = "rust"       # rust | node | go | php | python | ruby | swift | generic
release_branch = "main"

[version]
strategy = "conventional-commits"  # conventional-commits | interactive | explicit
cliff_config = "/path/to/cliff.toml"

[commands]
test = "cargo nextest run"
build = "cargo build --release"
publish = "cargo publish"
clean = "cargo clean"

[release]
changelog_tool = "git-cliff"
github_release = true               # set false to skip GitHub release creation entirely
assets = ["dist/{repo}-{tag}.png"]  # paths support {var} interpolation
notes_template = "/path/to/template.tera"
draft = true                        # default: true
title = "Release {version}"         # supports {version}, {tag}, {owner}, {repo}
discussion_category = "Announcements"

[hooks]
pre_ship = []
post_ship = []
pre_test = []
post_test = []
pre_bump = []
post_bump = []
pre_tag = []
post_tag = []
pre_release = []
post_release = []
pre_publish = []
post_publish = []

[ship]
confirm = true      # prompt before executing (override with --yes)
no_publish = false   # permanently skip publish phase
no_release = false   # permanently skip release phase
no_test = false
no_tag = false
no_push = false
no_git = false
no_changelog = false
no_deps = false
no_stats = false
no_notes = false
```

### Config Patterns

**Repo where CI handles publishing** (scrat owns version/tag/release, GitHub Actions builds and publishes):

```toml
[project]
type = "rust"

[ship]
no_publish = true
```

**Repo where scrat handles everything** (no CD workflow needed):

```toml
[project]
type = "node"

[release]
draft = false
```

**Generic project with custom test command:**

```yaml
project:
  type: generic

commands:
  test: shellspec
```

## Ecosystem Detection

scrat auto-detects by marker file:

| Ecosystem | Marker | Test | Publish | Bump |
|-----------|--------|------|---------|------|
| Rust | `Cargo.toml` | `cargo nextest run` (or `cargo test`) | `cargo publish` | `cargo set-version` |
| Node | `package.json` | `npm test` | `npm publish` | `npm version --no-git-tag-version` (defined, not wired — returns error) |
| Go | `go.mod` | `go test ./...` | (none) | (none) |
| PHP | `composer.json` | `composer test` | (none) | (none) |
| Python | `pyproject.toml` | `pytest` | `twine upload dist/*` | (none) |
| Ruby | `Gemfile` | `bundle exec rake test` | `gem push` | (none) |
| Swift | `Package.swift` | `swift test` | (none) | (none) |
| Generic | (none — config or interactive only) | (none) | (none) | (none) |

Override with `[project] type = "..."` in config. Commands can be individually overridden in `[commands]`.

Rust-specific: scrat probes PATH for `cargo-nextest`, `cargo-edit` (for `cargo set-version`), and `git-cliff`. Missing tools degrade gracefully to fallbacks.

## Version Strategies

**Conventional Commits** (default when `git-cliff` is on PATH): Delegates to `git-cliff --bumped-version`. Rust projects treat breaking changes in 0.x as minor bumps (not major). All other ecosystems follow standard semver (breaking = major).

**Interactive**: Shows recent commits and presents patch/minor/major candidates. Used when git-cliff is not available.

**Explicit**: `scrat ship --version 1.2.3`. Validates semver format.

Override with `[version] strategy = "..."` or point to a custom cliff config with `[version] cliff_config = "/path/to/cliff.toml"`.

## Hooks

Hooks are arrays of shell commands that run at phase boundaries.

### Execution Model

Commands in a hook list run **in parallel** by default. Two prefixes create synchronization points:

- **`sync:`** — all prior commands finish, this command runs alone, then subsequent commands resume in parallel
- **`filter:`** — like `sync:`, but receives `PipelineContext` as JSON on **stdin** and must return modified JSON on **stdout**. The returned JSON replaces the pipeline context for all subsequent phases. Invalid JSON output aborts the release.

### Variable Interpolation

Hook commands support these variables:

| Variable | Value |
|----------|-------|
| `{version}` | New version (e.g., `1.2.3`) |
| `{prev_version}` | Previous version |
| `{tag}` | Git tag (e.g., `v1.2.3`) |
| `{owner}` | Repository owner from git remote |
| `{repo}` | Repository name from git remote |
| `{changelog_path}` | Path to CHANGELOG file |

### Example: Shared User Config with Hooks

This user config applies to all repos. It runs a release-context enrichment filter before ship, generates a postcard image after bump, and uses a custom release notes template:

```toml
[hooks]
pre_ship = [
    "filter: ~/scripts/release-context --corpus ~/corpus/quotes.jsonl",
]
post_bump = [
    "sync: claylo-graphics --config dist/postcard-{tag}.yaml && mv dist/{repo}-{tag}-xl.png dist/{repo}-{tag}.png",
]

[release]
notes_template = "/path/to/release-notes.tera"
assets = ["dist/{repo}-{tag}.png"]
```

Per-repo configs override hook lists entirely (Figment replaces arrays, does not merge them). If a repo needs different hooks, define the complete list in the project config.

## PipelineContext

The JSON structure that flows through the pipeline. Available to filter hooks (stdin/stdout), templates, and `--json` output.

```json
{
  "version": "1.2.3",
  "previous_version": "1.2.2",
  "tag": "v1.2.3",
  "previous_tag": "v1.2.2",
  "date": "2026-04-04",
  "owner": "claylo",
  "repo": "scrat",
  "repo_url": "https://github.com/claylo/scrat",
  "branch": "main",
  "ecosystem": "rust",
  "stats": {
    "commit_count": 42,
    "files_changed": 15,
    "insertions": 380,
    "deletions": 120,
    "contributors": [{"name": "clay", "count": 42}]
  },
  "dependencies": [
    { "name": "serde", "from": "1.0.200", "to": "1.0.210" }
  ],
  "changelog_updated": true,
  "changelog_path": "CHANGELOG.md",
  "modified_files": ["Cargo.toml", "Cargo.lock", "CHANGELOG.md"],
  "commit_hash": "abc1234",
  "release_url": "https://github.com/claylo/scrat/releases/tag/v1.2.3",
  "assets": ["dist/scrat-v1.2.3.png"],
  "release_notes": "## What's New\n...",
  "metadata": {},
  "dry_run": false
}
```

Fields are populated progressively — `commit_hash` is null until the git phase completes, `release_url` is null until the release phase completes. Filter hooks in `pre_ship` see a minimal context; filter hooks in `post_release` see the full context.

## Release Notes

scrat uses git-cliff for changelog and release notes generation with a two-pass approach:

1. `git-cliff --unreleased --context` produces a JSON array of release objects
2. scrat injects extra data (stats, dependencies, metadata) into the context
3. `git-cliff --from-context - --body <template>` renders the final markdown

### Custom Templates

Set `[release] notes_template` to a Tera template path. The template receives the full git-cliff release context with scrat's extras available directly as `extra` (e.g., `extra.stats`, `extra.deps`, `extra.metadata`). Preview with `scrat notes`.

### Default Behavior

Without a custom template, scrat uses git-cliff's built-in rendering with the project's `cliff.toml` configuration.

## GitHub Actions Integration

scrat is designed to coexist with GitHub Actions CI/CD workflows. The typical split:

### What scrat owns (local)

- Version bumping and changelog generation
- Git commit, tag, and push
- GitHub release creation (with custom notes, assets)

### What GitHub Actions owns (CI)

- Cross-platform binary builds
- Package publishing (crates.io, npm, Homebrew, deb, rpm)
- Artifact signing and attestation

### Workflow Architecture

Three workflow files, each with a distinct trigger:

**ci.yml** — quality gate on PRs and pushes to main:
- Lint (rustfmt, clippy), test (nextest), cargo-deny, MSRV check

**release.yml** — auto-release on push to main (optional):
- Detects releasable conventional commits via git-cliff
- Creates and pushes a version tag
- Gated by repo variable `AUTO_RELEASE_ENABLED`
- **Disable this for scrat-managed repos** — scrat creates the tag locally

**cd.yml** — triggered by `v*.*.*` tag push:
- Builds binaries for all platforms
- Creates GitHub release (if it doesn't exist) or uploads assets to existing release
- Publishes to registries gated by repo variables (`CRATES_IO_ENABLED`, `HOMEBREW_ENABLED`, `NPM_ENABLED`, `DEB_ENABLED`, `RPM_ENABLED`)

### The Handoff

When scrat pushes a tag, cd.yml triggers automatically. scrat has already created the GitHub release with custom notes and assets. cd.yml uploads binaries and publishes to registries without touching the release body or title.

For this to work:

1. Set `AUTO_RELEASE_ENABLED = false` (so release.yml doesn't also create the tag)
2. Set `[ship] no_publish = true` in project config (so scrat doesn't also publish)
3. cd.yml upload steps must not set `body` or `release_name` (so they don't overwrite scrat's notes)

### Repo Variables Reference

| Variable | Effect |
|----------|--------|
| `AUTO_RELEASE_ENABLED` | Enable release.yml auto-tagging |
| `AUTO_RELEASE_DRY_RUN` | Log only, don't create tags |
| `CRATES_IO_ENABLED` | Publish to crates.io |
| `HOMEBREW_ENABLED` | Update Homebrew tap formula |
| `NPM_ENABLED` | Publish to npm |
| `DEB_ENABLED` | Build and upload .deb package |
| `RPM_ENABLED` | Build and upload .rpm package |
| `SBOM_ENABLED` | Generate CycloneDX SBOM |
| `GPG_SIGNING_ENABLED` | Sign release artifacts with GPG |

## Common Workflows

### "I want scrat to do everything"

No CI publishing needed. scrat handles the full lifecycle.

```toml
[project]
type = "rust"

[release]
draft = false
```

### "I want scrat for versioning, CI for publishing"

scrat bumps, tags, creates the GitHub release. CI builds binaries and publishes to registries.

```toml
[project]
type = "rust"

[ship]
no_publish = true
```

Set `AUTO_RELEASE_ENABLED = false` on the repo so release.yml doesn't compete.

### "I want CI to handle everything automatically"

No scrat involved. release.yml detects version bumps and creates tags, cd.yml builds and publishes.

Set `AUTO_RELEASE_ENABLED = true` on the repo. No scrat config needed.

### "I want to preview before shipping"

```bash
scrat preflight        # check readiness
scrat bump --dry-run   # see what version would be picked
scrat notes            # preview release notes
scrat ship --dry-run   # full dry run
scrat ship             # interactive confirmation before execution
```

## Gotchas

- **`notes_template` must be a full path.** `std::fs::read_to_string` does not expand `~`. Use absolute paths in config.
- **User config location is platform-specific.** macOS uses `~/Library/Application Support/scrat/`, Linux uses `~/.config/scrat/`. The `directories` crate determines this. Run `scrat doctor` to confirm.
- **Figment replaces arrays.** A project-level `[hooks] post_bump = [...]` replaces the user-level list entirely. There is no merge — define the complete hook list wherever you define it.
- **First release (no prior tags).** `scrat ship` handles this — version strategy resolves from the full commit history. If you're previewing with external scripts, they need to fall back to the root commit when no tags exist.
- **`release.draft = true` is the default.** Releases are created as drafts unless overridden with `--no-draft` or `draft = false` in config.
- **Generic ecosystem skips publish.** If `type = "generic"`, the publish phase is always skipped regardless of `--no-publish`.
- **git-cliff treats Rust 0.x differently.** Breaking changes in 0.x bump minor, not major. All other ecosystems follow standard semver.

## Crate Structure

| Crate | Purpose |
|-------|---------|
| `scrat` | CLI layer — argument parsing, user interaction, progress display |
| `scrat-core` | Library — config, detection, versioning, hooks, ship orchestration, pipeline context |

Key modules in `scrat-core`:
- `config.rs` — config schema, file discovery, loading
- `ship.rs` — phase orchestration, `ShipOptions`, `ShipOutcome`
- `pipeline.rs` — `PipelineContext` accumulator
- `hooks.rs` — hook execution, sync/filter prefixes, variable interpolation
- `notes.rs` — release notes rendering via git-cliff
- `detect/` — ecosystem detection, tool probing
- `version/` — version strategies (conventional, interactive, explicit)
- `bump.rs` — version file updates, changelog generation
- `git.rs` — commit, tag, push operations
- `stats.rs` — commit statistics collection
- `deps.rs` — dependency diff computation
