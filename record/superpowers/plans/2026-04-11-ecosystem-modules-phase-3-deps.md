# Ecosystem Modules Refactor — Phase 3: Extract `deps/`

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extract the seven per-ecosystem lockfile diff parsers from `crates/scrat-core/src/deps.rs` into sibling files under a new `crates/scrat-core/src/deps/` module directory, and introduce the first trait in the refactor arc — `LockfileDiffParser` — as a minimal `&self`-taking interface on zero-sized unit structs. Preserve `compute_deps`'s public signature unchanged. No behavior change.

**Architecture:** Phase 3 of the four-phase ecosystem-modules refactor. Unlike Phase 2 (which harmonized five divergent `bump_*` return types before extracting), Phase 3 starts from parsers that are *already* uniform in shape — every current `parse_*_diff(diff: &str) -> Vec<DepChange>` is a pure function with identical signature. That makes the extraction nearly free mechanically. The real work is introducing the `LockfileDiffParser` trait as a minimal `&self` interface on zero-sized unit structs (`RustLockfileParser`, `NodeLockfileParser`, …), rewriting `compute_deps`'s dispatch to call trait methods, and splitting 59 tests across 7 sibling files plus 4 shared-helper tests that stay in `deps/mod.rs`. The `&self` shape is load-bearing for Phase 4, where `RustDriver` will carry `bump_cmd` state without needing a signature change.

**Tech Stack:** Rust (scrat-core library crate). No new dependencies.

---

## The full arc (context, not in scope for this plan)

| Phase | Goal | Output | Status |
|-------|------|--------|--------|
| **1** | Finish `detect/` split | `detect/{rust,node,go,php,python,ruby,swift}.rs` + normalized `build_detection_for` dispatch | **COMPLETE** — merged 2026-04-10 as PR #37 (squash `0765242`) |
| **2** | Extract `bump/` with harmonized `BumpResult<Vec<String>>` return type | `bump/{rust,node,php,python,ruby}.rs` per ecosystem; uniform `ReadyBump::execute` dispatch | **COMPLETE** — merged 2026-04-10 as PR #38 (squash `bbdd2ab`) |
| **3 (this plan)** | Extract `deps/` with `LockfileDiffParser` trait | `deps/{rust,node,go,php,python,ruby,swift}.rs` + `LockfileDiffParser` trait + per-ecosystem unit structs | **THIS PLAN — first trait introduction** |
| **4** | Unify into `ecosystem/<name>.rs` with single `EcosystemDriver` trait | Single file per ecosystem implementing the unified trait; `bump/`, `deps/`, `detect/` directories collapsed into `ecosystem/` | Planned |

Phase 4 will absorb `LockfileDiffParser` into a larger `EcosystemDriver` trait that also covers bump, detect, and registry auth. The destination sketch — for posterity, NOT this plan's concern:

```rust
// Eventual destination in Phase 4. NOT in scope for this plan.
pub trait EcosystemDriver {
    fn marker_file(&self) -> Option<&'static str>;
    fn lockfile_path(&self) -> Option<&'static str>;
    fn detect(&self, root: &Utf8Path, s: VersionStrategy) -> ProjectDetection;
    fn bump_version_files(&self, root: &Utf8Path, v: &Version) -> BumpResult<BumpOutcome>;
    fn parse_lockfile_diff(&self, diff: &str) -> Vec<DepChange>;
    fn registry_auth(&self) -> Option<RegistryAuthCheck>;
    fn bump_config_toml(&self) -> &'static str;
}
```

The `LockfileDiffParser::parse_diff` method introduced in this plan uses the same `&self, diff: &str) -> Vec<DepChange>` signature as the eventual `EcosystemDriver::parse_lockfile_diff`, so Phase 4's absorption is a mechanical method rename (`parse_diff` → `parse_lockfile_diff`) without any type or parameter changes. The shorter `parse_diff` name fits Phase 3 because the trait name `LockfileDiffParser` already carries the "lockfile diff" context; Phase 4's unified trait needs the longer method name to disambiguate from hypothetical `parse_manifest_diff` or similar.

---

## Out of scope for Phase 3

