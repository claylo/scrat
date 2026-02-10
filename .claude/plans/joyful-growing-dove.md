# scrat: Release Management CLI

## Context

scrat started as a Rust CLI, briefly pivoted to a pure `just` script, and is now returning to Rust — informed by what we learned. The just experiment clarified that scrat's value isn't in being a task runner; it's the **intelligence layer** that sits above one. The version determination problem (conventional commits vs interactive prompt vs explicit) requires strategy detection, interactive TUI, and multi-ecosystem awareness — things that are natural in Rust but painful in shell.

**What scrat is:** An `np`-like release workflow tool that's opinionated per ecosystem but configurable. It knows that Rust means `cargo test` (or `cargo nextest run` if available), Node means `npm test`, etc. It automates the full release pipeline: preflight checks → version determination → test → bump → publish → git finalize → release notes → GitHub release.

**What scrat is NOT:** A task runner. It delegates to ecosystem tools, not to `just` or `make`.

---

## Architecture

### Crate structure

```
Cargo.toml              # workspace root
crates/
  scrat/                # CLI binary
    src/
      main.rs
      lib.rs            # Cli struct, Commands enum
      commands/
        mod.rs
        doctor.rs       # (existing) diagnose environment
        info.rs         # (existing) show package/config info
        preflight.rs    # git state validation
        bump.rs         # version determination + bump
        test.rs         # run project tests
        ship.rs         # full release orchestrator
      detect/
        mod.rs          # project type + tool detection
        rust.rs
        node.rs         # (stub for now)
      version/
        mod.rs          # version strategy trait
        conventional.rs # CC auto-compute (git-cliff/cog)
        interactive.rs  # inquire-based semver picker
        explicit.rs     # CLI arg passthrough
      git/
        mod.rs          # git operations (branch, tag, clean check)
        stats.rs        # commit stats between refs
        deps.rs         # lockfile diff parsing
      release/
        mod.rs
        notes.rs        # changelog generation (delegates to git-cliff)
        github.rs       # gh CLI wrapper for draft releases
        postcard.rs     # optional: ll-graphics + resvg
  scrat-core/           # shared library
    src/
      lib.rs
      config.rs         # extended config with project/version/commands
      error.rs
      ecosystem.rs      # Ecosystem enum, tool registry, smart defaults
```

### Config model (`.scrat.toml`)

```toml
# Most fields auto-detected — config is for overrides only

[project]
# type = "rust"  # auto-detected from Cargo.toml / package.json / etc.

[version]
# strategy = "conventional-commits"  # auto-detected from cog.toml / cliff.toml
# Possible values: "conventional-commits", "interactive", "explicit"

[commands]
# Smart defaults per ecosystem. Override any phase:
# test = "cargo nextest run"
# build = "cargo build --release"
# publish = "cargo publish"
# clean = "cargo clean"

[release]
# changelog_tool = "git-cliff"  # or "cog"
# github_release = true
# postcard = false  # opt-in, requires ll-graphics + resvg

[release.notes]
# template = "templates/release-notes.tera"  # custom template path
# quote_corpus = "corpus/quotes.jsonl"       # quote database path
```

### Smart defaults per ecosystem

```
Rust (detected via Cargo.toml):
  test:    cargo nextest run  (if nextest installed, else cargo test)
  build:   cargo build --release
  publish: cargo publish
  version: read from Cargo.toml [package] or [workspace.package]
  bump:    cargo set-version (if cargo-edit installed)
  deps:    parse Cargo.lock diff
  changelog: git-cliff (if cliff.toml exists, else cog if cog.toml exists)

Node (detected via package.json) — future:
  test:    npm test
  build:   npm run build (if script exists)
  publish: npm publish
  version: read from package.json
  bump:    npm version --no-git-tag-version
  deps:    parse package-lock.json diff
```

### Version strategy detection

```
Has cliff.toml OR cog.toml?
  → "conventional-commits" (auto-compute from commit history)

Has neither?
  → "interactive" (show commits since last tag, prompt patch/minor/major)

User passes --version v1.2.3 on CLI?
  → "explicit" (override everything)
```

### The `ship` workflow (np-inspired phases)

```
Phase 1: PREFLIGHT
  ├─ Working tree clean?
  ├─ On release branch? (detect: main > master, or --branch override)
  ├─ Remote in sync? (no unpulled commits)
  └─ Required tools installed? (ecosystem-specific)

Phase 2: VERSION
  ├─ Determine strategy (CC auto / interactive / explicit)
  ├─ Compute or prompt for next version
  └─ Validate semver

Phase 3: TEST
  ├─ Run test command (smart default or config override)
  └─ Fail fast if tests fail (--yolo to skip, like np)

Phase 4: BUMP
  ├─ Update version in project files (Cargo.toml, package.json, etc.)
  ├─ Generate/update CHANGELOG.md
  └─ Verify working tree is clean after bump

Phase 5: PUBLISH (optional, --no-publish to skip)
  ├─ Build release artifact
  └─ Publish to registry (crates.io, npm, etc.)

Phase 6: GIT FINALIZE
  ├─ Commit version bump + changelog
  ├─ Create annotated tag
  └─ Push commits + tags (--no-push to skip)

Phase 7: RELEASE (optional, --no-release to skip)
  ├─ Collect git stats (commits, files, authors)
  ├─ Generate release notes (template + stats + deps)
  ├─ Generate postcard (if configured)
  └─ Create GitHub draft release with notes + assets
```

