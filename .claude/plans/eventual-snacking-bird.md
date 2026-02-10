# M3 Plan: Ship Orchestrator with Full Hook Coverage

## Context

M2 delivered the building blocks — preflight checks, version computation (3 strategies), bump plan/execute, git status operations, and hooks config. M3 wires them into a single `scrat ship` pipeline that runs the full release workflow, with hooks firing at every major phase boundary.

The current `HooksConfig` only covers 3 of the 7 ship phases (bump, publish, tag). This plan expands it to cover all major phases and builds the hook executor + orchestrator.

---

## Phase Map with Hooks

```
pre_ship ─────────────────────────────────────────────────────┐
│                                                             │
│  1. PREFLIGHT  (no hooks — validation only)                 │
│  2. VERSION    (no hooks — internal decision)               │
│                                                             │
│  pre_test ──── 3. TEST ──── post_test                       │
│  pre_bump ──── 4. BUMP ──── post_bump                       │
│  pre_publish ─ 5. PUBLISH ─ post_publish   (--no-publish)   │
│  pre_tag ───── 6. GIT ───── post_tag       (--no-push)      │
│  pre_release ─ 7. RELEASE ─ post_release   (--no-release)   │
│                                                             │
post_ship ────────────────────────────────────────────────────┘
```

7 hook pairs = 14 hook points. `pre_ship`/`post_ship` bracket the entire pipeline.

---

## Implementation Steps

### Step 1: Expand `HooksConfig` in `config.rs`

Add 3 new hook pairs to `HooksConfig`:

```rust
pub struct HooksConfig {
    // NEW — bracket the entire ship workflow
    pub pre_ship: Option<Vec<String>>,
    pub post_ship: Option<Vec<String>>,
    // NEW — before/after test phase
    pub pre_test: Option<Vec<String>>,
    pub post_test: Option<Vec<String>>,
    // EXISTING
    pub pre_bump: Option<Vec<String>>,
    pub post_bump: Option<Vec<String>>,
    pub pre_publish: Option<Vec<String>>,
    pub post_publish: Option<Vec<String>>,
    pub pre_tag: Option<Vec<String>>,
    pub post_tag: Option<Vec<String>>,
    // NEW — before/after GitHub release
    pub pre_release: Option<Vec<String>>,
    pub post_release: Option<Vec<String>>,
}
```

Update `config/scrat.toml.example` to document all hook phases.

**Files**: `crates/scrat-core/src/config.rs`, `config/scrat.toml.example`

---

### Step 2: Hook Executor Module (`hooks.rs`)

New module: `crates/scrat-core/src/hooks.rs`

**Public API**:

```rust
/// Variables available for interpolation in hook commands.
pub struct HookContext {
    pub version: String,
    pub prev_version: String,
    pub tag: String,
    pub changelog_path: String,
    pub owner: String,
    pub repo: String,
}

/// Result of running a single hook command.
pub struct HookResult {
    pub command: String,
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
    pub duration: Duration,
}

/// Run a list of hook commands with variable interpolation.
///
/// Commands run in parallel by default. Prefix a command with
/// `sync:` to create a barrier — all prior commands must finish,
/// the sync command runs alone, then subsequent commands resume
/// in parallel.
pub fn run_hooks(
    commands: &[String],
    context: &HookContext,
    project_root: &Utf8Path,
) -> HookRunResult<Vec<HookResult>>
```

**Execution model**:

1. Parse commands into batches split at `sync:` boundaries
2. Each batch runs in parallel via `Command::spawn()` + collect handles
3. Sync commands run alone between batches
4. All commands get `{var}` interpolation before execution
5. If any command in a batch fails, remaining batches are skipped and error is returned

**Internal helpers**:

```rust
fn interpolate(command: &str, context: &HookContext) -> String
fn run_batch(commands: &[String], context: &HookContext, root: &Utf8Path) -> HookRunResult<Vec<HookResult>>
```

**Error type**:

```rust
#[derive(Error, Debug)]
pub enum HookError {
    #[error("hook command failed: {command}")]
    CommandFailed {
        command: String,
        exit_code: Option<i32>,
        stderr: String,
    },
    #[error("failed to execute hook: {0}")]
    Exec(#[from] std::io::Error),
}
```

**Tests**: Unit tests for interpolation, batch splitting at `sync:` boundaries, parallel execution order, failure propagation.

**File**: `crates/scrat-core/src/hooks.rs`

---

### Step 3: Extend Git Module

Add commit, tag, and push operations to `crates/scrat-core/src/git.rs`.

**New public functions**:

```rust
/// Stage files and create a commit.
pub fn commit(files: &[&str], message: &str) -> GitResult<String>
// Returns the commit hash

/// Create an annotated tag.
pub fn create_tag(name: &str, message: &str) -> GitResult<()>

/// Push branch and tags to remote.
pub fn push(remote: &str, branch: &str, push_tags: bool) -> GitResult<()>

/// Get stats since a ref: commit count, files changed, insertions, deletions.
pub fn stats_since(since: &str) -> GitResult<GitStats>

/// Get top contributors since a ref.
pub fn contributors_since(since: &str, limit: usize) -> GitResult<Vec<(String, usize)>>
```

**New types**:

```rust
#[derive(Debug, Clone, Serialize)]
pub struct GitStats {
    pub commit_count: usize,
    pub files_changed: usize,
    pub insertions: usize,
    pub deletions: usize,
}
```

All follow the existing pattern: private `git()` helper → `Command::new("git")` → check status → parse output.

**File**: `crates/scrat-core/src/git.rs`

---

### Step 4: Ship Orchestrator (`ship.rs`)

New module: `crates/scrat-core/src/ship.rs`

This is the core of M3. Follows the established plan/execute pattern.

**Public API**:

```rust
/// Options controlling which phases run.
pub struct ShipOptions {
    pub explicit_version: Option<String>,
    pub no_changelog: bool,
    pub no_publish: bool,
    pub no_push: bool,
    pub no_release: bool,
    pub dry_run: bool,
    pub skip_tests: bool,
}

/// Result of planning a ship.
pub enum ShipPlan {
    Ready(ReadyShip),
    NeedsInteraction(InteractiveShip),
}

/// A ship ready to execute (version already determined).
pub struct ReadyShip { .. }

/// A ship that needs user input for version selection.
pub struct InteractiveShip { .. }

/// Result of a single phase.
#[derive(Debug, Serialize)]
pub enum PhaseOutcome {
    Success { message: String },
    Skipped { reason: String },
}

/// Outcome of the full ship workflow.
#[derive(Debug, Serialize)]
pub struct ShipOutcome {
    pub version: Version,
    pub previous_version: Version,
    pub tag: String,
    pub phases: Vec<(ShipPhase, PhaseOutcome)>,
    pub hooks_run: usize,
    pub dry_run: bool,
}

/// Phases of the ship workflow (for progress reporting).
#[derive(Debug, Clone, Copy, Serialize)]
pub enum ShipPhase {
    Preflight,
    Version,
    Test,
    Bump,
    Publish,
    Git,
    Release,
}

/// Plan the ship workflow. Runs preflight + version resolution.
pub fn plan_ship(
    project_root: &Utf8Path,
    config: &Config,
    options: &ShipOptions,
) -> ShipResult<ShipPlan>

/// Resolve an interactive ship with the user's chosen version.
pub fn resolve_ship_interaction(
    plan: InteractiveShip,
    chosen_version: Version,
) -> ReadyShip

impl ReadyShip {
    /// Execute the ship workflow. Calls `on_event` at phase boundaries
    /// so the CLI can update progress display.
    pub fn execute(
        self,
        project_root: &Utf8Path,
        on_event: impl FnMut(ShipEvent),
    ) -> ShipResult<ShipOutcome>
}
```

**Events for progress reporting**:

```rust
pub enum ShipEvent {
    PhaseStarted(ShipPhase),
    PhaseCompleted(ShipPhase, PhaseOutcome),
    HooksStarted { phase: ShipPhase, count: usize },
    HooksCompleted { phase: ShipPhase, count: usize },
}
```

**Execute flow** (inside `ReadyShip::execute`):

```
emit PhaseStarted(Preflight)
  → preflight::run_preflight()            [reuse existing]
emit PhaseCompleted(Preflight)

emit PhaseStarted(Version)
  → (already resolved in plan phase)
emit PhaseCompleted(Version)

run_hooks(pre_ship)                       [bracket open]

run_hooks(pre_test)
emit PhaseStarted(Test)
  → run test command (from DetectedTools or CommandsConfig override)
emit PhaseCompleted(Test)
run_hooks(post_test)

run_hooks(pre_bump)
emit PhaseStarted(Bump)
  → ReadyBump::execute()                  [reuse existing]
emit PhaseCompleted(Bump)
run_hooks(post_bump)

IF !no_publish:
  run_hooks(pre_publish)
  emit PhaseStarted(Publish)
    → run publish command
  emit PhaseCompleted(Publish)
  run_hooks(post_publish)

IF !no_push:
  run_hooks(pre_tag)
  emit PhaseStarted(Git)
    → git::commit() + git::create_tag() + git::push()
  emit PhaseCompleted(Git)
  run_hooks(post_tag)

IF !no_release:
  run_hooks(pre_release)
  emit PhaseStarted(Release)
    → create GitHub release via `gh release create`
    → attach configured assets
  emit PhaseCompleted(Release)
  run_hooks(post_release)

run_hooks(post_ship)                      [bracket close]
```