- **Any behavior change.** Every parser's state machine, sort ordering, and edge-case handling moves verbatim. Hash-based collect-and-merge (Go, Ruby) stays hash-based. TOML state machines (Rust, Python) stay state machines. JSON state machines (PHP, Swift) stay JSON state machines.
- **Fixing `parse_package_lock_diff`'s "stub" reputation.** The handoff/auto-memory describes Node's parser as "currently stubbed" — reading the actual code shows it's a full parser against lockfile v2/v3 top-level entries. The memory note is stale. This plan moves Node's parser as a real parser. No expansion, no rewrite.
- **Normalizing `&Option<String>` parameters in `emit_change`.** Phase 2's lesson: preserve incidental asymmetries and clippy nits during mechanical refactors. If clippy warns on `&Option<String>` after the move, add `#[allow(clippy::ref_option)]` at the helper site and continue — do NOT rewrite the signature. This helper is scheduled for Phase 4's unification anyway.
- **Introducing a shared `parse_toml_package_diff` helper.** Clay's explicit decision: Python delegates literally to `RustLockfileParser.parse_diff(diff)`. The uv/cargo format match is incidental, not a shared primitive. Do NOT extract a shared TOML-package-diff helper in this phase. Phase 4 can revisit if bump + detect + deps all need one.
- **Touching `bump.rs`, `preflight.rs`, `detect/`.** Those are other phases' territory. `deps.rs` only.
- **Renaming parser functions.** Inside each sibling file, the extracted logic can become an inline `fn parse_diff(...)` impl body — but if a helper functions needs to exist (e.g., Node's `extract_top_level_node_modules_name` moves as a private fn in `deps/node.rs`), keep the existing name.
- **Changing `compute_deps`'s public signature.** `pub fn compute_deps(ecosystem: Ecosystem, previous_tag: &str) -> Vec<DepChange>` stays exactly as-is. This function is called from `pipeline.rs` and rewriting its signature would ripple unnecessarily.
- **Touching `crates/scrat-core/src/lib.rs`.** The `pub mod deps;` declaration at lib.rs resolves to either `deps.rs` OR `deps/mod.rs` automatically. Zero lib.rs changes needed.
- **Expanding test coverage.** 59 existing tests are the refactor safety net. Don't add, don't split, don't reorganize — move verbatim.

---

## File structure after Phase 3

```
crates/scrat-core/src/
├── deps/
│   ├── mod.rs        # compute_deps() (public API) +
│   │                 # LockfileDiffParser trait definition +
│   │                 # pub(super) emit_change +
│   │                 # pub(super) extract_toml_string_value +
│   │                 # pub(super) extract_json_string_value +
│   │                 # 4 shared-helper tests (extract_toml_* + extract_json_*)
│   ├── rust.rs       # RustLockfileParser + impl + 7 parse_cargo_lock_diff_* tests (~140 LOC)
│   ├── node.rs       # NodeLockfileParser + impl + node-private helpers
│   │                 #   (extract_top_level_node_modules_name, extract_json_version) +
│   │                 #   9 parse_package_lock_diff_* tests +
│   │                 #   4 extract_top_level_node_modules_name_* tests +
│   │                 #   2 extract_json_version_* tests (~240 LOC)
│   ├── go.rs         # GoLockfileParser + impl + 9 parse_go_mod_diff_* tests (~200 LOC)
│   ├── php.rs        # PhpLockfileParser + impl + 7 parse_composer_lock_diff_* tests (~150 LOC)
│   ├── python.rs     # PythonLockfileParser — DELEGATES to RustLockfileParser +
│   │                 #   module-level doc comment explaining the incidental format match +
│   │                 #   3 parse_uv_lock_diff_* tests (~70 LOC)
│   ├── ruby.rs       # RubyLockfileParser + impl + 7 parse_gemfile_lock_diff_* tests (~170 LOC)
│   └── swift.rs      # SwiftLockfileParser + impl + 7 parse_package_resolved_diff_* tests (~140 LOC)
└── ...
```

After extraction, `deps/mod.rs` should be roughly 250–280 LOC (down from 1343). That's `compute_deps` + trait definition + 3 shared helpers + module-level tests + module declarations. Target the 4 shared helper tests (`extract_toml_string_value_basic`, `extract_toml_string_value_no_match`, `extract_json_string_value_basic`, `extract_json_string_value_no_match`) stay in `deps/mod.rs::tests` — they exercise the shared helpers directly.

---

## Test allocation (59 total tests)

| Sibling file | Test functions | Count |
|---|---|---|
| `deps/rust.rs::tests` | `parse_cargo_lock_diff_*` (update, added, removed, mixed, empty, no_version_change, sorted) | 7 |
| `deps/node.rs::tests` | `parse_package_lock_diff_*` (9) + `extract_top_level_node_modules_name_*` (4) + `extract_json_version_*` (2) | 15 |
| `deps/go.rs::tests` | `parse_go_mod_diff_*` (update, added, removed, indirect_stripped, mixed, skips_headers, major_version_path, empty, pseudo_version) | 9 |
| `deps/php.rs::tests` | `parse_composer_lock_diff_*` (update, added, removed, mixed, ignores_reference, empty, stability_suffix) | 7 |
| `deps/python.rs::tests` | `parse_uv_lock_diff_*` (update, added, skips_header) | 3 |
| `deps/ruby.rs::tests` | `parse_gemfile_lock_diff_*` (update, added, removed, ignores_subdeps, mixed, empty, prerelease) | 7 |
| `deps/swift.rs::tests` | `parse_package_resolved_diff_*` (update, added, removed, ignores_revision, ignores_file_version, mixed, empty) | 7 |
| `deps/mod.rs::tests` | `extract_toml_string_value_basic`, `extract_toml_string_value_no_match`, `extract_json_string_value_basic`, `extract_json_string_value_no_match` | 4 |
| **Total** | | **59** |

If the baseline at Task 1 Step 4 reports a number other than 59, update this table and all references.

---

## The `LockfileDiffParser` trait

Define in `deps/mod.rs`:

```rust
/// Parses a unified diff of an ecosystem-specific lockfile into
/// [`DepChange`] entries.
///
/// Implemented by zero-sized unit structs per ecosystem
/// (`RustLockfileParser`, `NodeLockfileParser`, …). The `&self` receiver
/// carries no state today, but preserves Phase 4's flexibility to attach
/// per-ecosystem state (e.g., `RustDriver { bump_cmd }`) without changing
/// the method signature.
///
/// Parsers are infallible by convention: malformed input returns an empty
/// `Vec` rather than an error, matching the existing "deps diff failure is
/// non-fatal" contract established by [`compute_deps`].
pub trait LockfileDiffParser {
    /// Parse a unified diff into dependency changes.
    ///
    /// Returns an empty `Vec` if the diff contains no recognizable
    /// dependency changes. Implementations must sort the result by
    /// `DepChange.name` for deterministic output.
    fn parse_diff(&self, diff: &str) -> Vec<DepChange>;
}
```

Seven unit struct implementations, one per ecosystem:

```rust
pub struct RustLockfileParser;
pub struct NodeLockfileParser;
pub struct GoLockfileParser;
pub struct PhpLockfileParser;
pub struct PythonLockfileParser;
pub struct RubyLockfileParser;
pub struct SwiftLockfileParser;
```

`Ecosystem::Generic` does NOT get a parser struct. `compute_deps` short-circuits Generic in the match arm before dispatch.

---

## Python delegation (load-bearing decision — documented in code)

`deps/python.rs` contains a module-level doc comment explaining that delegation to `RustLockfileParser` is **intentional and incidental**, not a shared abstraction commitment:

```rust
//! Python lockfile diff parser for `uv.lock`.
//!
//! `uv.lock` currently uses the same TOML `[[package]]` format as
//! `Cargo.lock`, so this parser literally delegates to
//! [`super::rust::RustLockfileParser`]. This is NOT a commitment to a
//! shared "TOML package diff" abstraction — it's an incidental format
//! match. If uv diverges from Cargo's lockfile format in a future
//! release, this module grows its own state machine and stops
//! delegating. Do not extract a shared TOML-package-diff helper on
//! the assumption that Python and Rust will always share an
//! implementation.
```

The impl body is one line:

```rust
impl LockfileDiffParser for PythonLockfileParser {
    fn parse_diff(&self, diff: &str) -> Vec<DepChange> {
        super::rust::RustLockfileParser.parse_diff(diff)
    }
}
```

---

## Dispatch in `compute_deps`

Before (current state):

```rust
let changes = match ecosystem {
    Ecosystem::Rust => parse_cargo_lock_diff(&diff),
    Ecosystem::Node => parse_package_lock_diff(&diff),
    Ecosystem::Go => parse_go_mod_diff(&diff),
    Ecosystem::Php => parse_composer_lock_diff(&diff),
    Ecosystem::Python => parse_uv_lock_diff(&diff),
    Ecosystem::Ruby => parse_gemfile_lock_diff(&diff),
    Ecosystem::Swift => parse_package_resolved_diff(&diff),
    Ecosystem::Generic => Vec::new(),
};
```

After (Task 3 end state):

```rust
let changes = match ecosystem {
    Ecosystem::Rust    => rust::RustLockfileParser.parse_diff(&diff),
    Ecosystem::Node    => node::NodeLockfileParser.parse_diff(&diff),
    Ecosystem::Go      => go::GoLockfileParser.parse_diff(&diff),
    Ecosystem::Php     => php::PhpLockfileParser.parse_diff(&diff),
    Ecosystem::Python  => python::PythonLockfileParser.parse_diff(&diff),
    Ecosystem::Ruby    => ruby::RubyLockfileParser.parse_diff(&diff),
    Ecosystem::Swift   => swift::SwiftLockfileParser.parse_diff(&diff),
    Ecosystem::Generic => Vec::new(),
};
```

Alignment matches Phase 2's `ReadyBump::execute` dispatch style for consistency across the crate.

---

## Conventions used in this plan

- **Commits via `commit.txt` — APPEND, do not overwrite:** Each task's commit step APPENDS a sub-bullet section to `commit.txt` at the repo root. It does **not** overwrite the existing file. Clay runs `gtxt` (alias: `git commit -F commit.txt && rm commit.txt`) periodically — sometimes after every task, sometimes after batching several. When `gtxt` runs, the entire accumulated `commit.txt` becomes one bundled commit, and the file is deleted. The next task that produces a commit must re-create the skeleton. The worker does **not** run `git commit` directly.
- **Bundled-commit format:** `commit.txt` follows the structure used in `f706dc9`, `974deb4`, and `bbdd2ab` — one top-level subject line, a brief opening body paragraph, then multiple `* type(scope): subject` sub-bullet sections (each with its own body paragraph at column 0, NOT indented under the `*`). Each task contributes one sub-bullet section.
- **Task 2 intro paragraph is REAL, not a placeholder.** Phase 2's Task 2 commit.txt included a literal `[body to be appended as each task completes]` placeholder that leaked into the merged squash commit `bbdd2ab` as its second line. For Phase 3, the Task 2 opening paragraph is a real, complete sentence (see Task 2 Step 7 below). DO NOT include placeholder text anywhere in `commit.txt`.
- **Test cadence:** Full workspace test runs are slow on this machine. Each task runs `cargo check -p scrat-core` and `cargo clippy -p scrat-core --all-targets -- -D warnings` (both fast). Running `cargo nextest run -p scrat-core deps::` is fine — the deps test module has 59 tests and runs in well under a second. Running the full workspace suite requires asking Clay first. Use `dangerouslyDisableSandbox: true` for every cargo/nextest/just invocation.
- **Narrow test scope per task:** At the end of each extraction task (Tasks 4–10), run `cargo nextest run -p scrat-core deps::<ecosystem>::` to exercise just the extracted module's tests, then `cargo nextest run -p scrat-core deps::` to confirm no cross-module regression. Both should pass.
- **Branch:** `refactor/ecosystem-modules-phase-3`. One branch, several bundled commits via `gtxt`, one PR at the end.
- **Cargo sandbox flag:** Every `cargo`, `cargo nextest`, and `just` invocation in the Bash tool must use `dangerouslyDisableSandbox: true` because sccache fails under sandbox mode. This is not optional.
- **Module-level `use` imports only.** Every sibling file (`deps/rust.rs`, `deps/node.rs`, etc.) places all `use` statements at module level, never inside function bodies. The only exception is inline `use` inside `#[cfg(test)] mod tests` functions, which is fine.
- **Preserve existing imports exactly.** If the current `parse_gemfile_lock_diff` function uses `use std::collections::HashMap;` inline at the function level, lift it to module level when extracting. Do not leave inline `use` statements in the extracted sibling file.
- **Review optimization (validated twice):** Spec review is dispatched every task. Code-quality review runs ONLY on (a) the first repetition — Task 4, the Rust extraction, as the template validator; (b) the biggest extraction — Task 5, the Node extraction, which carries 15 tests and two private helpers; and (c) the final PR — Task 11. Middle tasks (6, 7, 8, 9, 10) get spec review only. This saves ~5 code-quality dispatches without losing coverage. Pattern validated in Phases 1 and 2, both merged clean on first try.

---

### Task 1: Branch setup and baseline verification

**Files:** none (git + verification only)

- [ ] **Step 1: Create feature branch**

Run:
```bash
git checkout -b refactor/ecosystem-modules-phase-3
```

Expected: `Switched to a new branch 'refactor/ecosystem-modules-phase-3'`.

- [ ] **Step 2: Verify clean build on the baseline**

Run (with `dangerouslyDisableSandbox: true`):
```bash
cargo check -p scrat-core
```
Expected: clean build, no errors, no warnings, exit code 0.

- [ ] **Step 3: Verify clippy is clean on the baseline**

Run (with `dangerouslyDisableSandbox: true`):
```bash
cargo clippy -p scrat-core --all-targets -- -D warnings
```
Expected: no warnings, exit code 0.

- [ ] **Step 4: Record baseline deps-module test count**

Run (with `dangerouslyDisableSandbox: true`):
```bash
cargo nextest run -p scrat-core deps::
```
Expected: all tests pass. Record the exact number — you will compare against this at Task 11 Step 3. The plan assumes **59** tests. If the baseline differs, update every task that references the count.

- [ ] **Step 5: Confirm starting line count**

Run:
```bash
wc -l crates/scrat-core/src/deps.rs
```
Expected:
```
    1343 crates/scrat-core/src/deps.rs
```
Record this. Task 11 will verify `deps/mod.rs` has shrunk to the 250–300 LOC range.

---

### Task 2: Convert `deps.rs` to `deps/mod.rs` (file move only, no code changes)

**Files:**
- Move: `crates/scrat-core/src/deps.rs` → `crates/scrat-core/src/deps/mod.rs`

This task is purely a filesystem restructure. No content changes, no behavior changes, no dispatch changes, no trait introduction. The `pub mod deps;` declaration in `crates/scrat-core/src/lib.rs` resolves to either `deps.rs` OR `deps/mod.rs` — Rust's module system picks up whichever exists. After this task, `deps.rs` no longer exists and `deps/mod.rs` contains the verbatim 1343 lines.

This task is separated from Task 3 (trait introduction + dispatch conversion) so that the file move commits cleanly as a rename in git, preserving blame history. A combined "move + edit" commit would register as a delete + add with rename heuristics and complicate `git blame` output.

- [ ] **Step 1: Create the `deps/` directory**

Run:
```bash
mkdir crates/scrat-core/src/deps
```
Expected: directory created silently.

- [ ] **Step 2: Move `deps.rs` into the new directory as `mod.rs`**

Run:
```bash
git mv crates/scrat-core/src/deps.rs crates/scrat-core/src/deps/mod.rs
```
Expected: `git mv` runs silently. `git status` should now show:
```
renamed:    crates/scrat-core/src/deps.rs -> crates/scrat-core/src/deps/mod.rs
```

- [ ] **Step 3: Verify compilation**

Run (with `dangerouslyDisableSandbox: true`):
```bash
cargo check -p scrat-core
```
Expected: clean build. If this fails, the file move broke something — investigate before continuing.

- [ ] **Step 4: Verify clippy is clean**

Run (with `dangerouslyDisableSandbox: true`):
```bash
cargo clippy -p scrat-core --all-targets -- -D warnings
```
Expected: no warnings.

- [ ] **Step 5: Verify test suite still passes**

Run (with `dangerouslyDisableSandbox: true`):
```bash
cargo nextest run -p scrat-core deps::
```
Expected: same test count as Task 1 Step 4 (59), all passing.

- [ ] **Step 6: Confirm `deps/mod.rs` line count**

Run:
```bash
wc -l crates/scrat-core/src/deps/mod.rs
```
Expected:
```
    1343 crates/scrat-core/src/deps/mod.rs
```

- [ ] **Step 7: Write commit.txt**

Create `commit.txt` at the repo root with this EXACT content (note the real intro paragraph — NOT a placeholder):

```
refactor(deps): extract per-ecosystem lockfile diff parsers into deps/

Phase 3 of the ecosystem-modules refactor. Extracts the seven
per-ecosystem lockfile diff parsers from deps.rs into sibling files
under a new deps/ module directory, and introduces the first trait in
the refactor arc — LockfileDiffParser — as a minimal &self interface
on zero-sized unit structs. Builds on Phase 1 (detect/, PR #37) and
Phase 2 (bump/, PR #38). No behavior change: every parser's state
machine, sort ordering, and test coverage moves verbatim.

* refactor(deps): convert deps.rs to deps/mod.rs

Pure filesystem restructure. git mv preserves blame history. The
`pub mod deps;` declaration in lib.rs resolves to deps/mod.rs
automatically — no import path changes needed anywhere in the
workspace.

No behavior change. All 59 deps:: tests still pass.
```

- [ ] **Step 8: Stop — Clay will run gtxt**

Do NOT run `git commit` directly. Wait for Clay to run `gtxt` before proceeding to Task 3. If Clay chooses to batch the commit with later tasks, proceed to Task 3 without running gtxt yourself.

---

### Task 3: Introduce `LockfileDiffParser` trait + 7 unit structs + rewrite dispatch (ATOMIC)

**Files:**
- Modify: `crates/scrat-core/src/deps/mod.rs`

**This task is atomic.** Introducing the trait, adding 7 unit struct impls, and rewriting the dispatch must happen together — any partial state produces either unused types (clippy warns) or a broken dispatch (doesn't compile). Do NOT split this task across multiple commits.

At the end of this task, `deps/mod.rs` contains:
- Unchanged: `compute_deps` body (except the dispatch match) + all existing `parse_*_diff` free functions + all shared helpers + all 59 tests
- New: `LockfileDiffParser` trait definition + 7 unit struct declarations + 7 `impl LockfileDiffParser for <Lang>LockfileParser` blocks (each delegating to the corresponding existing `parse_*_diff` free function)
- Changed: the dispatch match inside `compute_deps` now calls `<Lang>LockfileParser.parse_diff(&diff)` instead of the free function directly

The sibling files do NOT exist yet. Task 3 leaves everything in `deps/mod.rs` — Tasks 4–10 will peel each ecosystem out into its own file.

- [ ] **Step 1: Read the current `deps/mod.rs`**

Use the Read tool to load `crates/scrat-core/src/deps/mod.rs` so you have the full file in context.

- [ ] **Step 2: Add the `LockfileDiffParser` trait after the imports**

Find the imports block at the top of `deps/mod.rs` (around lines 17–21):

```rust
use tracing::{debug, warn};

use crate::ecosystem::Ecosystem;
use crate::git;
use crate::pipeline::DepChange;
```

Immediately after the imports (before `compute_deps`), insert:

```rust
/// Parses a unified diff of an ecosystem-specific lockfile into
/// [`DepChange`] entries.
///
/// Implemented by zero-sized unit structs per ecosystem
/// (`RustLockfileParser`, `NodeLockfileParser`, …). The `&self` receiver
/// carries no state today, but preserves Phase 4's flexibility to attach
/// per-ecosystem state (e.g., `RustDriver { bump_cmd }`) without changing
/// the method signature.
///
/// Parsers are infallible by convention: malformed input returns an empty
/// `Vec` rather than an error, matching the existing "deps diff failure is
/// non-fatal" contract established by [`compute_deps`].
pub trait LockfileDiffParser {
    /// Parse a unified diff into dependency changes.
    ///
    /// Returns an empty `Vec` if the diff contains no recognizable
    /// dependency changes. Implementations must sort the result by
    /// `DepChange.name` for deterministic output.
    fn parse_diff(&self, diff: &str) -> Vec<DepChange>;
}

/// Lockfile diff parser for Rust's `Cargo.lock`.
pub struct RustLockfileParser;

impl LockfileDiffParser for RustLockfileParser {
    fn parse_diff(&self, diff: &str) -> Vec<DepChange> {
        parse_cargo_lock_diff(diff)
    }
}

/// Lockfile diff parser for Node's `package-lock.json`.
pub struct NodeLockfileParser;

impl LockfileDiffParser for NodeLockfileParser {
    fn parse_diff(&self, diff: &str) -> Vec<DepChange> {
        parse_package_lock_diff(diff)
    }
}

/// Lockfile diff parser for Go's `go.mod`.
pub struct GoLockfileParser;

impl LockfileDiffParser for GoLockfileParser {
    fn parse_diff(&self, diff: &str) -> Vec<DepChange> {
        parse_go_mod_diff(diff)
    }
}

/// Lockfile diff parser for PHP's `composer.lock`.
pub struct PhpLockfileParser;

impl LockfileDiffParser for PhpLockfileParser {
    fn parse_diff(&self, diff: &str) -> Vec<DepChange> {
        parse_composer_lock_diff(diff)
    }
}

/// Lockfile diff parser for Python's `uv.lock`.
///
/// `uv.lock` currently uses the same TOML `[[package]]` format as
/// `Cargo.lock`. The Task 3 impl body calls `parse_uv_lock_diff`
/// (which itself is a one-line delegation to `parse_cargo_lock_diff`),
/// preserving the existing indirection and keeping clippy's dead_code
/// lint quiet. Task 8 (Python extraction) replaces this with a direct
/// `super::rust::RustLockfileParser.parse_diff(diff)` call and deletes
/// `parse_uv_lock_diff` from `deps/mod.rs`.
///
/// This is an incidental format match, not a shared abstraction —
/// if uv diverges from Cargo's format, the impl grows its own state
/// machine in `deps/python.rs`.
pub struct PythonLockfileParser;

impl LockfileDiffParser for PythonLockfileParser {
    fn parse_diff(&self, diff: &str) -> Vec<DepChange> {
        parse_uv_lock_diff(diff)
    }
}

/// Lockfile diff parser for Ruby's `Gemfile.lock`.
pub struct RubyLockfileParser;

impl LockfileDiffParser for RubyLockfileParser {
    fn parse_diff(&self, diff: &str) -> Vec<DepChange> {
        parse_gemfile_lock_diff(diff)
    }
}

/// Lockfile diff parser for Swift's `Package.resolved`.
pub struct SwiftLockfileParser;

impl LockfileDiffParser for SwiftLockfileParser {
    fn parse_diff(&self, diff: &str) -> Vec<DepChange> {
        parse_package_resolved_diff(diff)
    }
}
```

- [ ] **Step 3: Rewrite the dispatch in `compute_deps`**

Find the dispatch match inside `compute_deps` (around lines 46–55):

```rust
    let changes = match ecosystem {
        Ecosystem::Rust => parse_cargo_lock_diff(&diff),
        Ecosystem::Node => parse_package_lock_diff(&diff),
        Ecosystem::Go => parse_go_mod_diff(&diff),
        Ecosystem::Php => parse_composer_lock_diff(&diff),
        Ecosystem::Python => parse_uv_lock_diff(&diff),
        Ecosystem::Ruby => parse_gemfile_lock_diff(&diff),
        Ecosystem::Swift => parse_package_resolved_diff(&diff),
        Ecosystem::Generic => Vec::new(),
    };
```

Replace with:

```rust
    let changes = match ecosystem {
        Ecosystem::Rust    => RustLockfileParser.parse_diff(&diff),
        Ecosystem::Node    => NodeLockfileParser.parse_diff(&diff),
        Ecosystem::Go      => GoLockfileParser.parse_diff(&diff),
        Ecosystem::Php     => PhpLockfileParser.parse_diff(&diff),
        Ecosystem::Python  => PythonLockfileParser.parse_diff(&diff),
        Ecosystem::Ruby    => RubyLockfileParser.parse_diff(&diff),
        Ecosystem::Swift   => SwiftLockfileParser.parse_diff(&diff),
        Ecosystem::Generic => Vec::new(),
    };
```

Note the column alignment for readability — matches Phase 2's `ReadyBump::execute` dispatch style.

- [ ] **Step 4: Verify compilation**

Run (with `dangerouslyDisableSandbox: true`):
```bash
cargo check -p scrat-core
```
Expected: clean build. The free functions (`parse_cargo_lock_diff`, `parse_package_lock_diff`, etc.) are still used — by the impl blocks — so no unused-function warnings.

- [ ] **Step 5: Verify clippy is clean**

Run (with `dangerouslyDisableSandbox: true`):
```bash
cargo clippy -p scrat-core --all-targets -- -D warnings
```
Expected: no warnings.

Possible clippy concerns to watch for:
- `clippy::new_without_default` on unit structs — unit structs don't need `Default` impls, so this shouldn't fire. If it does, the impls are wrong.
- `clippy::needless_pass_by_value` — `fn parse_diff(&self, diff: &str)` passes `&self` and `&str`, neither is by-value. Won't fire.
- `clippy::unused_self` — the trait method body uses `diff` but not `self`. This MIGHT fire on implementations. If it does, the fix is to add `#[allow(clippy::unused_self)]` on the trait definition with a comment explaining Phase 4's state attachment plan.

- [ ] **Step 6: Run deps-module tests**

Run (with `dangerouslyDisableSandbox: true`):
```bash
cargo nextest run -p scrat-core deps::
```
Expected: 59 tests pass. No behavior change — the dispatch is identical except for the indirection through the trait.

- [ ] **Step 7: Append sub-bullet to commit.txt**

**CRITICAL: APPEND, do NOT overwrite.** If `commit.txt` no longer exists (Clay ran `gtxt` after Task 2), re-create it with the Task 2 header and subject line + intro paragraph + the Task 2 sub-bullet, then append this Task 3 sub-bullet. If `commit.txt` exists from Task 2, use the Edit tool to APPEND the Task 3 sub-bullet after the Task 2 sub-bullet.

Append this exact text to `commit.txt`:

```
* refactor(deps): introduce LockfileDiffParser trait and unit structs

Adds the LockfileDiffParser trait and seven unit struct impls
(RustLockfileParser, NodeLockfileParser, GoLockfileParser,
PhpLockfileParser, PythonLockfileParser, RubyLockfileParser,
SwiftLockfileParser) in deps/mod.rs. The dispatch in compute_deps
now calls trait methods instead of free functions directly.

PythonLockfileParser delegates to RustLockfileParser because
uv.lock currently uses the same TOML [[package]] format as
Cargo.lock. This is documented as an incidental format match,
not a shared abstraction commitment.

No behavior change. All 59 deps:: tests still pass.
```

- [ ] **Step 8: Stop — Clay may run gtxt or continue**

Do NOT run `git commit` directly. Wait for Clay's signal to proceed to Task 4.

---

### Task 4: Extract `RustLockfileParser` to `deps/rust.rs` (template validation)

**Files:**
- Create: `crates/scrat-core/src/deps/rust.rs`
- Modify: `crates/scrat-core/src/deps/mod.rs`

This is the FIRST extraction and serves as the **template validator** for Tasks 5–10. Code-quality review runs on this task (not just spec review). If the extraction pattern here compiles, passes tests, and passes clippy cleanly, Tasks 5–10 replicate this shape mechanically.

The shape to validate:
1. Sibling file starts with a module doc comment describing what it parses
2. Module-level imports (no inline `use`)
3. `pub struct <Lang>LockfileParser;` declaration
4. `impl LockfileDiffParser for <Lang>LockfileParser` with the inlined parser logic (not a delegating impl body anymore — the actual state machine moves into the impl body)
5. Any private helpers that are exclusive to this ecosystem become private functions in the sibling file
6. `#[cfg(test)] mod tests { ... }` containing the extracted test functions verbatim
7. `deps/mod.rs` declares `mod <name>;` and `pub use <name>::<Lang>LockfileParser;` (re-export keeps `compute_deps`'s dispatch unchanged)
8. The free function (`parse_cargo_lock_diff`) is deleted from `deps/mod.rs`
9. The delegating `impl LockfileDiffParser for RustLockfileParser` and the `pub struct RustLockfileParser;` are deleted from `deps/mod.rs` (they've moved to the sibling file)
10. The 7 Rust parser tests are deleted from `deps/mod.rs::tests` and reappear verbatim in `deps/rust.rs::tests`

- [ ] **Step 1: Read `deps/mod.rs` to locate Rust-specific code**

Use the Read tool. Locate:
- `parse_cargo_lock_diff` (starts around line 70)
- `pub struct RustLockfileParser;` and its impl block (added in Task 3)
- The 7 `parse_cargo_lock_diff_*` test functions in the `#[cfg(test)]` module

- [ ] **Step 2: Create `deps/rust.rs`**

Create the file with this content:

```rust
//! Lockfile diff parser for Rust's `Cargo.lock`.
//!
//! Implements [`LockfileDiffParser`] via a TOML state machine that tracks
//! per-`[[package]]` blocks, extracting `name` and `version` fields from
//! context, removed, and added lines in a unified diff.

use super::{LockfileDiffParser, emit_change, extract_toml_string_value};
use crate::pipeline::DepChange;

/// Lockfile diff parser for Rust's `Cargo.lock`.
pub struct RustLockfileParser;

impl LockfileDiffParser for RustLockfileParser {
    /// Parse a unified diff of `Cargo.lock` into dependency changes.
    ///
    /// State machine tracking per-`[[package]]` blocks:
    /// - `name` from any `name = "..."` line (context, removed, or added)
    /// - `old_version` from `-version = "..."` lines
    /// - `new_version` from `+version = "..."` lines
    ///
    /// At each `[[package]]` boundary or EOF, emits a [`DepChange`] if
    /// we have a name and at least one version that changed.
    fn parse_diff(&self, diff: &str) -> Vec<DepChange> {
        let mut changes: Vec<DepChange> = Vec::new();

        let mut current_name: Option<String> = None;
        let mut old_version: Option<String> = None;
        let mut new_version: Option<String> = None;

        for line in diff.lines() {
            // [[package]] boundary — any prefix (context, +, -)
            let trimmed = line
                .strip_prefix(' ')
                .or_else(|| line.strip_prefix('+'))
                .or_else(|| line.strip_prefix('-'))
                .unwrap_or(line);

            if trimmed.starts_with("[[package]]") {
                // Emit pending change from previous block
                emit_change(&mut changes, &current_name, &old_version, &new_version);
                current_name = None;
                old_version = None;
                new_version = None;
                continue;
            }

            // name = "..." — appears in context, removed, or added lines
            if let Some(name) = extract_toml_string_value(trimmed, "name") {
                current_name = Some(name);
                continue;
            }

            // -version = "..." — old version (removed line)
            if line.starts_with('-') {
                if let Some(ver) = extract_toml_string_value(trimmed, "version") {
                    old_version = Some(ver);
                }
                continue;
            }

            // +version = "..." — new version (added line)
            if line.starts_with('+')
                && let Some(ver) = extract_toml_string_value(trimmed, "version")
            {
                new_version = Some(ver);
            }
        }

        // Emit final pending block
        emit_change(&mut changes, &current_name, &old_version, &new_version);

        // Stable ordering
        changes.sort_by(|a, b| a.name.cmp(&b.name));
        changes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_cargo_lock_diff_update() {
        let diff = r#"
 [[package]]
 name = "serde"
-version = "1.0.0"
+version = "1.0.1"
 source = "registry+https://github.com/rust-lang/crates.io-index"
"#;
        let changes = RustLockfileParser.parse_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "serde");
        assert_eq!(changes[0].from.as_deref(), Some("1.0.0"));
        assert_eq!(changes[0].to.as_deref(), Some("1.0.1"));
    }

    #[test]
    fn parse_cargo_lock_diff_added() {
        let diff = r#"
+[[package]]
+name = "new-crate"
+version = "0.1.0"
+source = "registry+https://github.com/rust-lang/crates.io-index"
"#;
        let changes = RustLockfileParser.parse_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "new-crate");
        assert_eq!(changes[0].from, None);
        assert_eq!(changes[0].to.as_deref(), Some("0.1.0"));
    }

    #[test]
    fn parse_cargo_lock_diff_removed() {
        let diff = r#"
-[[package]]
-name = "old-crate"
-version = "2.0.0"
-source = "registry+https://github.com/rust-lang/crates.io-index"
"#;
        let changes = RustLockfileParser.parse_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "old-crate");
        assert_eq!(changes[0].from.as_deref(), Some("2.0.0"));
        assert_eq!(changes[0].to, None);
    }

    #[test]
    fn parse_cargo_lock_diff_mixed() {
        let diff = r#"
 [[package]]
 name = "serde"
-version = "1.0.0"
+version = "1.0.1"
 source = "registry+https://github.com/rust-lang/crates.io-index"
+[[package]]
+name = "new-crate"
+version = "0.1.0"
+source = "registry+https://github.com/rust-lang/crates.io-index"
-[[package]]
-name = "old-crate"
-version = "2.0.0"
-source = "registry+https://github.com/rust-lang/crates.io-index"
"#;
        let changes = RustLockfileParser.parse_diff(diff);
        assert_eq!(changes.len(), 3);
        // Sorted by name
        assert_eq!(changes[0].name, "new-crate");
        assert_eq!(changes[1].name, "old-crate");
        assert_eq!(changes[2].name, "serde");
    }

    #[test]
    fn parse_cargo_lock_diff_empty() {
        let changes = RustLockfileParser.parse_diff("");
        assert!(changes.is_empty());
    }

    #[test]
    fn parse_cargo_lock_diff_no_version_change() {
        // A block where name appears but no version lines changed — no dep change
        let diff = r#"
 [[package]]
 name = "unchanged"
 version = "1.0.0"
 source = "registry+https://github.com/rust-lang/crates.io-index"
-dependencies = []
+dependencies = ["foo"]
"#;
        let changes = RustLockfileParser.parse_diff(diff);
        assert!(changes.is_empty());
    }

    #[test]
    fn parse_cargo_lock_diff_sorted() {
        let diff = r#"
 [[package]]
 name = "zebra"
-version = "1.0.0"
+version = "2.0.0"
 [[package]]
 name = "alpha"
-version = "0.1.0"
+version = "0.2.0"
 [[package]]
 name = "middle"
-version = "3.0.0"
+version = "3.1.0"
"#;
        let changes = RustLockfileParser.parse_diff(diff);
        assert_eq!(changes.len(), 3);
        assert_eq!(changes[0].name, "alpha");
        assert_eq!(changes[1].name, "middle");
        assert_eq!(changes[2].name, "zebra");
    }
}
```

**Note:** the test bodies call `RustLockfileParser.parse_diff(diff)` instead of the old `parse_cargo_lock_diff(diff)`. This is the only structural change to the tests — same inputs, same assertions, trait-method invocation instead of free-function call.

- [ ] **Step 3: Update `deps/mod.rs` — add module declaration and re-export**

In `deps/mod.rs`, find the top of the file (after the module doc comment and before the imports). Add module declarations right after the imports block:

```rust
mod rust;

pub use rust::RustLockfileParser;
```

(Placement after imports, before the trait definition. The module declaration makes `deps::rust` a child module; the `pub use` re-export keeps `deps::RustLockfileParser` accessible for the dispatch match which references `RustLockfileParser.parse_diff(...)` unqualified.)

- [ ] **Step 4: Delete the Rust-specific items from `deps/mod.rs`**

Remove from `deps/mod.rs`:
1. The `pub struct RustLockfileParser;` declaration (added in Task 3)
2. The `impl LockfileDiffParser for RustLockfileParser { ... }` block (added in Task 3)
3. The `fn parse_cargo_lock_diff` function (original — was at ~line 70)
4. The 7 `parse_cargo_lock_diff_*` test functions inside `#[cfg(test)] mod tests { ... }`

After deletion, `deps/mod.rs` has 6 unit structs + impls remaining (Node, Go, Php, Python, Ruby, Swift), and 52 tests remaining in its `#[cfg(test)] mod tests` (59 − 7 = 52).

Important: `emit_change` and `extract_toml_string_value` are still used by `deps/mod.rs` (by Python's delegating impl, which calls `RustLockfileParser.parse_diff`, which in turn uses `extract_toml_string_value` — but after extraction to `deps/rust.rs`, Python's delegation calls the sibling-file impl which imports from `super::`). Keep `emit_change` and `extract_toml_string_value` in `deps/mod.rs` as they are — mark them `pub(super)` if not already, so `deps/rust.rs` can import them via `use super::{emit_change, extract_toml_string_value};`.

- [ ] **Step 5: Make shared helpers `pub(super)` if not already**

Verify `deps/mod.rs` has these helpers visible to child modules:

```rust
pub(super) fn emit_change(
    changes: &mut Vec<DepChange>,
    name: &Option<String>,
    old_version: &Option<String>,
    new_version: &Option<String>,
) { ... }

pub(super) fn extract_toml_string_value(line: &str, key: &str) -> Option<String> { ... }
```

If they're currently private (no visibility keyword), add `pub(super)`. If they already have `pub(super)`, leave them. Do NOT make them `pub` — they are internal to the `deps` module tree.

- [ ] **Step 6: Verify compilation**

Run (with `dangerouslyDisableSandbox: true`):
```bash
cargo check -p scrat-core
```
Expected: clean build.

- [ ] **Step 7: Verify clippy is clean**

Run (with `dangerouslyDisableSandbox: true`):
```bash
cargo clippy -p scrat-core --all-targets -- -D warnings
```
Expected: no warnings.

- [ ] **Step 8: Verify all deps tests still pass**

Run (with `dangerouslyDisableSandbox: true`):
```bash
cargo nextest run -p scrat-core deps::
```
Expected: 59 tests pass. Of these, 7 now come from `deps::rust::tests::parse_cargo_lock_diff_*`, and 52 from `deps::tests::*`.

Verify test provenance with:
```bash
cargo nextest list -p scrat-core deps::rust::
```
Expected output contains 7 test names, all starting with `parse_cargo_lock_diff_`.

```bash
cargo nextest list -p scrat-core deps::tests::parse_cargo_lock_diff_
```
Expected: no matches. The Rust parser tests have moved out of `deps::tests::`.

- [ ] **Step 9: Confirm line counts**

Run:
```bash
wc -l crates/scrat-core/src/deps/mod.rs crates/scrat-core/src/deps/rust.rs
```
Expected: `deps/mod.rs` is now roughly 1343 − 140 ≈ 1200 LOC (minus the trait + impl block that stays in mod.rs for the other 6 ecosystems, so the actual number is around 1200). `deps/rust.rs` is roughly 140 LOC. Exact numbers depend on Task 3's trait comment length; don't block on precision here — the key check is that `deps/rust.rs` contains the parser and its 7 tests.

- [ ] **Step 10: Append sub-bullet to commit.txt**

APPEND this sub-bullet to `commit.txt` (recreate the header + subject + intro paragraph + prior sub-bullets if `gtxt` was run since the last append):

```
* refactor(deps): extract RustLockfileParser to deps/rust.rs

First sibling file in the deps/ directory. Moves parse_cargo_lock_diff
logic into RustLockfileParser's impl body and the 7 parse_cargo_lock_diff_*
tests into deps::rust::tests. Uses `use super::{emit_change,
extract_toml_string_value, LockfileDiffParser}` to pull shared helpers
from deps/mod.rs. Validates the extraction template for Tasks 5-10.

No behavior change. All 59 deps:: tests still pass (7 now in deps::rust::).
```

- [ ] **Step 11: Stop — Clay may run gtxt or continue**

Wait for Clay's signal to proceed to Task 5. If Clay says "go" or similar, proceed.

---

### Task 5: Extract `NodeLockfileParser` to `deps/node.rs`

**Files:**
- Create: `crates/scrat-core/src/deps/node.rs`
- Modify: `crates/scrat-core/src/deps/mod.rs`

Second extraction. Node is the BIGGEST single sibling file (15 tests, ~240 LOC target) because it carries two private helpers (`extract_top_level_node_modules_name`, `extract_json_version`) that are exclusive to Node and move with it. **Code-quality review runs on this task** (second review gate after Task 4).

Template replicated from Task 4 with these deltas:
1. `use super::LockfileDiffParser;` only — no shared helpers imported (Node uses its own private helpers, not `emit_change` / `extract_toml_string_value` / `extract_json_string_value`). Wait — Node DOES use `emit_change`. Check the function body before writing the imports.

Correction: reading `parse_package_lock_diff` in the current `deps.rs`, it does call `emit_change`. So the imports are:

```rust
use super::{LockfileDiffParser, emit_change};
use crate::pipeline::DepChange;
```

Node does NOT use `extract_json_string_value` — that's PHP/Swift's helper. Node has its own `extract_json_version` which is distinct (extracts `"version": "..."` specifically). `extract_json_version` moves INTO `deps/node.rs` as a private function. Same for `extract_top_level_node_modules_name`.

- [ ] **Step 1: Read `deps/mod.rs` to locate Node-specific code**

Use the Read tool. Locate:
- `parse_package_lock_diff` (the Node parser)
- `extract_top_level_node_modules_name` (Node-private helper)
- `extract_json_version` (Node-private helper)
- `pub struct NodeLockfileParser;` + its impl (added in Task 3)
- The 15 tests: 9 `parse_package_lock_diff_*` + 4 `extract_top_level_node_modules_name_*` + 2 `extract_json_version_*`

- [ ] **Step 2: Create `deps/node.rs`**

Create the file with a module doc comment, imports, struct + impl, and the two private helpers, followed by the 15 tests. The impl body contains the full `parse_package_lock_diff` state machine. The private helpers are plain module-private functions (no visibility keyword).

Structure:

```rust
//! Lockfile diff parser for Node's `package-lock.json`.
//!
//! Targets npm lockfile version 2 and 3, which use the `packages` key
//! with paths like `"node_modules/<name>": { "version": "..." }`. Only
//! top-level packages are reported — nested entries like
//! `"node_modules/foo/node_modules/bar"` are intentionally skipped so
//! release notes focus on direct dependency changes.
//!
//! Scoped packages (`"node_modules/@scope/name"`) are preserved as
//! `@scope/name`.

use super::{LockfileDiffParser, emit_change};
use crate::pipeline::DepChange;

/// Lockfile diff parser for Node's `package-lock.json`.
pub struct NodeLockfileParser;

impl LockfileDiffParser for NodeLockfileParser {
    fn parse_diff(&self, diff: &str) -> Vec<DepChange> {
        let mut changes: Vec<DepChange> = Vec::new();

        let mut current_name: Option<String> = None;
        let mut old_version: Option<String> = None;
        let mut new_version: Option<String> = None;

        for line in diff.lines() {
            // Classify the diff line and strip the leading marker ('+', '-', ' ').
            let (is_removal, is_addition, content) = if let Some(s) = line.strip_prefix('-') {
                // Ignore the `--- a/path` header
                if s.starts_with("-- ") || s.is_empty() {
                    continue;
                }
                (true, false, s)
            } else if let Some(s) = line.strip_prefix('+') {
                // Ignore the `+++ b/path` header
                if s.starts_with("++ ") || s.is_empty() {
                    continue;
                }
                (false, true, s)
            } else if let Some(s) = line.strip_prefix(' ') {
                (false, false, s)
            } else {
                // Hunk headers (`@@`) and anything else we don't care about.
                continue;
            };

            let trimmed = content.trim_start();

            // A new `"node_modules/..."` package block starts a new logical unit.
            if let Some(name) = extract_top_level_node_modules_name(trimmed) {
                // Flush the previous block
                emit_change(&mut changes, &current_name, &old_version, &new_version);
                current_name = Some(name);
                old_version = None;
                new_version = None;
                continue;
            }

            // Version lines within the current block
            if current_name.is_some()
                && let Some(version) = extract_json_version(trimmed)
            {
                match (is_removal, is_addition) {
                    (true, _) => old_version = Some(version),
                    (_, true) => new_version = Some(version),
                    // Context lines (unchanged) provide the baseline version for
                    // blocks where only one side actually changes a field.
                    _ => {
                        if old_version.is_none() {
                            old_version = Some(version.clone());
                        }
                        if new_version.is_none() {
                            new_version = Some(version);
                        }
                    }
                }
            }
        }

        // Emit final pending block
        emit_change(&mut changes, &current_name, &old_version, &new_version);

        // Stable ordering for deterministic output
        changes.sort_by(|a, b| a.name.cmp(&b.name));
        changes
    }
}

/// Extract a top-level `node_modules/<name>` path from a JSON key line.
/// Returns `None` for nested entries or non-matching lines.
fn extract_top_level_node_modules_name(line: &str) -> Option<String> {
    let rest = line.strip_prefix('"')?;
    let close = rest.find('"')?;
    let path = &rest[..close];
    let name = path.strip_prefix("node_modules/")?;
    // Reject nested entries like `node_modules/express/node_modules/debug`.
    if name.contains("/node_modules/") {
        return None;
    }
    // Ensure this really is a key (next significant chars are `": {`).
    let after = &rest[close + 1..];
    if !after.trim_start().starts_with(": {") {
        return None;
    }
    Some(name.to_string())
}

/// Extract a version string from a `"version": "x.y.z"` JSON line.
fn extract_json_version(line: &str) -> Option<String> {
    let rest = line.strip_prefix("\"version\":")?;
    let rest = rest.trim_start();
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── NodeLockfileParser.parse_diff ──────────────────────────────

    #[test]
    fn parse_package_lock_diff_version_update() {
        // npm lockfile v3 format — common case: version bump for express
        let diff = r#"
     "node_modules/express": {
-      "version": "4.17.1",
+      "version": "4.18.2",
       "resolved": "https://registry.npmjs.org/express/-/express-4.18.2.tgz",
       "integrity": "sha512-..."
     },
"#;
        let changes = NodeLockfileParser.parse_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "express");
        assert_eq!(changes[0].from.as_deref(), Some("4.17.1"));
        assert_eq!(changes[0].to.as_deref(), Some("4.18.2"));
    }

    #[test]
    fn parse_package_lock_diff_added_dependency() {
        let diff = r#"
+    "node_modules/chalk": {
+      "version": "5.3.0",
+      "resolved": "https://registry.npmjs.org/chalk/-/chalk-5.3.0.tgz",
+      "license": "MIT"
+    },
"#;
        let changes = NodeLockfileParser.parse_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "chalk");
        assert_eq!(changes[0].from, None);
        assert_eq!(changes[0].to.as_deref(), Some("5.3.0"));
    }

    #[test]
    fn parse_package_lock_diff_removed_dependency() {
        let diff = r#"
-    "node_modules/lodash": {
-      "version": "4.17.21",
-      "resolved": "https://registry.npmjs.org/lodash/-/lodash-4.17.21.tgz",
-      "license": "MIT"
-    },
"#;
        let changes = NodeLockfileParser.parse_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "lodash");
        assert_eq!(changes[0].from.as_deref(), Some("4.17.21"));
        assert_eq!(changes[0].to, None);
    }

    #[test]
    fn parse_package_lock_diff_scoped_package() {
        let diff = r#"
     "node_modules/@babel/core": {
-      "version": "7.22.5",
+      "version": "7.23.0",
       "resolved": "https://registry.npmjs.org/@babel/core/-/core-7.23.0.tgz"
     },
"#;
        let changes = NodeLockfileParser.parse_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "@babel/core");
        assert_eq!(changes[0].from.as_deref(), Some("7.22.5"));
        assert_eq!(changes[0].to.as_deref(), Some("7.23.0"));
    }

    #[test]
    fn parse_package_lock_diff_skips_nested_dependencies() {
        // Nested node_modules (deep dedup) should NOT appear in output —
        // release notes focus on top-level changes only.
        let diff = r#"
     "node_modules/express/node_modules/debug": {
-      "version": "2.6.8",
+      "version": "2.6.9"
     },
"#;
        let changes = NodeLockfileParser.parse_diff(diff);
        assert!(changes.is_empty(), "nested entries should be skipped");
    }

    #[test]
    fn parse_package_lock_diff_mixed_changes() {
        let diff = r#"
     "node_modules/express": {
-      "version": "4.17.1",
+      "version": "4.18.2",
       "resolved": "https://..."
     },
+    "node_modules/chalk": {
+      "version": "5.3.0"
+    },
-    "node_modules/lodash": {
-      "version": "4.17.21"
-    },
"#;
        let changes = NodeLockfileParser.parse_diff(diff);
        assert_eq!(changes.len(), 3);
        // Sorted alphabetically
        assert_eq!(changes[0].name, "chalk");
        assert_eq!(changes[1].name, "express");
        assert_eq!(changes[2].name, "lodash");
    }

    #[test]
    fn parse_package_lock_diff_empty_diff() {
        assert!(NodeLockfileParser.parse_diff("").is_empty());
    }

    #[test]
    fn parse_package_lock_diff_ignores_diff_headers() {
        // Don't mistake `--- a/package-lock.json` / `+++ b/...` for content
        let diff = r#"--- a/package-lock.json
+++ b/package-lock.json
@@ -12,7 +12,7 @@
     "node_modules/express": {
-      "version": "4.17.1",
+      "version": "4.18.2"
     }"#;
        let changes = NodeLockfileParser.parse_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "express");
    }

    #[test]
    fn parse_package_lock_diff_no_version_change() {
        // Package block mentioned but only non-version fields changed
        let diff = r#"
     "node_modules/express": {
       "version": "4.18.2",
-      "resolved": "https://old-registry/..."
+      "resolved": "https://new-registry/..."
     },
"#;
        let changes = NodeLockfileParser.parse_diff(diff);
        assert!(
            changes.is_empty(),
            "only version changes should be reported"
        );
    }

    // ── extract_top_level_node_modules_name ────────────────────────

    #[test]
    fn extract_top_level_node_modules_name_basic() {
        assert_eq!(
            extract_top_level_node_modules_name("\"node_modules/express\": {"),
            Some("express".to_string())
        );
    }

    #[test]
    fn extract_top_level_node_modules_name_scoped() {
        assert_eq!(
            extract_top_level_node_modules_name("\"node_modules/@babel/core\": {"),
            Some("@babel/core".to_string())
        );
    }

    #[test]
    fn extract_top_level_node_modules_name_rejects_nested() {
        assert_eq!(
            extract_top_level_node_modules_name("\"node_modules/express/node_modules/debug\": {"),
            None
        );
    }

    #[test]
    fn extract_top_level_node_modules_name_rejects_non_package_key() {
        assert_eq!(
            extract_top_level_node_modules_name("\"name\": \"foo\""),
            None
        );
        assert_eq!(
            extract_top_level_node_modules_name("\"dependencies\": {}"),
            None
        );
    }

    // ── extract_json_version ───────────────────────────────────────

    #[test]
    fn extract_json_version_basic() {
        assert_eq!(
            extract_json_version("\"version\": \"1.2.3\""),
            Some("1.2.3".to_string())
        );
        assert_eq!(
            extract_json_version("\"version\":\"1.2.3\","),
            Some("1.2.3".to_string())
        );
    }

    #[test]
    fn extract_json_version_no_match() {
        assert_eq!(extract_json_version("\"name\": \"foo\""), None);
        assert_eq!(extract_json_version("\"resolved\": \"http://\""), None);
    }
}
```

- [ ] **Step 3: Update `deps/mod.rs` — add module declaration and re-export**

Add immediately after `pub use rust::RustLockfileParser;` in `deps/mod.rs`:

```rust
mod node;