---

## Implementation Plan — Milestone 1: Detection + Config Extension

User will restore the existing Rust workspace (Cargo.toml, scrat-core, scrat CLI with doctor/info, justfile, CI, etc.). This milestone adds new code on top of that foundation.

### New files

- `crates/scrat-core/src/ecosystem.rs` — `Ecosystem` enum, tool detection, smart defaults
- `crates/scrat/src/detect/mod.rs` — project type detection logic
- `crates/scrat/src/detect/rust.rs` — Rust-specific detection

### Config extension

Extend existing `Config` struct in `crates/scrat-core/src/config.rs` with new sections:
- `project: ProjectConfig` (type override, release branch)
- `version: VersionConfig` (strategy override)
- `commands: CommandsConfig` (test/build/publish overrides)
- `release: ReleaseConfig` (changelog tool, github release, postcard)

All fields optional — auto-detection fills in defaults, config overrides.

### New dependency

```toml
# scrat (CLI) — add to existing deps
which = "7"  # detect installed tools on PATH
```

### What detection looks like

```rust
pub enum Ecosystem {
    Rust,
    Node,
    // Python, Go, etc. — future
}

pub struct ProjectDetection {
    pub ecosystem: Ecosystem,
    pub version_strategy: VersionStrategy,
    pub tools: DetectedTools,
}

pub struct DetectedTools {
    pub test_cmd: String,      // "cargo nextest run" or "cargo test"
    pub build_cmd: String,
    pub publish_cmd: Option<String>,
    pub changelog_tool: Option<ChangelogTool>,
}

pub enum VersionStrategy {
    ConventionalCommits { tool: ChangelogTool },
    Interactive,
    Explicit(String),
}

pub enum ChangelogTool {
    GitCliff,
    Cog,
}
```

Detection logic:
1. Walk current directory for marker files (`Cargo.toml`, `package.json`, etc.)
2. Determine ecosystem
3. Probe PATH for available tools (`which::which("cargo-nextest")`, etc.)
4. Check for CC config files (`cliff.toml`, `cog.toml`)
5. Assemble `ProjectDetection` with smart defaults

---

## Implementation Plan — Milestone 2: Preflight + Version

Add the `preflight` and `bump` commands.

### `scrat preflight`
- Check git working tree is clean
- Detect and validate release branch
- Check remote is in sync (no unpulled commits)
- Verify required tools are installed for detected ecosystem
- Output: colored checklist (pass/fail for each check)
- `--json` output for CI

### `scrat bump`
- Run detection to determine version strategy
- **CC mode**: shell out to `git cliff --bumped-version` or `cog bump --dry-run`
- **Interactive mode**: show recent commits via `git log --oneline`, prompt with inquire for patch/minor/major, compute next version
- **Explicit mode**: accept `--version v1.2.3` flag
- Display proposed version, confirm with user
- Update version in project files (`cargo set-version` for Rust)
- Generate/update CHANGELOG.md
- Output: the new version string

---

## Implementation Plan — Milestone 3: Ship

The full orchestrator command.

### `scrat ship`
- Runs phases 1-7 in sequence
- Each phase reports status (spinners via indicatif)
- Flags: `--yolo` (skip tests), `--no-publish`, `--no-push`, `--no-release`, `--dry-run`
- On failure at any phase: stop, report which phase failed, suggest fix

### Git stats collection (port from stats-since.sh)
- `git rev-list --count` for commit count
- `git diff --shortstat` parsing for files/insertions/deletions
- `git shortlog -sn` for top authors
- Output as struct, serialize to JSON for templates

### Dependency diff (port from deps-since.sh)
- Parse `git diff FROM..TO -- Cargo.lock`
- Extract `{name, from, to}` triples
- Rust implementation using string parsing (no awk needed)

### Release notes
- Delegate to `git-cliff` or `cog` for changelog body
- Wrap with template (quote, mood, codename, stats, deps)
- For now: use git-cliff's built-in templating via cliff.toml

### GitHub release
- Shell out to `gh release create/edit`
- Attach release notes file + postcard (if generated)

---

## Implementation Plan — Milestone 4: Postcard (Optional)

- Optional module, only activates if `ll-graphics` and `resvg` are on PATH
- `scrat ship` checks config `[release] postcard = true` and tool availability
- Generates SVG → PNG, attaches to GitHub release

---

## Verification

After each milestone:
1. `just check` (fmt + clippy + deny + test + doc-test)
2. Manual smoke test: `cargo run -p scrat -- <command>`
3. For milestone 3: end-to-end test on a real repo with `scrat ship --dry-run`

---

## Out of scope (for now)
- Node/Python ecosystem implementations (stubs only)
- Quote corpus management tooling
- AI-driven release metadata generation (keep as external prompt)
- Custom Tera template rendering (use git-cliff's built-in)
