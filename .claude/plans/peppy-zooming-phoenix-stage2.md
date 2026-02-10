# Built-in Deps Diff (M4 #2)

## Context

The pipeline context (M4 #1) defined `DepChange` and `PipelineContext.dependencies: Vec<DepChange>` but left them unpopulated. This item builds the engine that fills them: parse `git diff` of lockfiles between the previous tag and HEAD to extract dependency changes. This data feeds release notes templates and `filter:` hooks.

Reference impl: `ref/release-stuff/scripts/deps-since.sh` — parses unified diff of `Cargo.lock` with awk.

## Plan

### 1. Add `lockfile_path()` to `Ecosystem`

**File:** `crates/scrat-core/src/ecosystem.rs`

Add a method to `Ecosystem`:

```rust
/// Primary lockfile for this ecosystem, relative to project root.
pub const fn lockfile_path(self) -> &'static str {
    match self {
        Self::Rust => "Cargo.lock",
        Self::Node => "package-lock.json",
    }
}
```

Simple const fn, one test.

### 2. Add `diff_file()` to `git.rs`

**File:** `crates/scrat-core/src/git.rs`

New public function:

```rust
/// Get the unified diff for a specific file between a ref and HEAD.
///
/// Returns an empty string if the file doesn't exist in either ref
/// or has no changes.
pub fn diff_file(since: &str, path: &str) -> GitResult<String>
```

Runs: `git diff {since}..HEAD -- {path}`

Returns the raw diff output. Empty string on no changes (not an error).

### 3. Create `crates/scrat-core/src/deps.rs`

**New module** — lockfile diff parsing.

#### Public API:

```rust
/// Compute dependency changes between a ref and HEAD for the given ecosystem.
///
/// Returns an empty Vec if the lockfile doesn't exist or hasn't changed.
/// Deps diff failure is non-fatal — logs a warning and returns empty.
pub fn compute_deps(ecosystem: Ecosystem, previous_tag: &str) -> Vec<DepChange>
```

This is the entry point called from `ship.rs`. It:
1. Looks up `ecosystem.lockfile_path()`
2. Calls `git::diff_file(previous_tag, lockfile_path)`
3. Dispatches to the ecosystem-specific parser
4. Returns the parsed changes (empty vec on no diff or unsupported ecosystem)

Note: returns `Vec<DepChange>`, not `Result`. Deps diff failure is non-fatal — log a warning and return empty.

#### Cargo.lock parser:

```rust
/// Parse a unified diff of Cargo.lock into dependency changes.
fn parse_cargo_lock_diff(diff: &str) -> Vec<DepChange>
```

State machine that tracks per-package block:
- `current_name: Option<String>` — from any `name = "..."` line (context, removed, or added)
- `old_version: Option<String>` — from `-version = "..."` lines
- `new_version: Option<String>` — from `+version = "..."` lines

On `[[package]]` boundary (any prefix) or EOF: emit pending `DepChange` if we have name + at least one version, then reset.

Emitted changes:
- **Updated**: old and new version both present and differ → `DepChange { name, from: Some(old), to: Some(new) }`
- **Added**: only new_version → `DepChange { name, from: None, to: Some(new) }`
- **Removed**: only old_version → `DepChange { name, from: Some(old), to: None }`

Sort output by name for stable ordering.

#### Node (stub for now):

```rust
fn parse_package_lock_diff(_diff: &str) -> Vec<DepChange> {
    Vec::new()
}
```

### 4. Register module in `lib.rs`

**File:** `crates/scrat-core/src/lib.rs`

Add `pub mod deps;` and doc line.

### 5. Wire into `ship.rs` orchestrator

**File:** `crates/scrat-core/src/ship.rs`

Add deps computation **after building PipelineContext, before pre_ship hooks**. This is a silent data-gathering step — no new `ShipPhase` variant, no events. It populates `ctx.dependencies`.

In `ReadyShip::execute()`, after the `ctx.set_assets(...)` block and before deriving `hook_ctx`:

```rust
// ── Deps diff (silent, populates context) ──
if !self.options.no_deps {
    ctx.dependencies = deps::compute_deps(
        self.detection.ecosystem,
        &ctx.previous_tag,
    );
}
```

Add `use crate::deps;` to imports.

### 6. Add `no_deps` to `ShipOptions`

**File:** `crates/scrat-core/src/ship.rs`

Add to `ShipOptions`:
```rust
/// Skip dependency diff computation.
pub no_deps: bool,
```

### 7. Add `--no-deps` to CLI

**File:** `crates/scrat/src/commands/ship.rs`

Add to `ShipArgs`:
```rust
/// Skip dependency diff
#[arg(long)]
pub no_deps: bool,
```

Wire in `ShipOptions` construction:
```rust
no_deps: args.no_deps,
```

### 8. Tests

**In `deps.rs`:**
- `parse_cargo_lock_diff_update` — version bump: serde 1.0.0 → 1.0.1
- `parse_cargo_lock_diff_added` — new dependency (all `+` lines)
- `parse_cargo_lock_diff_removed` — removed dependency (all `-` lines)
- `parse_cargo_lock_diff_mixed` — multiple changes: update + add + remove
- `parse_cargo_lock_diff_empty` — empty diff → empty vec
- `parse_cargo_lock_diff_no_version_change` — name but same version → no change
- `parse_cargo_lock_diff_sorted` — output sorted by name
- `compute_deps_node_returns_empty` — Node ecosystem returns empty (stub)

**In `ecosystem.rs`:**
- `lockfile_paths` — verify Rust→Cargo.lock, Node→package-lock.json

**In `git.rs`:**
- `diff_file_nonexistent` — non-existent file → empty string

**In `ship.rs`:**
- Update `ship_options_default` test to include `no_deps: false`

## Files touched

| File | Change |
|------|--------|
| `crates/scrat-core/src/ecosystem.rs` | Add `lockfile_path()` + test |
| `crates/scrat-core/src/git.rs` | Add `diff_file()` + test |
| `crates/scrat-core/src/deps.rs` | **NEW** — `compute_deps()`, Cargo.lock parser, tests |
| `crates/scrat-core/src/lib.rs` | Add `pub mod deps` |
| `crates/scrat-core/src/ship.rs` | Wire deps, add `no_deps` to ShipOptions, update test |
| `crates/scrat/src/commands/ship.rs` | Add `--no-deps` flag + wire |

## Dependency graph

```
deps.rs     ──imports──>  ecosystem.rs (Ecosystem::lockfile_path)
deps.rs     ──imports──>  git.rs (diff_file)
deps.rs     ──imports──>  pipeline.rs (DepChange type)
ship.rs     ──imports──>  deps.rs (compute_deps)
```

No circular deps.

## Not included (deferred)

- package-lock.json parser (Node returns empty stub)
- composer.lock, yarn.lock, pnpm-lock.yaml parsers
- New `ShipPhase::Deps` variant (deps is a silent data step for now)
- pre_deps/post_deps hook pairs
- Release notes template consuming deps data (M4 #7)

## Verification

```bash
just check    # fmt + clippy + deny + test + doc-test
```