pub use node::NodeLockfileParser;
```

- [ ] **Step 4: Delete the Node-specific items from `deps/mod.rs`**

Remove from `deps/mod.rs`:
1. `pub struct NodeLockfileParser;` and its impl block (added in Task 3)
2. `fn parse_package_lock_diff` (original parser)
3. `fn extract_top_level_node_modules_name` (Node-private helper)
4. `fn extract_json_version` (Node-private helper)
5. The 9 `parse_package_lock_diff_*` test functions
6. The 4 `extract_top_level_node_modules_name_*` test functions
7. The 2 `extract_json_version_*` test functions

After deletion, `deps/mod.rs::tests` has 52 − 15 = 37 tests remaining.

- [ ] **Step 5: Verify compilation**

Run (with `dangerouslyDisableSandbox: true`):
```bash
cargo check -p scrat-core
```
Expected: clean build.

- [ ] **Step 6: Verify clippy is clean**

Run (with `dangerouslyDisableSandbox: true`):
```bash
cargo clippy -p scrat-core --all-targets -- -D warnings
```
Expected: no warnings.

- [ ] **Step 7: Verify all deps tests still pass**

Run (with `dangerouslyDisableSandbox: true`):
```bash
cargo nextest run -p scrat-core deps::
```
Expected: 59 tests pass. 7 from `deps::rust::`, 15 from `deps::node::`, 37 from `deps::tests::`.

- [ ] **Step 8: Verify Node test provenance**

Run (with `dangerouslyDisableSandbox: true`):
```bash
cargo nextest list -p scrat-core deps::node::
```
Expected: 15 test names. Specifically: 9 `parse_package_lock_diff_*`, 4 `extract_top_level_node_modules_name_*`, 2 `extract_json_version_*`. Verify each test name individually — do NOT batch-verify this. If any test name is missing, the extraction dropped a test.

- [ ] **Step 9: Append sub-bullet to commit.txt**

APPEND this sub-bullet to `commit.txt`:

```
* refactor(deps): extract NodeLockfileParser to deps/node.rs

