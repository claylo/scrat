# Pipeline Context Model (M4 Item #1)

## Context

Every phase in `scrat ship` currently produces human-readable strings (`PhaseOutcome::Success { message }`), but no structured data accumulates across the pipeline. This blocks three upcoming M4 features:
- **`filter:` hooks** need a JSON-serializable context to pipe through stdin/stdout
- **Release notes template** needs structured stats, deps, metadata as Tera context
- **Built-in deps/stats steps** need somewhere to write their output

The pipeline context is the unifying data model. Define it now, wire it into the orchestrator, populate it from existing phase results. Future M4 items fill in `stats` and `dependencies`.

## Plan

### 1. Promote `serde_json` to runtime dep

**File:** `crates/scrat-core/Cargo.toml`

Move `serde_json = "1.0"` from `[dev-dependencies]` to `[dependencies]`. Required for `metadata: HashMap<String, serde_json::Value>` and JSON round-trip in `filter:` hooks.

### 2. Create `crates/scrat-core/src/pipeline.rs`

New module with these types:

- **`PipelineContext`** — the main accumulator struct. Fields:
  - Version: `version`, `previous_version`, `tag`, `previous_tag`, `date` (all `String`)
  - Repository: `owner`, `repo`, `repo_url: Option<String>`, `branch: Option<String>`
  - Project: `ecosystem: String`
  - Stats: `stats: Option<ReleaseStats>` (populated by M4 #3)
  - Deps: `dependencies: Vec<DepChange>` (populated by M4 #2)
  - Bump: `changelog_updated: bool`, `changelog_path: String`, `modified_files: Vec<String>`
  - Git: `commit_hash: Option<String>`
  - Release: `release_url: Option<String>`, `assets: Vec<String>`
  - Extensible: `metadata: HashMap<String, serde_json::Value>`
  - Control: `dry_run: bool`

- **`ReleaseStats`** — `commit_count`, `files_changed`, `insertions`, `deletions`, `contributors: Vec<Contributor>`
- **`Contributor`** — `name: String`, `count: usize`
- **`DepChange`** — `name: String`, `from: Option<String>`, `to: Option<String>`

All derive `Debug, Clone, Serialize, Deserialize`.

Key impl methods:
- `PipelineContext::new(...)` — constructs from ReadyShip data, fills version/repo/ecosystem, leaves stats/deps/git empty
- `hook_context(&self) -> HookContext` — derives the 6-field interpolation struct from context
- `record_bump(&mut self, ...)`, `record_git(&mut self, ...)`, `record_release(&mut self, ...)` — phase contribution setters
- `set_assets(&mut self, assets)` — load from config

Helper: `iso_date_today() -> String` — Hinnant algorithm for YYYY-MM-DD without chrono dep.

Version fields are `String` (not `semver::Version`) so JSON round-trip from `filter:` hooks doesn't require semver-aware parsing.

### 3. Register module in `lib.rs`

**File:** `crates/scrat-core/src/lib.rs`

Add `pub mod pipeline;` and doc line.

### 4. Wire into `ship.rs` orchestrator

**File:** `crates/scrat-core/src/ship.rs`

**a)** Add `context: PipelineContext` field to `ShipOutcome`. The CLI already serializes `ShipOutcome` to JSON (`serde_json::to_string_pretty(&outcome)`) — this just adds richer data to that output. CLI accesses `outcome.tag`, `.phases.len()`, `.hooks_run` — no breakage.

**b)** In `ReadyShip::execute()`: Replace `build_hook_context()` call with `PipelineContext::new()`, then derive `hook_ctx` via `ctx.hook_context()`. Remove the private `build_hook_context()` function.

**c)** Refactor `run_git_phase()` — currently returns `PhaseOutcome` (string). Change to return a small private `GitPhaseResult { hash, tag, pushed, branch }` struct. Build `PhaseOutcome` in the caller. This lets us call `ctx.record_git(result.hash, result.branch)`.

**d)** Refactor `run_release_phase()` — similar: return `ReleasePhaseResult { url: Option<String> }`. Build `PhaseOutcome` in the caller. Lets us call `ctx.record_release(result.url)`.

**e)** After bump phase (non-dry-run path): call `ctx.record_bump(result.changelog_updated, result.modified_files)`.

**f)** After config load: call `ctx.set_assets(...)` from `config.release.assets`.

**g)** Include `context: ctx` in final `ShipOutcome` construction.

### 5. Update `hooks.rs` doc comment

**File:** `crates/scrat-core/src/hooks.rs`

Add doc note to `HookContext` indicating it's derived from `PipelineContext::hook_context()`. No structural changes.

### 6. Tests

**In `pipeline.rs`:**
- `new_sets_version_fields` — verify constructor populates version/tag/previous_tag
- `new_computes_date` — verify YYYY-MM-DD format
- `new_starts_with_empty_phase_results` — stats=None, deps=empty, etc.
- `hook_context_derives_correctly` — all 6 fields match
- `record_bump_updates_fields`, `record_git_updates_fields`, `record_release_updates_url`
- `json_round_trip` — serialize → deserialize preserves all fields including metadata
- `json_round_trip_with_stats` — verify nested struct round-trips
- `dep_change_serializes` — verify shape
- `iso_date_today_format` — verify 10-char YYYY-MM-DD
- `set_assets`

**In `ship.rs`:**
- Update `build_hook_context_structure` test → test `PipelineContext::new()` + `.hook_context()` instead
- Update `ship_outcome_serializes` test → include `context` field

### Dependency graph

```
pipeline.rs  ──imports──>  hooks.rs (HookContext type)
ship.rs      ──imports──>  pipeline.rs (PipelineContext)
ship.rs      ──imports──>  hooks.rs (run_hooks, interpolate_command)
```

No circular deps. `hooks` depends on neither.

## Files touched

| File | Change |
|------|--------|
| `crates/scrat-core/Cargo.toml` | `serde_json` dev→runtime |
| `crates/scrat-core/src/pipeline.rs` | **NEW** — types, constructor, methods, tests |
| `crates/scrat-core/src/lib.rs` | Add `pub mod pipeline` |
| `crates/scrat-core/src/ship.rs` | Wire context, refactor git/release phase returns, update tests |
| `crates/scrat-core/src/hooks.rs` | Doc comment only |

## Not included (deferred)

- Populating `stats` — M4 #3
- Populating `dependencies` — M4 #2
- `filter:` hook prefix — M4 #4
- Template rendering — M4 #7
- `--no-stats` / `--no-deps` flags — M4 #6

## Verification

```bash
just check    # fmt + clippy + deny + test + doc-test
```