**Hook context assembly**: Built from the resolved version, previous version, git remote info, and changelog path. The `HookContext` is constructed once after version resolution and passed to all `run_hooks` calls.

**Dry-run mode**: When `dry_run` is true, each phase logs what _would_ happen and emits `PhaseCompleted` with a description, but performs no mutations (no file writes, no git ops, no publishes). Hooks do NOT run in dry-run mode.

**Error handling**: Any phase failure aborts the pipeline. The error includes which phase failed and what was already completed, so the user knows the state. No automatic rollback — the user handles recovery.

**File**: `crates/scrat-core/src/ship.rs`

---

### Step 5: CLI `ship` Command

New file: `crates/scrat/src/commands/ship.rs`

**Args**:

```rust
pub struct ShipArgs {
    /// Set version explicitly (e.g., "1.2.3")
    #[arg(long)]
    pub version: Option<String>,

    /// Skip changelog generation
    #[arg(long)]
    pub no_changelog: bool,

    /// Skip publishing to registry
    #[arg(long)]
    pub no_publish: bool,

    /// Skip git push (still commits and tags locally)
    #[arg(long)]
    pub no_push: bool,

    /// Skip GitHub release creation
    #[arg(long)]
    pub no_release: bool,

    /// Skip running tests
    #[arg(long)]
    pub skip_tests: bool,

    /// Preview what would happen without making changes
    #[arg(long)]
    pub dry_run: bool,
}
```

**Implementation** (thin CLI pattern):

1. Build `ShipOptions` from args
2. Call `ship::plan_ship(cwd, config, &options)`
3. If `NeedsInteraction` → prompt with `inquire::Select` (reuse pattern from bump command)
4. Call `ready.execute(cwd, on_event)` with a closure that drives `indicatif` spinners
5. Display final summary (version, phases completed, hooks run)
6. JSON mode: serialize `ShipOutcome`

**Progress display**: Use `indicatif::MultiProgress` with one spinner per active phase. The `on_event` callback creates/finishes spinners as phases start/complete.

**Files**: `crates/scrat/src/commands/ship.rs`, `crates/scrat/src/commands/mod.rs`, `crates/scrat/src/lib.rs`, `crates/scrat/src/main.rs`

---

### Step 6: Wire Up and Update Exports

- Add `pub mod hooks;` and `pub mod ship;` to `crates/scrat-core/src/lib.rs`
- Add `Ship` variant to `Commands` enum
- Add match arm in `main.rs`
- Export key types from `lib.rs` if needed

---

### Step 7: Tests

| Area | Test type | What to verify |
|------|-----------|----------------|
| Hook interpolation | Unit | All 6 variables replaced, missing vars left as-is |
| Hook batch splitting | Unit | `sync:` prefix creates barriers correctly |
| Hook parallel execution | Unit | Non-sync commands run concurrently |
| Hook failure propagation | Unit | Batch failure skips remaining batches |
| HooksConfig deser | Unit | TOML with all 14 hook fields parses |
| Git commit/tag/push | Unit | Commands invoked with correct args |
| Git stats | Unit | Parse `--stat` and `shortlog` output |
| Ship plan | Unit | Returns Ready or NeedsInteraction correctly |
| Ship execute | Integration | Full pipeline with mock commands |
| Ship dry-run | Integration | No mutations occur |
| Ship skip flags | Integration | Phases skipped correctly |
| CLI ship | Integration (`assert_cmd`) | Basic usage, flags, JSON output |

---

## Implementation Order

```
1. HooksConfig expansion (config.rs)     ← small, no deps
2. Hook executor (hooks.rs)              ← foundation, no deps
3. Git extensions (git.rs)               ← no deps on 1-2
4. Ship orchestrator (ship.rs)           ← depends on 1, 2, 3
5. CLI ship command (commands/ship.rs)   ← depends on 4
6. Wire up + integration tests           ← depends on 5
7. Update example config                 ← last
```

Steps 1, 2, and 3 are independent and can be implemented in parallel.

---

## Verification

1. `just check` passes (fmt + clippy + deny + nextest + doc-test)
2. All new tests pass via `just test`
3. `scrat ship --dry-run` in the scrat repo itself shows the full phase plan
4. `scrat ship --help` shows all flags
5. Hook interpolation works with test commands
6. `--json` output is valid JSON with all phase outcomes