Largest single sibling file — carries parse_package_lock_diff plus
two Node-private helpers (extract_top_level_node_modules_name,
extract_json_version) and all 15 Node-related tests (9 package-lock
parsers + 4 extract_top_level + 2 extract_json_version).

No behavior change. All 59 deps:: tests still pass (15 now in deps::node::).
```

- [ ] **Step 10: Stop — Clay may run gtxt or continue**

---

### Task 6: Extract `GoLockfileParser` to `deps/go.rs`

**Files:**
- Create: `crates/scrat-core/src/deps/go.rs`
- Modify: `crates/scrat-core/src/deps/mod.rs`

Third extraction. Template replicated from Task 4. Go does NOT use any shared helpers (`emit_change`, `extract_toml_string_value`, `extract_json_string_value`) — it uses its own hash-map collect-and-merge approach with `std::collections::HashMap`. Import `HashMap` at module level, not inline.

- [ ] **Step 1: Read `deps/mod.rs` to locate Go-specific code**

- `parse_go_mod_diff` function (around line 411)
- `pub struct GoLockfileParser;` + impl (added in Task 3)
- 9 `parse_go_mod_diff_*` tests

- [ ] **Step 2: Create `deps/go.rs`**

```rust
//! Lockfile diff parser for Go's `go.mod`.
//!
//! Line-oriented collect-and-merge: each `require` line is
//! `<module> <version>`. Collects removed/added lines into maps, then
//! merges to produce [`DepChange`] entries.

use std::collections::HashMap;

use super::LockfileDiffParser;
use crate::pipeline::DepChange;

/// Lockfile diff parser for Go's `go.mod`.
pub struct GoLockfileParser;

impl LockfileDiffParser for GoLockfileParser {
    fn parse_diff(&self, diff: &str) -> Vec<DepChange> {
        let mut removed: HashMap<String, String> = HashMap::new();
        let mut added: HashMap<String, String> = HashMap::new();

        for line in diff.lines() {
            let (is_remove, is_add) = (line.starts_with('-'), line.starts_with('+'));
            if !is_remove && !is_add {
                continue;
            }

            // Strip diff prefix and whitespace
            let content = line[1..].trim();

            // Skip diff headers and require/block markers
            if content.starts_with("++")
                || content.starts_with("--")
                || content == "require ("
                || content == ")"
                || content.starts_with("module ")
                || content.starts_with("go ")
                || content.starts_with("toolchain ")
            {
                continue;
            }

            // Strip `// indirect` suffix
            let content = content.split("//").next().unwrap_or(content).trim_end();

            // Parse: <module-path> <version>
            let mut parts = content.split_whitespace();
            let Some(module) = parts.next() else {
                continue;
            };
            let Some(version) = parts.next() else {
                continue;
            };

            if is_remove {
                removed.insert(module.to_string(), version.to_string());
            } else {
                added.insert(module.to_string(), version.to_string());
            }
        }

        let mut changes: Vec<DepChange> = Vec::new();

        // Updated: in both removed and added
        for (name, old_ver) in &removed {
            if let Some(new_ver) = added.get(name) {
                if old_ver != new_ver {
                    changes.push(DepChange {
                        name: name.clone(),
                        from: Some(old_ver.clone()),
                        to: Some(new_ver.clone()),
                    });
                }
            } else {
                // Removed only
                changes.push(DepChange {
                    name: name.clone(),
                    from: Some(old_ver.clone()),
                    to: None,
                });
            }
        }

        // Added only: in added but not removed
        for (name, new_ver) in &added {
            if !removed.contains_key(name) {
                changes.push(DepChange {
                    name: name.clone(),
                    from: None,
                    to: Some(new_ver.clone()),
                });
            }
        }

        changes.sort_by(|a, b| a.name.cmp(&b.name));
        changes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_go_mod_diff_update() {
        let diff = "\
-\tgithub.com/spf13/cobra v1.7.0
+\tgithub.com/spf13/cobra v1.8.0";
        let changes = GoLockfileParser.parse_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "github.com/spf13/cobra");
        assert_eq!(changes[0].from.as_deref(), Some("v1.7.0"));
        assert_eq!(changes[0].to.as_deref(), Some("v1.8.0"));
    }

    #[test]
    fn parse_go_mod_diff_added() {
        let diff = "\
+\tgithub.com/new/dep v1.0.0";
        let changes = GoLockfileParser.parse_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "github.com/new/dep");
        assert_eq!(changes[0].from, None);
        assert_eq!(changes[0].to.as_deref(), Some("v1.0.0"));
    }

    #[test]
    fn parse_go_mod_diff_removed() {
        let diff = "\
-\tgithub.com/old/dep v2.0.0";
        let changes = GoLockfileParser.parse_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "github.com/old/dep");
        assert_eq!(changes[0].from.as_deref(), Some("v2.0.0"));
        assert_eq!(changes[0].to, None);
    }

    #[test]
    fn parse_go_mod_diff_indirect_stripped() {
        let diff = "\
-\tgolang.org/x/sys v0.14.0 // indirect
+\tgolang.org/x/sys v0.15.0 // indirect";
        let changes = GoLockfileParser.parse_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "golang.org/x/sys");
        assert_eq!(changes[0].from.as_deref(), Some("v0.14.0"));
        assert_eq!(changes[0].to.as_deref(), Some("v0.15.0"));
    }

    #[test]
    fn parse_go_mod_diff_mixed() {
        let diff = "\
-\tgithub.com/spf13/cobra v1.7.0
+\tgithub.com/spf13/cobra v1.8.0
+\tgithub.com/new/dep v1.0.0
-\tgithub.com/old/dep v2.0.0";
        let changes = GoLockfileParser.parse_diff(diff);
        assert_eq!(changes.len(), 3);
        assert_eq!(changes[0].name, "github.com/new/dep");
        assert_eq!(changes[1].name, "github.com/old/dep");
        assert_eq!(changes[2].name, "github.com/spf13/cobra");
    }

    #[test]
    fn parse_go_mod_diff_skips_headers() {
        let diff = "\
--- a/go.mod
+++ b/go.mod
-\tgithub.com/foo/bar v1.0.0
+\tgithub.com/foo/bar v1.1.0
-module github.com/my/project
+module github.com/my/project
-go 1.21
+go 1.22";
        let changes = GoLockfileParser.parse_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "github.com/foo/bar");
    }

    #[test]
    fn parse_go_mod_diff_major_version_path() {
        let diff = "\
-\tgithub.com/pelletier/go-toml/v2 v2.1.0
+\tgithub.com/pelletier/go-toml/v2 v2.2.0";
        let changes = GoLockfileParser.parse_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "github.com/pelletier/go-toml/v2");
    }

    #[test]
    fn parse_go_mod_diff_empty() {
        assert!(GoLockfileParser.parse_diff("").is_empty());
    }

    #[test]
    fn parse_go_mod_diff_pseudo_version() {
        let diff = "\
-\tgithub.com/foo/bar v0.0.0-20230905200255-921286631fa9
+\tgithub.com/foo/bar v0.0.0-20240101120000-abcdef123456";
        let changes = GoLockfileParser.parse_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(
            changes[0].from.as_deref(),
            Some("v0.0.0-20230905200255-921286631fa9")
        );
    }
}
```

- [ ] **Step 3: Update `deps/mod.rs` — add module declaration and re-export**

Add after `pub use node::NodeLockfileParser;`:

```rust
mod go;

pub use go::GoLockfileParser;
```

- [ ] **Step 4: Delete Go-specific items from `deps/mod.rs`**

Remove:
1. `pub struct GoLockfileParser;` + impl (added in Task 3)
2. `fn parse_go_mod_diff`
3. 9 `parse_go_mod_diff_*` tests

After deletion, `deps/mod.rs::tests` has 37 − 9 = 28 tests remaining.

- [ ] **Step 5: Verify compilation + clippy + tests**

Run in sequence (all with `dangerouslyDisableSandbox: true`):
```bash
cargo check -p scrat-core
cargo clippy -p scrat-core --all-targets -- -D warnings
cargo nextest run -p scrat-core deps::
```
Expected: all clean, 59 tests pass.

- [ ] **Step 6: Verify Go test provenance**

Run:
```bash
cargo nextest list -p scrat-core deps::go::
```
Expected: 9 test names starting with `parse_go_mod_diff_`.

- [ ] **Step 7: Append sub-bullet to commit.txt**

APPEND:

```
* refactor(deps): extract GoLockfileParser to deps/go.rs

Moves parse_go_mod_diff logic into GoLockfileParser's impl body
and the 9 parse_go_mod_diff_* tests into deps::go::tests. Uses
a hash-map collect-and-merge approach; no shared helpers needed.

No behavior change. All 59 deps:: tests still pass (9 now in deps::go::).
```

- [ ] **Step 8: Stop — Clay may run gtxt or continue**

---

### Task 7: Extract `PhpLockfileParser` to `deps/php.rs`

**Files:**
- Create: `crates/scrat-core/src/deps/php.rs`
- Modify: `crates/scrat-core/src/deps/mod.rs`

Fourth extraction. PHP uses `emit_change` and `extract_json_string_value` from `deps/mod.rs`.

- [ ] **Step 1: Read `deps/mod.rs` to locate PHP-specific code**

- `parse_composer_lock_diff` function
- `pub struct PhpLockfileParser;` + impl
- 7 `parse_composer_lock_diff_*` tests

- [ ] **Step 2: Create `deps/php.rs`**

```rust
//! Lockfile diff parser for PHP's `composer.lock`.
//!
//! JSON state machine tracking `"name":` boundaries in the diff,
//! similar in shape to the Cargo.lock parser but matching JSON key
//! patterns instead of TOML.

use super::{LockfileDiffParser, emit_change, extract_json_string_value};
use crate::pipeline::DepChange;

/// Lockfile diff parser for PHP's `composer.lock`.
pub struct PhpLockfileParser;

impl LockfileDiffParser for PhpLockfileParser {
    fn parse_diff(&self, diff: &str) -> Vec<DepChange> {
        let mut changes: Vec<DepChange> = Vec::new();

        let mut current_name: Option<String> = None;
        let mut old_version: Option<String> = None;
        let mut new_version: Option<String> = None;

        for line in diff.lines() {
            let trimmed = line
                .strip_prefix(' ')
                .or_else(|| line.strip_prefix('+'))
                .or_else(|| line.strip_prefix('-'))
                .unwrap_or(line)
                .trim();

            // "name": boundary — emit pending, start new tracking
            if let Some(name) = extract_json_string_value(trimmed, "name") {
                emit_change(&mut changes, &current_name, &old_version, &new_version);
                current_name = Some(name);
                old_version = None;
                new_version = None;
                continue;
            }

            // -"version": — old version
            if line.starts_with('-') {
                if let Some(ver) = extract_json_string_value(trimmed, "version") {
                    old_version = Some(ver);
                }
                continue;
            }

            // +"version": — new version
            if line.starts_with('+')
                && let Some(ver) = extract_json_string_value(trimmed, "version")
            {
                new_version = Some(ver);
            }
        }

        emit_change(&mut changes, &current_name, &old_version, &new_version);
        changes.sort_by(|a, b| a.name.cmp(&b.name));
        changes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_composer_lock_diff_update() {
        let diff = r#"
             "name": "sendgrid/php-http-client",
-            "version": "3.14.3",
+            "version": "3.14.4",
             "source": {
"#;
        let changes = PhpLockfileParser.parse_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "sendgrid/php-http-client");
        assert_eq!(changes[0].from.as_deref(), Some("3.14.3"));
        assert_eq!(changes[0].to.as_deref(), Some("3.14.4"));
    }

    #[test]
    fn parse_composer_lock_diff_added() {
        let diff = r#"
+            "name": "new/package",
+            "version": "1.0.0",
"#;
        let changes = PhpLockfileParser.parse_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "new/package");
        assert_eq!(changes[0].from, None);
        assert_eq!(changes[0].to.as_deref(), Some("1.0.0"));
    }

    #[test]
    fn parse_composer_lock_diff_removed() {
        let diff = r#"
-            "name": "old/package",
-            "version": "2.0.0",
"#;
        let changes = PhpLockfileParser.parse_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "old/package");
        assert_eq!(changes[0].from.as_deref(), Some("2.0.0"));
        assert_eq!(changes[0].to, None);
    }

    #[test]
    fn parse_composer_lock_diff_mixed() {
        let diff = r#"
             "name": "updated/pkg",
-            "version": "1.0.0",
+            "version": "1.1.0",
+            "name": "new/pkg",
+            "version": "0.1.0",
-            "name": "old/pkg",
-            "version": "3.0.0",
"#;
        let changes = PhpLockfileParser.parse_diff(diff);
        assert_eq!(changes.len(), 3);
        assert_eq!(changes[0].name, "new/pkg");
        assert_eq!(changes[1].name, "old/pkg");
        assert_eq!(changes[2].name, "updated/pkg");
    }

    #[test]
    fn parse_composer_lock_diff_ignores_reference() {
        let diff = r#"
             "name": "vendor/lib",
-            "version": "1.0.0",
+            "version": "1.0.1",
-                "reference": "abc123"
+                "reference": "def456"
"#;
        let changes = PhpLockfileParser.parse_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "vendor/lib");
        assert_eq!(changes[0].to.as_deref(), Some("1.0.1"));
    }

    #[test]
    fn parse_composer_lock_diff_empty() {
        assert!(PhpLockfileParser.parse_diff("").is_empty());
    }

    #[test]
    fn parse_composer_lock_diff_stability_suffix() {
        let diff = r#"
             "name": "vendor/lib",
-            "version": "1.12.17-patch7",
+            "version": "1.12.18",
"#;
        let changes = PhpLockfileParser.parse_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].from.as_deref(), Some("1.12.17-patch7"));
    }
}
```

- [ ] **Step 3: Update `deps/mod.rs` — add `mod php;` + `pub use php::PhpLockfileParser;`**

- [ ] **Step 4: Delete PHP-specific items from `deps/mod.rs`**

Remove `PhpLockfileParser` struct + impl, `fn parse_composer_lock_diff`, and the 7 `parse_composer_lock_diff_*` tests. After deletion, `deps/mod.rs::tests` has 28 − 7 = 21 tests remaining.

- [ ] **Step 5: Verify compilation + clippy + tests**

```bash
cargo check -p scrat-core
cargo clippy -p scrat-core --all-targets -- -D warnings
cargo nextest run -p scrat-core deps::
```
Expected: all clean, 59 tests pass.

- [ ] **Step 6: Verify PHP test provenance**

```bash
cargo nextest list -p scrat-core deps::php::
```
Expected: 7 test names starting with `parse_composer_lock_diff_`.

- [ ] **Step 7: Append sub-bullet to commit.txt**

```
* refactor(deps): extract PhpLockfileParser to deps/php.rs

Moves parse_composer_lock_diff logic into PhpLockfileParser's
impl body and the 7 parse_composer_lock_diff_* tests into
deps::php::tests. Imports emit_change and extract_json_string_value
from deps/mod.rs as shared helpers.

No behavior change. All 59 deps:: tests still pass (7 now in deps::php::).
```

- [ ] **Step 8: Stop — Clay may run gtxt or continue**

---

### Task 8: Extract `PythonLockfileParser` to `deps/python.rs` (DELEGATION + DOC COMMENT)

**Files:**
- Create: `crates/scrat-core/src/deps/python.rs`
- Modify: `crates/scrat-core/src/deps/mod.rs`

Fifth extraction. **This task is special**: Python delegates literally to `RustLockfileParser` instead of carrying its own state machine. The module-level doc comment explaining the rationale is **load-bearing** — without it, a future reader will "helpfully" extract a shared `parse_toml_package_diff` helper and break the intentional asymmetry.

After Task 8, `deps/mod.rs` no longer contains `parse_uv_lock_diff` (it was already a 3-line delegation to `parse_cargo_lock_diff`; now it's absorbed into the trait impl).

- [ ] **Step 1: Read `deps/mod.rs` to locate Python-specific code**

- `parse_uv_lock_diff` function (3-line delegation)
- `pub struct PythonLockfileParser;` + impl (added in Task 3, already delegates to `RustLockfileParser.parse_diff(diff)` via `parse_uv_lock_diff` — will need to update to delegate directly)
- 3 `parse_uv_lock_diff_*` tests

- [ ] **Step 2: Create `deps/python.rs`**

```rust
//! Lockfile diff parser for Python's `uv.lock`.
//!
//! `uv.lock` currently uses the same TOML `[[package]]` format as
//! `Cargo.lock`, so this parser literally delegates to
//! [`super::rust::RustLockfileParser`]. This is NOT a commitment to a
//! shared "TOML package diff" abstraction — it's an incidental format
//! match. If uv diverges from Cargo's lockfile format in a future
//! release, this module grows its own state machine and stops
//! delegating. Do not extract a shared TOML-package-diff helper on
//! the assumption that Python and Rust will always share an
//! implementation.

use super::{LockfileDiffParser, rust::RustLockfileParser};
use crate::pipeline::DepChange;

/// Lockfile diff parser for Python's `uv.lock`.
///
/// Delegates to [`RustLockfileParser`] because `uv.lock` currently uses
/// the same TOML `[[package]]` format as `Cargo.lock`. See the module
/// doc comment for the rationale behind this intentional delegation.
pub struct PythonLockfileParser;

impl LockfileDiffParser for PythonLockfileParser {
    fn parse_diff(&self, diff: &str) -> Vec<DepChange> {
        RustLockfileParser.parse_diff(diff)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_uv_lock_diff_update() {
        // Identical to Cargo.lock format
        let diff = r#"
 [[package]]
 name = "requests"
-version = "2.31.0"
+version = "2.32.0"
 source = { registry = "https://pypi.org/simple" }
"#;
        let changes = PythonLockfileParser.parse_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "requests");
        assert_eq!(changes[0].from.as_deref(), Some("2.31.0"));
        assert_eq!(changes[0].to.as_deref(), Some("2.32.0"));
    }

    #[test]
    fn parse_uv_lock_diff_added() {
        let diff = r#"
+[[package]]
+name = "new-dep"
+version = "1.0.0"
"#;
        let changes = PythonLockfileParser.parse_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].from, None);
        assert_eq!(changes[0].to.as_deref(), Some("1.0.0"));
    }

    #[test]
    fn parse_uv_lock_diff_skips_header() {
        // uv.lock has file-level version/requires-python before [[package]]
        let diff = r#"
-version = 1
+version = 2
 requires-python = ">=3.14"
 [[package]]
 name = "foo"
-version = "1.0.0"
+version = "1.1.0"
"#;
        let changes = PythonLockfileParser.parse_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "foo");
    }
}
```

**Naming note on the tests:** the test function names keep their `parse_uv_lock_diff_*` prefix rather than becoming `python_lockfile_parser_*`. This preserves grep-ability and matches the Phase 1/2 precedent of not renaming during mechanical refactors.

- [ ] **Step 3: Update `deps/mod.rs` — add `mod python;` + `pub use python::PythonLockfileParser;`**

Add after `pub use php::PhpLockfileParser;`:

```rust
mod python;

pub use python::PythonLockfileParser;
```

- [ ] **Step 4: Delete Python-specific items from `deps/mod.rs`**

Remove:
1. `pub struct PythonLockfileParser;` and its impl block (added in Task 3)
2. `fn parse_uv_lock_diff` (the 3-line delegation function — no longer needed; the sibling file handles it)
3. The 3 `parse_uv_lock_diff_*` test functions

After deletion, `deps/mod.rs::tests` has 21 − 3 = 18 tests remaining.

- [ ] **Step 5: Verify compilation + clippy + tests**

```bash
cargo check -p scrat-core
cargo clippy -p scrat-core --all-targets -- -D warnings
cargo nextest run -p scrat-core deps::
```
Expected: all clean, 59 tests pass. Critical: the `PythonLockfileParser.parse_diff(diff)` call path now goes `python.rs::parse_diff` → `RustLockfileParser.parse_diff(diff)` → `deps/rust.rs::parse_diff` (the real parser). If Python tests pass, the delegation chain works.

- [ ] **Step 6: Verify Python test provenance**

```bash
cargo nextest list -p scrat-core deps::python::
```
Expected: 3 test names (`parse_uv_lock_diff_update`, `parse_uv_lock_diff_added`, `parse_uv_lock_diff_skips_header`).

- [ ] **Step 7: Append sub-bullet to commit.txt**

```
* refactor(deps): extract PythonLockfileParser to deps/python.rs

PythonLockfileParser delegates to RustLockfileParser because uv.lock
currently uses the same TOML [[package]] format as Cargo.lock. The
module doc comment explicitly documents this as an incidental format
match, NOT a shared abstraction commitment — if uv diverges, this
module grows its own state machine.

Removes parse_uv_lock_diff (the 3-line delegation free function) in
favor of the trait-method delegation. The 3 parse_uv_lock_diff_*
tests move to deps::python::tests.

No behavior change. All 59 deps:: tests still pass (3 now in deps::python::).
```

- [ ] **Step 8: Stop — Clay may run gtxt or continue**

---

### Task 9: Extract `RubyLockfileParser` to `deps/ruby.rs`

**Files:**
- Create: `crates/scrat-core/src/deps/ruby.rs`
- Modify: `crates/scrat-core/src/deps/mod.rs`

Sixth extraction. Ruby uses `std::collections::HashMap` (hash-map collect-and-merge), not shared helpers. Lift the `use std::collections::HashMap;` to module level.

- [ ] **Step 1: Read `deps/mod.rs` to locate Ruby-specific code**

- `parse_gemfile_lock_diff` function
- `pub struct RubyLockfileParser;` + impl
- 7 `parse_gemfile_lock_diff_*` tests

- [ ] **Step 2: Create `deps/ruby.rs`**

```rust
//! Lockfile diff parser for Ruby's `Gemfile.lock`.
//!
//! Line-oriented collect-and-merge. Only matches lines with exactly 4
//! spaces of indent (top-level gems under `specs:`), ignoring
//! sub-dependency lines at 6+ spaces. Hash-map-based merge — not a
//! stateful parser.

use std::collections::HashMap;

use super::LockfileDiffParser;
use crate::pipeline::DepChange;

/// Lockfile diff parser for Ruby's `Gemfile.lock`.
pub struct RubyLockfileParser;

impl LockfileDiffParser for RubyLockfileParser {
    fn parse_diff(&self, diff: &str) -> Vec<DepChange> {
        let mut removed: HashMap<String, String> = HashMap::new();
        let mut added: HashMap<String, String> = HashMap::new();

        for line in diff.lines() {
            let (is_remove, is_add) = (line.starts_with('-'), line.starts_with('+'));
            if !is_remove && !is_add {
                continue;
            }

            let content = &line[1..];

            // Skip diff headers
            if content.starts_with("++") || content.starts_with("--") {
                continue;
            }

            // Must be exactly 4 spaces indent (top-level gem, not a sub-dep at 6+)
            if !content.starts_with("    ") || content.starts_with("      ") {
                continue;
            }

            let trimmed = content.trim();

            // Parse "gem-name (1.2.3)" or "gem-name (1.2.3.alpha)"
            if let Some((name, rest)) = trimmed.split_once(" (")
                && let Some(version) = rest.strip_suffix(')')
            {
                if is_remove {
                    removed.insert(name.to_string(), version.to_string());
                } else {
                    added.insert(name.to_string(), version.to_string());
                }
            }
        }

        let mut changes: Vec<DepChange> = Vec::new();

        for (name, old_ver) in &removed {
            if let Some(new_ver) = added.get(name) {
                if old_ver != new_ver {
                    changes.push(DepChange {
                        name: name.clone(),
                        from: Some(old_ver.clone()),
                        to: Some(new_ver.clone()),
                    });
                }
            } else {
                changes.push(DepChange {
                    name: name.clone(),
                    from: Some(old_ver.clone()),
                    to: None,
                });
            }
        }

        for (name, new_ver) in &added {
            if !removed.contains_key(name) {
                changes.push(DepChange {
                    name: name.clone(),
                    from: None,
                    to: Some(new_ver.clone()),
                });
            }
        }

        changes.sort_by(|a, b| a.name.cmp(&b.name));
        changes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_gemfile_lock_diff_update() {
        let diff = "\
-    rails (7.1.2)\n\
+    rails (7.1.3)";
        let changes = RubyLockfileParser.parse_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "rails");
        assert_eq!(changes[0].from.as_deref(), Some("7.1.2"));
        assert_eq!(changes[0].to.as_deref(), Some("7.1.3"));
    }

    #[test]
    fn parse_gemfile_lock_diff_added() {
        let diff = "+    new-gem (1.0.0)";
        let changes = RubyLockfileParser.parse_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "new-gem");
        assert_eq!(changes[0].from, None);
        assert_eq!(changes[0].to.as_deref(), Some("1.0.0"));
    }

    #[test]
    fn parse_gemfile_lock_diff_removed() {
        let diff = "-    old-gem (2.0.0)";
        let changes = RubyLockfileParser.parse_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].from.as_deref(), Some("2.0.0"));
        assert_eq!(changes[0].to, None);
    }

    #[test]
    fn parse_gemfile_lock_diff_ignores_subdeps() {
        // Sub-deps have 6+ spaces indent — must be ignored
        let diff = "\
-    rails (7.1.2)\n\
+    rails (7.1.3)\n\
-      actionpack (= 7.1.2)\n\
+      actionpack (= 7.1.3)";
        let changes = RubyLockfileParser.parse_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "rails");
    }

    #[test]
    fn parse_gemfile_lock_diff_mixed() {
        let diff = "\
-    rails (7.1.2)\n\
+    rails (7.1.3)\n\
+    new-gem (1.0.0)\n\
-    old-gem (2.0.0)";
        let changes = RubyLockfileParser.parse_diff(diff);
        assert_eq!(changes.len(), 3);
        assert_eq!(changes[0].name, "new-gem");
        assert_eq!(changes[1].name, "old-gem");
        assert_eq!(changes[2].name, "rails");
    }

    #[test]
    fn parse_gemfile_lock_diff_empty() {
        assert!(RubyLockfileParser.parse_diff("").is_empty());
    }

    #[test]
    fn parse_gemfile_lock_diff_prerelease() {
        let diff = "\
-    nokogiri (1.16.0.rc1)\n\
+    nokogiri (1.16.0)";
        let changes = RubyLockfileParser.parse_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].from.as_deref(), Some("1.16.0.rc1"));
    }
}
```

- [ ] **Step 3: Update `deps/mod.rs` — add `mod ruby;` + `pub use ruby::RubyLockfileParser;`**

- [ ] **Step 4: Delete Ruby-specific items from `deps/mod.rs`**

Remove `RubyLockfileParser` struct + impl, `fn parse_gemfile_lock_diff`, and the 7 `parse_gemfile_lock_diff_*` tests. After deletion, `deps/mod.rs::tests` has 18 − 7 = 11 tests remaining.

- [ ] **Step 5: Verify compilation + clippy + tests**

```bash
cargo check -p scrat-core
cargo clippy -p scrat-core --all-targets -- -D warnings
cargo nextest run -p scrat-core deps::
```
Expected: all clean, 59 tests pass.

- [ ] **Step 6: Verify Ruby test provenance**

```bash
cargo nextest list -p scrat-core deps::ruby::
```
Expected: 7 test names starting with `parse_gemfile_lock_diff_`.

- [ ] **Step 7: Append sub-bullet to commit.txt**

```
* refactor(deps): extract RubyLockfileParser to deps/ruby.rs

Moves parse_gemfile_lock_diff logic into RubyLockfileParser's impl
body and the 7 parse_gemfile_lock_diff_* tests into deps::ruby::tests.
Uses hash-map collect-and-merge with 4-space-indent detection; no
shared helpers needed.

No behavior change. All 59 deps:: tests still pass (7 now in deps::ruby::).
```

- [ ] **Step 8: Stop — Clay may run gtxt or continue**

---

### Task 10: Extract `SwiftLockfileParser` to `deps/swift.rs`

**Files:**
- Create: `crates/scrat-core/src/deps/swift.rs`
- Modify: `crates/scrat-core/src/deps/mod.rs`

Seventh and final extraction. Swift uses `emit_change` and `extract_json_string_value` (same shape as PHP, but keyed on `"identity"` rather than `"name"`).

- [ ] **Step 1: Read `deps/mod.rs` to locate Swift-specific code**

- `parse_package_resolved_diff` function
- `pub struct SwiftLockfileParser;` + impl
- 7 `parse_package_resolved_diff_*` tests

- [ ] **Step 2: Create `deps/swift.rs`**

```rust
//! Lockfile diff parser for Swift's `Package.resolved`.
//!
//! JSON state machine keyed on `"identity":` boundaries, same pattern
//! as [`super::php::PhpLockfileParser`] but using `"identity"` as the
//! package-key field instead of `"name"`.

use super::{LockfileDiffParser, emit_change, extract_json_string_value};
use crate::pipeline::DepChange;

/// Lockfile diff parser for Swift's `Package.resolved`.
pub struct SwiftLockfileParser;

impl LockfileDiffParser for SwiftLockfileParser {
    fn parse_diff(&self, diff: &str) -> Vec<DepChange> {
        let mut changes: Vec<DepChange> = Vec::new();

        let mut current_name: Option<String> = None;
        let mut old_version: Option<String> = None;
        let mut new_version: Option<String> = None;

        for line in diff.lines() {
            let trimmed = line
                .strip_prefix(' ')
                .or_else(|| line.strip_prefix('+'))
                .or_else(|| line.strip_prefix('-'))
                .unwrap_or(line)
                .trim();

            // "identity": boundary — emit pending, start new tracking
            if let Some(name) = extract_json_string_value(trimmed, "identity") {
                emit_change(&mut changes, &current_name, &old_version, &new_version);
                current_name = Some(name);
                old_version = None;
                new_version = None;
                continue;
            }

            // -"version": — old version
            if line.starts_with('-') {
                if let Some(ver) = extract_json_string_value(trimmed, "version") {
                    old_version = Some(ver);
                }
                continue;
            }

            // +"version": — new version
            if line.starts_with('+')
                && let Some(ver) = extract_json_string_value(trimmed, "version")
            {
                new_version = Some(ver);
            }
        }

        emit_change(&mut changes, &current_name, &old_version, &new_version);
        changes.sort_by(|a, b| a.name.cmp(&b.name));
        changes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_package_resolved_diff_update() {
        let diff = r#"
       "identity" : "swift-nio",
       "kind" : "remoteSourceControl",
       "state" : {
-        "version" : "2.92.0"
+        "version" : "2.92.1"
       }
"#;
        let changes = SwiftLockfileParser.parse_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "swift-nio");
        assert_eq!(changes[0].from.as_deref(), Some("2.92.0"));
        assert_eq!(changes[0].to.as_deref(), Some("2.92.1"));
    }

    #[test]
    fn parse_package_resolved_diff_added() {
        let diff = r#"
+      "identity" : "swift-log",
+      "state" : {
+        "version" : "1.5.4"
+      }
"#;
        let changes = SwiftLockfileParser.parse_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "swift-log");
        assert_eq!(changes[0].from, None);
        assert_eq!(changes[0].to.as_deref(), Some("1.5.4"));
    }

    #[test]
    fn parse_package_resolved_diff_removed() {
        let diff = r#"
-      "identity" : "old-package",
-      "state" : {
-        "version" : "1.0.0"
-      }
"#;
        let changes = SwiftLockfileParser.parse_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].from.as_deref(), Some("1.0.0"));
        assert_eq!(changes[0].to, None);
    }

    #[test]
    fn parse_package_resolved_diff_ignores_revision() {
        let diff = r#"
       "identity" : "swift-nio",
       "state" : {
-        "revision" : "abc123",
-        "version" : "2.92.0"
+        "revision" : "def456",
+        "version" : "2.92.1"
       }
"#;
        let changes = SwiftLockfileParser.parse_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].to.as_deref(), Some("2.92.1"));
    }

    #[test]
    fn parse_package_resolved_diff_ignores_file_version() {
        // File-level "version": 3 should not be emitted as a dep change
        let diff = r#"
-  "version" : 2
+  "version" : 3
       "identity" : "swift-nio",
-        "version" : "2.92.0"
+        "version" : "2.92.1"
"#;
        let changes = SwiftLockfileParser.parse_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "swift-nio");
    }

    #[test]
    fn parse_package_resolved_diff_mixed() {
        let diff = r#"
       "identity" : "updated-pkg",
-        "version" : "1.0.0"
+        "version" : "1.1.0"
+      "identity" : "new-pkg",
+        "version" : "0.1.0"
-      "identity" : "old-pkg",
-        "version" : "3.0.0"
"#;
        let changes = SwiftLockfileParser.parse_diff(diff);
        assert_eq!(changes.len(), 3);
        assert_eq!(changes[0].name, "new-pkg");
        assert_eq!(changes[1].name, "old-pkg");
        assert_eq!(changes[2].name, "updated-pkg");
    }

    #[test]
    fn parse_package_resolved_diff_empty() {
        assert!(SwiftLockfileParser.parse_diff("").is_empty());
    }
}
```

- [ ] **Step 3: Update `deps/mod.rs` — add `mod swift;` + `pub use swift::SwiftLockfileParser;`**

- [ ] **Step 4: Delete Swift-specific items from `deps/mod.rs`**

Remove `SwiftLockfileParser` struct + impl, `fn parse_package_resolved_diff`, and the 7 `parse_package_resolved_diff_*` tests. After deletion, `deps/mod.rs::tests` has 11 − 7 = 4 tests remaining — exactly the 4 shared-helper tests (`extract_toml_string_value_basic`, `extract_toml_string_value_no_match`, `extract_json_string_value_basic`, `extract_json_string_value_no_match`).

- [ ] **Step 5: Verify compilation + clippy + tests**

```bash
cargo check -p scrat-core
cargo clippy -p scrat-core --all-targets -- -D warnings
cargo nextest run -p scrat-core deps::
```
Expected: all clean, 59 tests pass. After this task, `deps/mod.rs::tests` contains only 4 shared-helper tests; the other 55 are distributed across 7 sibling files.

- [ ] **Step 6: Verify Swift test provenance**

```bash
cargo nextest list -p scrat-core deps::swift::
```
Expected: 7 test names starting with `parse_package_resolved_diff_`.

- [ ] **Step 7: Verify the final shared-helper test set in `deps::tests::`**

```bash
cargo nextest list -p scrat-core deps::tests::
```
Expected: exactly 4 test names:
- `deps::tests::extract_json_string_value_basic`
- `deps::tests::extract_json_string_value_no_match`
- `deps::tests::extract_toml_string_value_basic`
- `deps::tests::extract_toml_string_value_no_match`

If the list is not exactly these 4, a prior task either dropped a test or left an ecosystem-specific test behind. Investigate before proceeding to Task 11.

- [ ] **Step 8: Append sub-bullet to commit.txt**

```
* refactor(deps): extract SwiftLockfileParser to deps/swift.rs

Moves parse_package_resolved_diff logic into SwiftLockfileParser's
impl body and the 7 parse_package_resolved_diff_* tests into
deps::swift::tests. Imports emit_change and extract_json_string_value
from deps/mod.rs as shared helpers — same pattern as PhpLockfileParser,
keyed on "identity" instead of "name".

Final per-ecosystem extraction. After this task, deps/mod.rs::tests
contains only 4 shared-helper tests; the other 55 are distributed
across 7 sibling files.

No behavior change. All 59 deps:: tests still pass (7 now in deps::swift::).
```

- [ ] **Step 9: Stop — Clay may run gtxt or continue**

---

### Task 11: Final verification, cleanup, and PR

**Files:**
- Modify: `crates/scrat-core/src/deps/mod.rs` (possible final cleanup)
- Run: full workspace test suite (ONLY with Clay's explicit go-ahead)

This task closes out Phase 3 with cross-module verification, line count sanity checks, commit message finalization, and PR creation.

- [ ] **Step 1: Verify the final `deps/mod.rs` shape**

Read `crates/scrat-core/src/deps/mod.rs` and confirm it contains only:
1. Module doc comment (the `//!` block at the top)
2. Module imports (`use tracing`, `use crate::ecosystem`, etc.)
3. `mod <name>;` + `pub use <name>::<Lang>LockfileParser;` for all 7 ecosystems
4. The `LockfileDiffParser` trait definition
5. `compute_deps` function (public API)
6. `pub(super) fn emit_change`
7. `pub(super) fn extract_toml_string_value`
8. `pub(super) fn extract_json_string_value`
9. `#[cfg(test)] mod tests` with exactly 4 shared-helper tests

Anything else — any leftover `parse_*_diff` function, any leftover `pub struct <Lang>LockfileParser;`, any leftover per-ecosystem test — is a bug from a prior task. Fix before proceeding.

- [ ] **Step 2: Confirm final line counts**

Run:
```bash
wc -l crates/scrat-core/src/deps/mod.rs crates/scrat-core/src/deps/*.rs
```
Expected ballpark:
```
    250–300 crates/scrat-core/src/deps/mod.rs
    130–160 crates/scrat-core/src/deps/rust.rs
    220–260 crates/scrat-core/src/deps/node.rs
    180–220 crates/scrat-core/src/deps/go.rs
    140–170 crates/scrat-core/src/deps/php.rs
     60–90  crates/scrat-core/src/deps/python.rs
    160–200 crates/scrat-core/src/deps/ruby.rs
    130–160 crates/scrat-core/src/deps/swift.rs
   1270–1560 total
```
Do not block on exact numbers. Block only if `deps/mod.rs` is still above 400 LOC (means leftover parser logic) or any sibling file is under 50 LOC (means it's missing tests).

- [ ] **Step 3: Full deps::-scope test run**

Run (with `dangerouslyDisableSandbox: true`):
```bash
cargo nextest run -p scrat-core deps::
```
Expected: 59 tests pass. This is the final per-module gate.

- [ ] **Step 4: Verify test distribution across the 8 modules**

Run each in sequence:
```bash
cargo nextest list -p scrat-core deps::rust::    # expect 7
cargo nextest list -p scrat-core deps::node::    # expect 15
cargo nextest list -p scrat-core deps::go::      # expect 9
cargo nextest list -p scrat-core deps::php::     # expect 7
cargo nextest list -p scrat-core deps::python::  # expect 3
cargo nextest list -p scrat-core deps::ruby::    # expect 7
cargo nextest list -p scrat-core deps::swift::   # expect 7
cargo nextest list -p scrat-core deps::tests::   # expect 4
```

Total must equal 59. If any bucket is short or long, fix before proceeding.

- [ ] **Step 5: Ask Clay before running full workspace test suite**

The full workspace suite is multi-minute on this machine. Ask Clay:

> "Task 11 Step 5: all 59 deps::-scope tests pass and test distribution checks out (7/15/9/7/3/7/7/4). Ready to run `just test` on the full workspace, or should I skip ahead to the PR?"

If Clay says run it: execute `just test` with `dangerouslyDisableSandbox: true`. Expect ~580 tests to pass (same as Phase 2's baseline).

If Clay says skip: proceed to Step 6.

- [ ] **Step 6: Final commit.txt review**

Read `commit.txt` at the repo root. Verify:
1. It starts with `refactor(deps): extract per-ecosystem lockfile diff parsers into deps/`
2. The intro paragraph is a real complete sentence (NOT `[body to be appended as each task completes]` or any placeholder)
3. It contains sub-bullets for each of Tasks 2, 3, 4, 5, 6, 7, 8, 9, 10 — 9 sub-bullets total
4. Each sub-bullet has a `* refactor(deps): ...` header and a body paragraph at column 0 (not indented)
5. No sub-bullet contains `[body]`, `TBD`, `TODO`, or placeholder text

If Clay has run `gtxt` multiple times during execution, some sub-bullets may already be in prior commits. That's fine — just verify the CURRENT `commit.txt` (whatever remains after the last `gtxt`) has valid content for any un-committed sub-bullets.

- [ ] **Step 7: Ask Clay to run the final gtxt + git pm**

Ask Clay:

> "commit.txt verified. Ready for you to run `gtxt` for the final bundled commit, and then `git pm` to push the branch and open the PR."

Wait for Clay to run `gtxt` and `git pm`. Do NOT push or PR yourself.

- [ ] **Step 8: Post-PR code quality review dispatch**

Once Clay reports the PR is open (e.g., "PR #39 is up"), dispatch the final code-quality review subagent to audit the diff. This is the third and final code-quality review gate (after Task 4 and Task 5). The reviewer should check:
1. `LockfileDiffParser` trait definition matches the plan exactly
2. All 7 unit structs are `pub struct <Lang>LockfileParser;` (unit structs, not tuple structs)
3. Python's `deps/python.rs` has the module doc comment explaining incidental format match
4. Every sibling file has the same structural template (doc comment → imports → struct → impl → tests)
5. No leftover `parse_*_diff` free functions in `deps/mod.rs`
6. The 4 shared-helper tests in `deps/mod.rs::tests` are exactly the 4 expected names
7. No clippy warnings

Use `superpowers:code-reviewer` subagent. Provide the PR number and a link to this plan.

- [ ] **Step 9: Update project memory on Phase 3 completion**

After the PR merges, update `project_ecosystem_modules_refactor.md` in auto-memory to reflect Phase 3 complete. Specifically:
1. Mark Phase 3 as **COMPLETE** in the arc table with the merged PR number and squash commit
2. Add a "Phase 3 outcome" section documenting: final line counts, trait shape validated, Python delegation rationale
3. Update "Phase 4 starting conditions" with fresh context (preflight.rs LOC, unified-trait destination updated with observed `LockfileDiffParser` shape)

- [ ] **Step 10: Write Phase 3 completion handoff**

Write a handoff document at `.handoffs/2026-04-11-<HHMM>-ecosystem-modules-phase-3-complete.md` following the template from `.handoffs/2026-04-10-2349-ecosystem-modules-phase-2-complete.md`. Include:
1. Branch state (Green)
2. What shipped (PR number, squash commit, LOC shrinkage)
3. Decisions made (trait shape, Python delegation, sibling file template)
4. What's next (Phase 4 planning)
5. Landmines for the Phase 4 planner
6. Pointers (plan doc, PR, squash commit, phase-1/2 handoffs)

---

## Execution handoff

**Plan complete and saved to `record/superpowers/plans/2026-04-11-ecosystem-modules-phase-3-deps.md`. Two execution options:**

**1. Subagent-Driven (recommended)** — Dispatch a fresh subagent per task with two-stage review. Reuse Phase 1 and Phase 2's validated review optimization: spec review every task; code-quality review on Task 4 (first extraction / template validator), Task 5 (Node, the biggest extraction), and Task 11 (final PR). Pattern validated on two consecutive phases, both merged clean on first try.

**2. Inline Execution** — Execute tasks in this session using `superpowers:executing-plans`, with checkpoints for review.

**Which approach?**

**If Subagent-Driven chosen:**
- **REQUIRED SUB-SKILL:** Use `superpowers:subagent-driven-development`
- Fresh subagent per task + two-stage review on Tasks 4, 5, 11

**If Inline Execution chosen:**
- **REQUIRED SUB-SKILL:** Use `superpowers:executing-plans`
- Batch execution with checkpoints for review

---

## Landmines

- **The `parse_package_lock_diff` "stub" is stale memory.** The handoff and auto-memory describe Node's parser as "currently stubbed." Reading the code shows it's a full parser against npm lockfile v2/v3 top-level entries. Do NOT "fix" or expand it during extraction — move as-is.
- **Python delegation is load-bearing.** Do NOT extract a shared `parse_toml_package_diff` helper on the assumption that Rust and Python should share one. Clay explicitly rejected this. The module doc comment in `deps/python.rs` exists specifically to prevent this drift in future phases.
- **`commit.txt` APPEND, never overwrite.** 9 sub-bullets accumulate across Tasks 2–10. Use the Edit tool to append to `commit.txt` when it exists. Re-create the header + subject + intro paragraph ONLY if `gtxt` has deleted `commit.txt` since the last append. When creating, use a REAL intro paragraph (not `[body to be appended...]`).
- **The Task 3 atomic refactor must land together.** Trait introduction, 7 unit struct impls, and dispatch rewrite are interlocking. Splitting them produces either unused types (clippy warns) or a broken dispatch. Keep Task 3's steps together in a single commit.
- **Node's private helpers move WITH Node.** `extract_top_level_node_modules_name` and `extract_json_version` are exclusive to the package-lock.json parser. They go into `deps/node.rs` as private functions, NOT into `deps/mod.rs` as shared helpers.
- **Ruby and Go do NOT use shared helpers.** Their parsers are hash-map collect-and-merge, not state machines calling `emit_change`. Do NOT add unnecessary `use super::emit_change;` imports to `deps/ruby.rs` or `deps/go.rs`.
- **The test count is 59 starting, 59 ending.** If any task reports a different number, STOP and investigate before proceeding. A missing test is a dropped-on-the-floor regression, not a cosmetic issue.
- **Shared helper visibility must be `pub(super)`, not `pub`.** `emit_change`, `extract_toml_string_value`, `extract_json_string_value` are internal to the `deps` module tree. Making them `pub` leaks them to the rest of scrat-core.
- **Narrow test scope per task.** `cargo nextest run -p scrat-core deps::` = 59 tests, sub-second. Do not run the full workspace suite except at Task 11 Step 5 (with Clay's explicit go-ahead).
- **Sandbox flag on every cargo invocation.** `dangerouslyDisableSandbox: true` is required for sccache. Forgetting it causes cryptic compile errors.
- **If clippy warns on `clippy::unused_self` on trait impl methods**, add `#[allow(clippy::unused_self)]` on the trait definition with a comment: "Phase 4 will attach state to per-ecosystem drivers; `&self` preserves the ABI across phases." Do NOT remove `&self` from the trait method.
- **Phase 2's `[body to be appended as each task completes]` placeholder leaked into merged commit `bbdd2ab`.** For Phase 3, the Task 2 Step 7 commit.txt skeleton uses a real intro paragraph. Verify at Task 11 Step 6 that no placeholder text made it into the final commit message.

---

## Success criteria

Phase 3 is complete when:
1. ✅ Branch `refactor/ecosystem-modules-phase-3` is merged to main via PR
2. ✅ `deps/mod.rs` is between 250 and 300 LOC (down from 1343)
3. ✅ 7 sibling files exist: `deps/{rust,node,go,php,python,ruby,swift}.rs`
4. ✅ `LockfileDiffParser` trait is defined in `deps/mod.rs` with `&self, diff: &str -> Vec<DepChange>` signature
5. ✅ 7 unit structs (`<Lang>LockfileParser`) implement `LockfileDiffParser`
6. ✅ `compute_deps` dispatch uses trait method calls, not free function calls
7. ✅ Python delegates to `RustLockfileParser` with a module doc comment explaining the rationale
8. ✅ All 59 deps tests pass, distributed 7/15/9/7/3/7/7/4 across the 8 modules
9. ✅ `cargo clippy -p scrat-core --all-targets -- -D warnings` is clean
10. ✅ `compute_deps`'s public signature is unchanged
11. ✅ `lib.rs` has zero changes
12. ✅ Phase 3 completion handoff exists at `.handoffs/2026-04-11-<HHMM>-ecosystem-modules-phase-3-complete.md`
13. ✅ `project_ecosystem_modules_refactor.md` in auto-memory reflects Phase 3 as complete

---

## Self-review notes (for the plan author, before dispatch)

These are reminders from writing this plan. Delete this section before dispatching to workers if it clutters the document.

- **Spec coverage:** Every decision from the brainstorming session is reflected in at least one task: `&self` trait shape (Task 3 Step 2), Python literal delegation (Task 8 Step 2), `{Lang}LockfileParser` naming (used throughout), shared helpers in `deps/mod.rs` as `pub(super)` (Task 4 Step 5), Node's private helpers travel with Node (Task 5 Step 2), Generic stays as dispatch arm (Task 3 Step 3).
- **Placeholder scan:** Search for `TBD`, `TODO`, `[body`, `fill in`, `similar to Task N` — none should appear in the task bodies. The Task 2 intro paragraph is a REAL complete sentence, verified against the Phase 2 cosmetic bug.
- **Type consistency:** All struct names are `<Lang>LockfileParser` (Rust, Node, Go, Php, Python, Ruby, Swift). All trait method calls use `.parse_diff(diff)` — never `parse_diff(&self, diff)` or `.parse_lockfile(...)`. Dispatch match arms align on `=>`.
- **Test count:** 7 + 15 + 9 + 7 + 3 + 7 + 7 + 4 = 59. Verified against live nextest output at plan-writing time.
