# Ecosystem Modules Refactor — Phase 2: Extract `bump/`

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extract the five per-ecosystem version-bump helpers (`bump_rust_version`, `bump_node_version`, `bump_composer_version`, `bump_pyproject_version`, `bump_ruby_version` + Ruby's four supporting helpers) from `crates/scrat-core/src/bump.rs` into sibling files under a new `crates/scrat-core/src/bump/` module directory. Harmonize their return types to `BumpResult<Vec<String>>` so the dispatch in `ReadyBump::execute` becomes uniform. No trait yet.

**Architecture:** Phase 2 of a four-phase refactor that will eventually collapse the ecosystem-by-ecosystem scatter across scrat-core's monster files (`bump.rs` 1644 LOC, `deps.rs` 1343 LOC, `preflight.rs` 1641 LOC) into a single `crates/scrat-core/src/ecosystem/<name>.rs` module tree implementing a unified `EcosystemDriver` trait. Phase 2 is pure restructuring of `bump/` — it harmonizes the per-ecosystem helper return type to `Vec<String>` (matching Ruby's existing shape and `BumpOutcome.modified_files`) and then moves each helper into its own sibling file. The file-per-ecosystem pattern validated in Phase 1 (PR #37) is reused verbatim. No trait introduction in this phase — that's Phase 4's job, and it will be designed from observed usage, not on a whiteboard.

**Tech Stack:** Rust (scrat-core library crate). No new dependencies.

---

## The full arc (context, not in scope for this plan)

| Phase | Goal | Output | Status |
|-------|------|--------|--------|
| **1** | Finish `detect/` split | `detect/{rust,node,go,php,python,ruby,swift}.rs` + normalized `build_detection_for` dispatch | **COMPLETE** — merged 2026-04-10 as PR #37 (squash commit `0765242`) |
| **2 (this plan)** | Extract `bump/` with harmonized `Vec<String>` return type | `bump/<name>.rs` per ecosystem (rust, node, php, python, ruby); uniform execute() dispatch | **THIS PLAN** |
| **3** | Extract `deps/` with `LockfileDiffParser` trait | `deps/<name>.rs` per ecosystem | Planned |
| **4** | Unify into `ecosystem/<name>.rs` with single `EcosystemDriver` trait | Single file per ecosystem implementing the unified trait; `bump/`, `deps/`, `detect/` directories collapsed into `ecosystem/` | Planned |

Phases 3–4 will be planned as separate documents after each phase completes and we have observed how the abstraction feels under real use. The destination trait sketch — for posterity:

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

The reasoning for the incremental approach: traits designed on a whiteboard tend to over-specify (every method required whether it makes sense per-ecosystem or not) or under-specify (missing escape hatches — e.g., Ruby's multi-file walker, Rust's tool-invocation path). Designing from observed usage is safer.

---

## Out of scope for Phase 2

- **Any behavior change.** Every per-ecosystem helper's logic is moved verbatim after the return type is harmonized in Task 3.
- **Introducing a `VersionBumper` trait.** Phase 2 discovers what the trait should look like through extraction; it does not write one. The trait is Phase 4's work.
- **Touching `deps.rs`, `preflight.rs`.** Those are Phases 3–4.
- **Normalizing helper signatures.** `bump_rust_version` takes `&ProjectDetection`, the others don't. That asymmetry stays — Phase 4 will harmonize signatures as part of trait introduction.
- **Adding new tests.** The existing `bump::tests` module has ~44 tests that cover the public API (plus 19 Ruby-specific private-helper tests). Those are the refactor safety net. Per-ecosystem test expansion is deferred.
- **Renaming functions.** `bump_rust_version`, `bump_node_version`, etc. keep their current names. Shortening to `bump_rust` etc. is scope creep for this phase.
- **Changing the `BumpOutcome`, `BumpError`, `BumpPlan`, `ReadyBump`, `InteractiveBump` public API.** Phase 2 is strictly backward compatible at the module boundary.
- **Touching `crates/scrat-core/src/lib.rs`.** The `pub mod bump;` declaration at line 38 resolves to either `bump.rs` OR `bump/mod.rs` automatically. Zero changes needed.

---

## File structure after Phase 2

```
crates/scrat-core/src/
├── bump/
│   ├── mod.rs        # coordinator: plan_bump, plan_bump_with_detection,
│   │                 #   resolve_interactive, resolve_strategy, current_or_zero,
│   │                 #   BumpPlan/ReadyBump/InteractiveBump/BumpOutcome/BumpError,
│   │                 #   ReadyBump::execute (with harmonized dispatch),
│   │                 #   generate_changelog, public-API tests (~25 tests)
│   ├── rust.rs       # bump_rust_version (~55 LOC)
│   ├── node.rs       # bump_node_version (~55 LOC)
│   ├── php.rs        # bump_composer_version (~55 LOC)
│   ├── python.rs     # bump_pyproject_version (~65 LOC)
│   └── ruby.rs       # bump_ruby_version + update_ruby_version_file +
│                     #   update_gemspec_version_file + replace_ruby_version_line +
│                     #   replace_gemspec_version_line + 19 private-helper tests
│                     #   (~510 LOC)
└── ...
```

After extraction, `bump/mod.rs` should be ~1010 LOC (down from 1644). Most of that is the public-API test module (~500 LOC of tests that exercise `execute()`, `plan_bump()`, `resolve_interactive()`, error displays, and `BumpOutcome` serialization through the public surface). The coordinator logic itself is ~350 LOC.

---

## Conventions used in this plan

- **Commits via `commit.txt` — APPEND, do not overwrite:** Each task's commit step APPENDS a sub-bullet section to `commit.txt` at the repo root. It does **not** overwrite the existing file. Clay runs `gtxt` (alias: `git commit -F commit.txt && rm commit.txt`) periodically — sometimes after every task, sometimes after batching several. When `gtxt` runs, the entire accumulated `commit.txt` becomes one bundled commit, and the file is deleted. The next task that produces a commit must re-create the skeleton. The worker does **not** run `git commit` directly.
- **Bundled-commit format:** `commit.txt` follows the structure used in `f706dc9` and `974deb4` — one top-level subject line, a brief opening body paragraph, then multiple `* type(scope): subject` sub-bullet sections (each with its own body paragraph at column 0, NOT indented under the `*`). Each task contributes one sub-bullet section.
- **Test cadence:** Full workspace test runs are slow on this machine. Each task runs `cargo check -p scrat-core` and `cargo clippy -p scrat-core --all-targets -- -D warnings` (both fast). Running `cargo nextest run -p scrat-core bump::` is fine — the bump test module has ~44 tests and runs in under a second. Running the full workspace suite requires asking Clay first.
- **Branch:** `refactor/ecosystem-modules-phase-2`. One branch, several bundled commits via `gtxt`, one PR at the end.
- **Cargo sandbox flag:** Every `cargo`, `cargo nextest`, and `just` invocation in the Bash tool must use `dangerouslyDisableSandbox: true` because sccache fails under sandbox mode. This is not optional.
- **Module-level `use` imports only.** Every sibling file (`bump/rust.rs`, etc.) places all `use` statements at module level, never inside function bodies. This lesson carried over from Phase 1. The only exception is inline `use` inside `#[cfg(test)] mod tests` functions, which is fine.
- **`use semver::Version;` (direct):** The `semver` crate is a direct dep of scrat-core (Cargo.toml:37) AND re-exported at lib.rs:85. Both `semver::Version` and `crate::semver::Version` compile inside scrat-core, but the direct import is shorter and matches the existing `bump.rs` import at line 18.

---

### Task 1: Branch setup and baseline verification

**Files:** none (git + verification only)

- [ ] **Step 1: Create feature branch**

Run:
```bash
git checkout -b refactor/ecosystem-modules-phase-2
```

Expected: `Switched to a new branch 'refactor/ecosystem-modules-phase-2'`.

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

- [ ] **Step 4: Record baseline bump-module test count**

Run (with `dangerouslyDisableSandbox: true`):
```bash
cargo nextest run -p scrat-core bump::
```
Expected: all tests pass. Record the exact number — you will compare against this at Task 9.

- [ ] **Step 5: Confirm starting line counts**

Run:
```bash
wc -l crates/scrat-core/src/bump.rs
```
Expected:
```
    1644 crates/scrat-core/src/bump.rs
```
Record this. Task 9 will verify `bump/mod.rs` has shrunk to roughly the 950–1050 line range.

---

### Task 2: Convert `bump.rs` to `bump/mod.rs` (file move only, no code changes)

**Files:**
- Move: `crates/scrat-core/src/bump.rs` → `crates/scrat-core/src/bump/mod.rs`

This task is purely a filesystem restructure. No content changes, no behavior changes, no dispatch changes. The `pub mod bump;` declaration at `crates/scrat-core/src/lib.rs:38` resolves to either `bump.rs` OR `bump/mod.rs` — Rust's module system picks up whichever exists. After this task, `bump.rs` no longer exists and `bump/mod.rs` contains the verbatim 1644 lines.

This task is separated from Task 3 (the return-type harmonization) so that the file move commits cleanly as a rename in git, preserving blame history. A combined "move + edit" commit would register as a delete + add with rename heuristics.

- [ ] **Step 1: Create the `bump/` directory**

Run:
```bash
mkdir crates/scrat-core/src/bump
```
Expected: directory created silently.

- [ ] **Step 2: Move `bump.rs` into the new directory as `mod.rs`**

Run:
```bash
git mv crates/scrat-core/src/bump.rs crates/scrat-core/src/bump/mod.rs
```
Expected: `git mv` runs silently. `git status` should now show:
```
renamed:    crates/scrat-core/src/bump.rs -> crates/scrat-core/src/bump/mod.rs
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
cargo nextest run -p scrat-core bump::
```
Expected: same test count as Task 1 Step 4, all passing.

- [ ] **Step 6: Write commit.txt**

Create `commit.txt` at the repo root with this content:

```
refactor(bump): extract per-ecosystem version bump helpers into bump/

[body to be appended as each task completes]

* refactor(bump): convert bump.rs to bump/mod.rs

Pure filesystem restructure. git mv preserves blame history. The
`pub mod bump;` declaration in lib.rs:38 resolves to bump/mod.rs
automatically — no import path changes needed anywhere in the
workspace.

No behavior change. All bump::tests still pass.
```

- [ ] **Step 7: Commit via gtxt**

Clay will run `gtxt` to consume `commit.txt`. Wait for that to happen before proceeding to Task 3.

---

### Task 3: Harmonize per-ecosystem helper return types to `BumpResult<Vec<String>>`

**Files:**
- Modify: `crates/scrat-core/src/bump/mod.rs`

This task is the only non-mechanical change in Phase 2. Before any helpers are extracted into sibling files, their return types are normalized so that Tasks 4–8 can be pure mechanical moves. Four helpers change:

| Helper | Before | After |
|---|---|---|
| `bump_rust_version` | `BumpResult<()>` | `BumpResult<Vec<String>>` — returns `vec!["Cargo.toml".into()]` on success |
| `bump_node_version` | `BumpResult<bool>` | `BumpResult<Vec<String>>` — returns `vec!["package.json".into()]` or `vec![]` |
| `bump_composer_version` | `BumpResult<bool>` | `BumpResult<Vec<String>>` — returns `vec!["composer.json".into()]` or `vec![]` |
| `bump_pyproject_version` | `BumpResult<bool>` | `BumpResult<Vec<String>>` — returns `vec!["pyproject.toml".into()]` or `vec![]` |

`bump_ruby_version` is **unchanged** — it already returns `BumpResult<Vec<String>>`.

The dispatch inside `ReadyBump::execute` is rewritten so every arm follows the same shape: call the helper, extend `modified_files`. Silent-skip and error policies (PHP/Python debug-log on empty, Ruby error on empty + no version_files) stay in the dispatch because they're inherently per-ecosystem policy decisions that consume non-ecosystem state (`self.version_files`).

After this task, every per-ecosystem helper returns `BumpResult<Vec<String>>` and the dispatch is uniform enough that Tasks 4–8 become pure mechanical file moves.

- [ ] **Step 1: Apply the atomic refactor to `bump/mod.rs`**

Read the current `crates/scrat-core/src/bump/mod.rs`. Make the following changes together (they interlock — splitting them produces intermediate states that don't compile).

**1a. Change `bump_rust_version` signature and return value.** Find the function (around line 362). Replace:

```rust
/// Bump the version in Cargo.toml using `cargo set-version`.
fn bump_rust_version(
    project_root: &Utf8Path,
    version: &Version,
    detection: &ProjectDetection,
) -> BumpResult<()> {
    let Some(ref bump_cmd) = detection.tools.bump_cmd else {
        return Err(BumpError::NoBumpTool);
    };

    debug!(%bump_cmd, %version, "bumping Rust version");

    let parts: Vec<&str> = bump_cmd.split_whitespace().collect();
    let (bin, args) = parts.split_first().unwrap_or((&"cargo", &[]));

    let output = Command::new(bin)
        .args(args)
        .arg(version.to_string())
        .current_dir(project_root.as_std_path())
        .output()
        .map_err(|e| BumpError::ToolFailed {
            tool: bump_cmd.clone(),
            message: format!("failed to execute: {e}"),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(BumpError::ToolFailed {
            tool: bump_cmd.clone(),
            message: stderr,
        });
    }

    Ok(())
}
```
with:
```rust
/// Bump the version in Cargo.toml using `cargo set-version`.
///
/// Returns the repo-relative path of the file that was updated.
fn bump_rust_version(
    project_root: &Utf8Path,
    version: &Version,
    detection: &ProjectDetection,
) -> BumpResult<Vec<String>> {
    let Some(ref bump_cmd) = detection.tools.bump_cmd else {
        return Err(BumpError::NoBumpTool);
    };

    debug!(%bump_cmd, %version, "bumping Rust version");

    let parts: Vec<&str> = bump_cmd.split_whitespace().collect();
    let (bin, args) = parts.split_first().unwrap_or((&"cargo", &[]));

    let output = Command::new(bin)
        .args(args)
        .arg(version.to_string())
        .current_dir(project_root.as_std_path())
        .output()
        .map_err(|e| BumpError::ToolFailed {
            tool: bump_cmd.clone(),
            message: format!("failed to execute: {e}"),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(BumpError::ToolFailed {
            tool: bump_cmd.clone(),
            message: stderr,
        });
    }

    Ok(vec!["Cargo.toml".into()])
}
```

**1b. Change `bump_node_version` signature and return value.** Find the function (around line 406). Replace:

```rust
/// Bump the version in `package.json` directly.
///
/// scrat edits only `package.json` — it is intentionally *not* a
/// lockfile manager. If the user needs `package-lock.json` (or
/// `yarn.lock`, `pnpm-lock.yaml`) synced after the bump, that's their
/// package manager's job (e.g. a pre-commit scrat hook running
/// `npm install --package-lock-only`).
///
/// Returns `true` if the file was modified.
fn bump_node_version(project_root: &Utf8Path, version: &Version) -> BumpResult<bool> {
```
with:
```rust
/// Bump the version in `package.json` directly.
///
/// scrat edits only `package.json` — it is intentionally *not* a
/// lockfile manager. If the user needs `package-lock.json` (or
/// `yarn.lock`, `pnpm-lock.yaml`) synced after the bump, that's their
/// package manager's job (e.g. a pre-commit scrat hook running
/// `npm install --package-lock-only`).
///
/// Returns the repo-relative path of the file that was updated.
fn bump_node_version(project_root: &Utf8Path, version: &Version) -> BumpResult<Vec<String>> {
```

Then find the last line of the function body:
```rust
    debug!(%version, "bumped package.json version");
    Ok(true)
}
```
and replace with:
```rust
    debug!(%version, "bumped package.json version");
    Ok(vec!["package.json".into()])
}
```

**1c. Change `bump_composer_version` signature and return value.** Find the function (around line 446). Replace the signature and doc comment:

```rust
/// Bump the version in `composer.json` if it has a `"version"` field.
///
/// Returns `true` if the file was modified, `false` if no version field exists.
fn bump_composer_version(project_root: &Utf8Path, version: &Version) -> BumpResult<bool> {
```
with:
```rust
/// Bump the version in `composer.json` if it has a `"version"` field.
///
/// Returns the repo-relative path of the file that was updated, or an
/// empty vec if `composer.json` is missing or has no `version` field.
fn bump_composer_version(project_root: &Utf8Path, version: &Version) -> BumpResult<Vec<String>> {
```

Then update the three return paths:
- Replace `Err(_) => return Ok(false),` with `Err(_) => return Ok(vec![]),`
- Replace the `if parsed.get("version").and_then(|v| v.as_str()).is_none() { return Ok(false); }` block's `return Ok(false);` with `return Ok(vec![]);`
- Replace the final `Ok(true)` with `Ok(vec!["composer.json".into()])`

**1d. Change `bump_pyproject_version` signature and return value.** Find the function (around line 795). Replace the signature and doc comment:

```rust
/// Bump the version in `pyproject.toml` if it has a `version` field under `[project]`.
///
/// Returns `true` if the file was modified, `false` if no version field exists.
fn bump_pyproject_version(project_root: &Utf8Path, version: &Version) -> BumpResult<bool> {
```
with:
```rust
/// Bump the version in `pyproject.toml` if it has a `version` field under `[project]`.
///
/// Returns the repo-relative path of the file that was updated, or an
/// empty vec if `pyproject.toml` is missing or has no `[project] version` field.
fn bump_pyproject_version(project_root: &Utf8Path, version: &Version) -> BumpResult<Vec<String>> {
```

Then update the return paths:
- Replace `Err(_) => return Ok(false),` with `Err(_) => return Ok(vec![]),`
- Replace `if !found { return Ok(false); }` with `if !found { return Ok(vec![]); }`
- Replace the final `Ok(true)` with `Ok(vec!["pyproject.toml".into()])`

**1e. Rewrite the `ReadyBump::execute` dispatch.** Find the dispatch block (starts around line 271 with `// Update version in project files`). Replace the entire `match self.detection.ecosystem { ... }` block with this uniform version:

```rust
        // Update version in project files (Generic has no project files to update)
        match self.detection.ecosystem {
            Ecosystem::Rust => {
                let files = bump_rust_version(project_root, &self.next, &self.detection)?;
                modified_files.extend(files);
            }
            Ecosystem::Node => {
                let files = bump_node_version(project_root, &self.next)?;
                modified_files.extend(files);
            }
            Ecosystem::Go | Ecosystem::Swift => {
                debug!(%self.detection.ecosystem, "version lives in git tags, no file to bump");
            }
            Ecosystem::Php => {
                let files = bump_composer_version(project_root, &self.next)?;
                if files.is_empty() {
                    debug!("composer.json has no version field, skipping");
                }
                modified_files.extend(files);
            }
            Ecosystem::Python => {
                let files = bump_pyproject_version(project_root, &self.next)?;
                if files.is_empty() {
                    debug!("pyproject.toml has no version field, skipping");
                }
                modified_files.extend(files);
            }
            Ecosystem::Ruby => {
                let files = bump_ruby_version(project_root, &self.next)?;
                if files.is_empty() && self.version_files.is_empty() {
                    return Err(BumpError::ToolFailed {
                        tool: "ruby".into(),
                        message: "no lib/**/version.rb or gemspec with a literal version \
                                  was found, and no `[[version_files]]` entries are \
                                  configured — the release would be tagged without \
                                  updating any file"
                            .into(),
                    });
                }
                modified_files.extend(files);
            }
            Ecosystem::Generic => {
                debug!("generic ecosystem — no project files to bump");
            }
        }
```

Notes on the dispatch:
- Rust is the only arm that passes `&self.detection` because `bump_rust_version` needs `detection.tools.bump_cmd`. The asymmetry stays — Phase 4 will harmonize signatures as part of trait introduction.
- PHP and Python debug-log on empty — unchanged policy, just moved out of the helper's `if bumped` check into the dispatch's `if files.is_empty()` check.
- Ruby's error-on-empty-and-no-version-files check moves from `files.is_empty()` on a `Vec<String>` return (which already worked) to the same check — unchanged, just kept in the same shape as the others.
- `Ecosystem::Go | Ecosystem::Swift` still short-circuits with a debug log; there are no bump helpers for Go or Swift because their versions live entirely in git tags.
- `Ecosystem::Generic` still short-circuits with a debug log; Generic has no project files to update.

- [ ] **Step 2: Verify compilation**

Run (with `dangerouslyDisableSandbox: true`):
```bash
cargo check -p scrat-core
```
Expected: clean build, no errors. If this fails, one of the substeps (1a–1e) is incomplete or inconsistent.

- [ ] **Step 3: Verify clippy is clean**

Run (with `dangerouslyDisableSandbox: true`):
```bash
cargo clippy -p scrat-core --all-targets -- -D warnings
```
Expected: no warnings.

- [ ] **Step 4: Run bump tests**

Run (with `dangerouslyDisableSandbox: true`):
```bash
cargo nextest run -p scrat-core bump::
```
Expected: same test count as Task 1 Step 4, all passing. The existing tests cover the behavior: `execute_node_updates_package_json_only` still asserts `outcome.modified_files == vec!["package.json"]`, and after the harmonization the dispatch still extends `modified_files` with exactly that value. `execute_rust_no_bump_tool_returns_error` still expects `BumpError::NoBumpTool`. No test edits are needed.

- [ ] **Step 5: Append to commit.txt**

Read the current `commit.txt` (it exists from Task 2 and has Clay's gtxt-run state — if `gtxt` ran after Task 2, this file does NOT exist and the skeleton must be recreated; if `gtxt` has not run yet, it still contains Task 2's sub-bullet).

**If `commit.txt` exists from Task 2**, use the Edit tool to APPEND a new sub-bullet section. Find the last line of Task 2's sub-bullet (`No behavior change. All bump::tests still pass.`) and replace it with:

```
No behavior change. All bump::tests still pass.

* refactor(bump): harmonize per-ecosystem bump helper return types

Normalize bump_rust_version, bump_node_version, bump_composer_version,
and bump_pyproject_version to all return BumpResult<Vec<String>> —
matching the existing shape of bump_ruby_version and the overall
BumpOutcome.modified_files field type.

Rewrite the ReadyBump::execute dispatch so every ecosystem arm follows
the same `let files = helper(...)?; modified_files.extend(files);`
pattern. PHP/Python keep their silent-skip behavior when no version
field is present; Ruby keeps its error-when-no-files-and-no-
version_files policy check. Go/Swift/Generic remain short-circuits.

Preparatory step for extracting each ecosystem's bump helper into its
own sibling file (bump/rust.rs, bump/node.rs, etc.) — part of phase 2
of the ecosystem-modules refactor.

No behavior change. All bump::tests still pass.
```

**If `commit.txt` does not exist** (Clay ran `gtxt` after Task 2), create it fresh with the Write tool:

```
refactor(bump): harmonize per-ecosystem bump helper return types

Part of phase 2 of the ecosystem-modules refactor. Before mechanical
extraction into sibling files, normalize the per-ecosystem bump
helpers to a uniform return shape.

* refactor(bump): harmonize per-ecosystem bump helper return types

Normalize bump_rust_version, bump_node_version, bump_composer_version,
and bump_pyproject_version to all return BumpResult<Vec<String>> —
matching the existing shape of bump_ruby_version and the overall
BumpOutcome.modified_files field type.

Rewrite the ReadyBump::execute dispatch so every ecosystem arm follows
the same `let files = helper(...)?; modified_files.extend(files);`
pattern. PHP/Python keep their silent-skip behavior when no version
field is present; Ruby keeps its error-when-no-files-and-no-
version_files policy check. Go/Swift/Generic remain short-circuits.

No behavior change. All bump::tests still pass.
```

- [ ] **Step 6: Commit via gtxt**

Clay runs `gtxt`. Wait before Task 4.

---

### Task 4: Extract `bump_rust_version` → `bump/rust.rs`

**Files:**
- Create: `crates/scrat-core/src/bump/rust.rs`
- Modify: `crates/scrat-core/src/bump/mod.rs`

**First repetition task.** The template validated here gets carried into Tasks 5–8. Code-quality review runs on this task specifically to catch template drift.

Mechanical move. The body of `bump_rust_version` (post-harmonization from Task 3) is copied verbatim into a new file. `bump/mod.rs` loses the local definition and gains a `mod rust;` declaration plus a `rust::bump_rust_version(...)` dispatch call.

- [ ] **Step 1: Create `crates/scrat-core/src/bump/rust.rs`**

Create the file with this exact content:

```rust
//! Rust ecosystem version bumping.
//!
//! Bumps `Cargo.toml` via whichever `bump_cmd` was detected (typically
//! `cargo set-version` from the `cargo-edit` extension). The tool
//! invocation and stderr propagation are handled here; `bump/mod.rs`
//! owns the dispatch and result aggregation.

use std::process::Command;

use camino::Utf8Path;
use semver::Version;
use tracing::debug;

use super::{BumpError, BumpResult};
use crate::ecosystem::ProjectDetection;

/// Bump the version in Cargo.toml using `cargo set-version`.
///
/// Returns the repo-relative path of the file that was updated.
pub(super) fn bump_rust_version(
    project_root: &Utf8Path,
    version: &Version,
    detection: &ProjectDetection,
) -> BumpResult<Vec<String>> {
    let Some(ref bump_cmd) = detection.tools.bump_cmd else {
        return Err(BumpError::NoBumpTool);
    };

    debug!(%bump_cmd, %version, "bumping Rust version");

    let parts: Vec<&str> = bump_cmd.split_whitespace().collect();
    let (bin, args) = parts.split_first().unwrap_or((&"cargo", &[]));

    let output = Command::new(bin)
        .args(args)
        .arg(version.to_string())
        .current_dir(project_root.as_std_path())
        .output()
        .map_err(|e| BumpError::ToolFailed {
            tool: bump_cmd.clone(),
            message: format!("failed to execute: {e}"),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(BumpError::ToolFailed {
            tool: bump_cmd.clone(),
            message: stderr,
        });
    }

    Ok(vec!["Cargo.toml".into()])
}
```

Template notes (carry forward to Tasks 5–8):
- Module-level doc comment starts with `//! <Ecosystem name> ecosystem version bumping.` and a 2–3 sentence description.
- Imports are grouped: `std::*` first, then external crates (`camino`, `semver`, `tracing`), then parent/crate imports (`super::*`, `crate::*`). One blank line between groups.
- `use super::{BumpError, BumpResult};` — lift these from the parent `bump/mod.rs` module. Not every ecosystem file will import both; Node/PHP/Python only need `BumpError` for tool-failure errors and `BumpResult` for the return type.
- `use crate::ecosystem::ProjectDetection;` — **rust.rs is the only sibling file that needs this import** because it's the only helper that takes `&ProjectDetection`. Do not add it to the other sibling files.
- Function visibility is `pub(super)` — visible to the parent `bump` module, invisible to the rest of the crate.
- Function signature and body are VERBATIM from `bump/mod.rs` after Task 3. No normalization, no rename, no cleanup.
- Leave a blank line before the `Ok(vec!["..."])` return (matches the existing format in bump/mod.rs).

- [ ] **Step 2: Add `mod rust;` to `bump/mod.rs`**

Near the top of `crates/scrat-core/src/bump/mod.rs` (after the module-level doc comment and the `use` block), add the module declaration. Since this is the first sibling file, there will be only one `mod` declaration for now:

```rust
mod rust;
```

Place it immediately after the `use` statements and before any type definitions (around the existing `// ──────────────────────────────────────────────` divider for Errors). Module declarations must be in alphabetical order as more sibling files are added in Tasks 5–8.

- [ ] **Step 3: Delete the local `bump_rust_version` function from `bump/mod.rs`**

Remove the entire `fn bump_rust_version(...) -> BumpResult<Vec<String>> { ... }` function body from `bump/mod.rs`. That body now lives exclusively in `bump/rust.rs`. The doc comment above it is also removed (it's been moved to the sibling file).

- [ ] **Step 4: Update the dispatch arm in `ReadyBump::execute`**

In the dispatch block, change the Rust arm from:
```rust
            Ecosystem::Rust => {
                let files = bump_rust_version(project_root, &self.next, &self.detection)?;
                modified_files.extend(files);
            }
```
to:
```rust
            Ecosystem::Rust => {
                let files = rust::bump_rust_version(project_root, &self.next, &self.detection)?;
                modified_files.extend(files);
            }
```

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
Expected: no warnings. Common warnings to watch for: unused imports (if `use std::process::Command;` was accidentally left in `bump/mod.rs` after the helper was deleted — it's still needed for `generate_changelog`, so DON'T remove it).

- [ ] **Step 7: Run bump tests**

Run (with `dangerouslyDisableSandbox: true`):
```bash
cargo nextest run -p scrat-core bump::
```
Expected: same test count as baseline, all passing. The `execute_rust_no_bump_tool_returns_error` test exercises Rust bump through the public API and verifies that the move preserved behavior.

- [ ] **Step 8: Append to commit.txt**

Use the Edit tool to APPEND a new sub-bullet section. Find the last line of Task 3's sub-bullet (`No behavior change. All bump::tests still pass.` — the second occurrence, at the end of Task 3's body) and replace it with:

```
No behavior change. All bump::tests still pass.

* refactor(bump): extract bump_rust_version to bump/rust.rs

Body moved verbatim from bump/mod.rs to bump/rust.rs. Dispatch in
ReadyBump::execute updated to call rust::bump_rust_version. No
behavior change.
```

If `commit.txt` does not exist because Clay ran `gtxt` between Task 3 and Task 4, create it fresh with the Write tool using this content:

```
refactor(bump): extract bump_rust_version to bump/rust.rs

Part of phase 2 of the ecosystem-modules refactor.

* refactor(bump): extract bump_rust_version to bump/rust.rs

Body moved verbatim from bump/mod.rs to bump/rust.rs. Dispatch in
ReadyBump::execute updated to call rust::bump_rust_version. No
behavior change.
```

- [ ] **Step 9: Commit via gtxt**

Clay runs `gtxt`. Wait before Task 5.

---

### Task 5: Extract `bump_node_version` → `bump/node.rs`

**Files:**
- Create: `crates/scrat-core/src/bump/node.rs`
- Modify: `crates/scrat-core/src/bump/mod.rs`

- [ ] **Step 1: Create `crates/scrat-core/src/bump/node.rs`**

Create the file with this exact content:

```rust
//! Node.js ecosystem version bumping.
//!
//! Edits `package.json` directly. scrat is intentionally *not* a
//! lockfile manager — `package-lock.json` / `yarn.lock` / `pnpm-lock.yaml`
//! sync is the user's package manager's job.

use camino::Utf8Path;
use semver::Version;
use tracing::debug;

use super::{BumpError, BumpResult};

/// Bump the version in `package.json` directly.
///
/// scrat edits only `package.json` — it is intentionally *not* a
/// lockfile manager. If the user needs `package-lock.json` (or
/// `yarn.lock`, `pnpm-lock.yaml`) synced after the bump, that's their
/// package manager's job (e.g. a pre-commit scrat hook running
/// `npm install --package-lock-only`).
///
/// Returns the repo-relative path of the file that was updated.
pub(super) fn bump_node_version(
    project_root: &Utf8Path,
    version: &Version,
) -> BumpResult<Vec<String>> {
    let package_path = project_root.join("package.json");
    let content = std::fs::read_to_string(&package_path).map_err(|e| BumpError::ToolFailed {
        tool: "package.json".into(),
        message: format!("failed to read: {e}"),
    })?;

    let mut parsed: serde_json::Value =
        serde_json::from_str(&content).map_err(|e| BumpError::ToolFailed {
            tool: "package.json".into(),
            message: format!("failed to parse: {e}"),
        })?;

    if parsed.get("version").and_then(|v| v.as_str()).is_none() {
        return Err(BumpError::ToolFailed {
            tool: "package.json".into(),
            message: "no `version` field found — cannot bump".into(),
        });
    }

    parsed["version"] = serde_json::Value::String(version.to_string());

    // npm convention: 2-space indent, trailing newline
    let output = serde_json::to_string_pretty(&parsed).map_err(|e| BumpError::ToolFailed {
        tool: "package.json".into(),
        message: format!("failed to serialize: {e}"),
    })?;

    std::fs::write(&package_path, format!("{output}\n")).map_err(|e| BumpError::ToolFailed {
        tool: "package.json".into(),
        message: format!("failed to write: {e}"),
    })?;

    debug!(%version, "bumped package.json version");
    Ok(vec!["package.json".into()])
}
```

Carry forward from Task 4:
- Module-level doc comment first.
- Import groups: external crates, then `super::`, then `crate::` (no `crate::` needed for Node).
- `pub(super)` visibility.
- Function body verbatim from `bump/mod.rs` after Task 3's harmonization.

- [ ] **Step 2: Add `mod node;` to `bump/mod.rs`**

Add `mod node;` to the module declarations, keeping alphabetical order:

```rust
mod node;
mod rust;
```

- [ ] **Step 3: Delete the local `bump_node_version` function from `bump/mod.rs`**

Remove the entire `fn bump_node_version(...) -> BumpResult<Vec<String>> { ... }` function body from `bump/mod.rs`, along with its doc comment.

- [ ] **Step 4: Update the dispatch arm in `ReadyBump::execute`**

Change:
```rust
            Ecosystem::Node => {
                let files = bump_node_version(project_root, &self.next)?;
                modified_files.extend(files);
            }
```
to:
```rust
            Ecosystem::Node => {
                let files = node::bump_node_version(project_root, &self.next)?;
                modified_files.extend(files);
            }
```

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

- [ ] **Step 7: Run bump tests**

Run (with `dangerouslyDisableSandbox: true`):
```bash
cargo nextest run -p scrat-core bump::
```
Expected: same test count as baseline, all passing. `execute_node_updates_package_json_only` and `execute_node_errors_without_version_field` exercise Node bump through the public API.

- [ ] **Step 8: Append to commit.txt**

Use the Edit tool to APPEND a new sub-bullet. Find the last line of Task 4's sub-bullet (`...No behavior change.`) and replace with:

```
No behavior change.

* refactor(bump): extract bump_node_version to bump/node.rs

Body moved verbatim from bump/mod.rs to bump/node.rs. Dispatch in
ReadyBump::execute updated to call node::bump_node_version. No
behavior change.
```

If `commit.txt` does not exist because `gtxt` ran, create it fresh with a skeleton header + this sub-bullet (same pattern as Task 4 Step 8's fallback).

- [ ] **Step 9: Commit via gtxt**

Clay runs `gtxt`. Wait before Task 6.

---

### Task 6: Extract `bump_composer_version` → `bump/php.rs`

**Files:**
- Create: `crates/scrat-core/src/bump/php.rs`
- Modify: `crates/scrat-core/src/bump/mod.rs`

- [ ] **Step 1: Create `crates/scrat-core/src/bump/php.rs`**

Create the file with this exact content:

```rust
//! PHP / Composer ecosystem version bumping.
//!
//! Edits `composer.json` directly if and only if a `"version"` field
//! already exists. Composer does not require a version field at the
//! package level — most packages rely on git tags — so absence is
//! treated as a silent skip, not an error.

use camino::Utf8Path;
use semver::Version;
use tracing::debug;

use super::{BumpError, BumpResult};

/// Bump the version in `composer.json` if it has a `"version"` field.
///
/// Returns the repo-relative path of the file that was updated, or an
/// empty vec if `composer.json` is missing or has no `version` field.
pub(super) fn bump_composer_version(
    project_root: &Utf8Path,
    version: &Version,
) -> BumpResult<Vec<String>> {
    let composer_path = project_root.join("composer.json");
    let content = match std::fs::read_to_string(&composer_path) {
        Ok(c) => c,
        Err(_) => return Ok(vec![]),
    };

    let mut parsed: serde_json::Value =
        serde_json::from_str(&content).map_err(|e| BumpError::ToolFailed {
            tool: "composer.json".into(),
            message: format!("failed to parse: {e}"),
        })?;

    // Only write if the field already exists — don't add it if absent
    if parsed.get("version").and_then(|v| v.as_str()).is_none() {
        return Ok(vec![]);
    }

    parsed["version"] = serde_json::Value::String(version.to_string());

    let output = serde_json::to_string_pretty(&parsed).map_err(|e| BumpError::ToolFailed {
        tool: "composer.json".into(),
        message: format!("failed to serialize: {e}"),
    })?;

    // Composer convention: trailing newline
    std::fs::write(&composer_path, format!("{output}\n")).map_err(|e| BumpError::ToolFailed {
        tool: "composer.json".into(),
        message: format!("failed to write: {e}"),
    })?;

    debug!(%version, "bumped composer.json version");
    Ok(vec!["composer.json".into()])
}
```

- [ ] **Step 2: Add `mod php;` to `bump/mod.rs`**

Add `mod php;` in alphabetical order:

```rust
mod node;
mod php;
mod rust;
```

- [ ] **Step 3: Delete the local `bump_composer_version` function from `bump/mod.rs`**

Remove the entire `fn bump_composer_version(...) -> BumpResult<Vec<String>> { ... }` function body and its doc comment.

- [ ] **Step 4: Update the dispatch arm in `ReadyBump::execute`**

Change:
```rust
            Ecosystem::Php => {
                let files = bump_composer_version(project_root, &self.next)?;
                if files.is_empty() {
                    debug!("composer.json has no version field, skipping");
                }
                modified_files.extend(files);
            }
```
to:
```rust
            Ecosystem::Php => {
                let files = php::bump_composer_version(project_root, &self.next)?;
                if files.is_empty() {
                    debug!("composer.json has no version field, skipping");
                }
                modified_files.extend(files);
            }
```

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

- [ ] **Step 7: Run bump tests**

Run (with `dangerouslyDisableSandbox: true`):
```bash
cargo nextest run -p scrat-core bump::
```
Expected: same test count as baseline, all passing.

- [ ] **Step 8: Append to commit.txt**

Use the Edit tool to APPEND a new sub-bullet:

```
No behavior change.

* refactor(bump): extract bump_composer_version to bump/php.rs

Body moved verbatim from bump/mod.rs to bump/php.rs. Dispatch in
ReadyBump::execute updated to call php::bump_composer_version. No
behavior change.
```

(Fresh-skeleton fallback if `gtxt` ran since Task 5: same pattern as Task 4 Step 8.)

- [ ] **Step 9: Commit via gtxt**

Clay runs `gtxt`. Wait before Task 7.

---

### Task 7: Extract `bump_pyproject_version` → `bump/python.rs`

**Files:**
- Create: `crates/scrat-core/src/bump/python.rs`
- Modify: `crates/scrat-core/src/bump/mod.rs`

- [ ] **Step 1: Create `crates/scrat-core/src/bump/python.rs`**

Create the file with this exact content:

```rust
//! Python ecosystem version bumping.
//!
//! Edits `pyproject.toml` directly when a `version` field exists under
//! the `[project]` table. Absence is treated as a silent skip — a
//! `pyproject.toml` without a `[project] version` field is valid (the
//! version may come from a dynamic source like `setuptools-scm`).

use camino::Utf8Path;
use semver::Version;
use tracing::debug;

use super::{BumpError, BumpResult};

/// Bump the version in `pyproject.toml` if it has a `version` field under `[project]`.
///
/// Returns the repo-relative path of the file that was updated, or an
/// empty vec if `pyproject.toml` is missing or has no `[project] version` field.
pub(super) fn bump_pyproject_version(
    project_root: &Utf8Path,
    version: &Version,
) -> BumpResult<Vec<String>> {
    let pyproject_path = project_root.join("pyproject.toml");
    let content = match std::fs::read_to_string(&pyproject_path) {
        Ok(c) => c,
        Err(_) => return Ok(vec![]),
    };

    // Look for `version = "..."` under `[project]` section
    let mut in_project = false;
    let mut found = false;
    let mut lines: Vec<String> = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_project = trimmed == "[project]";
        }
        if in_project
            && trimmed.starts_with("version")
            && let Some((key, _)) = trimmed.split_once('=')
            && key.trim() == "version"
        {
            lines.push(format!("version = \"{version}\""));
            found = true;
        } else {
            lines.push(line.to_string());
        }
    }

    if !found {
        return Ok(vec![]);
    }

    std::fs::write(&pyproject_path, lines.join("\n") + "\n").map_err(|e| {
        BumpError::ToolFailed {
            tool: "pyproject.toml".into(),
            message: format!("failed to write: {e}"),
        }
    })?;

    debug!(%version, "bumped pyproject.toml version");
    Ok(vec!["pyproject.toml".into()])
}
```

- [ ] **Step 2: Add `mod python;` to `bump/mod.rs`**

Add `mod python;` in alphabetical order:

```rust
mod node;
mod php;
mod python;
mod rust;
```

- [ ] **Step 3: Delete the local `bump_pyproject_version` function from `bump/mod.rs`**

Remove the entire `fn bump_pyproject_version(...) -> BumpResult<Vec<String>> { ... }` function body and its doc comment.

- [ ] **Step 4: Update the dispatch arm in `ReadyBump::execute`**

Change:
```rust
            Ecosystem::Python => {
                let files = bump_pyproject_version(project_root, &self.next)?;
                if files.is_empty() {
                    debug!("pyproject.toml has no version field, skipping");
                }
                modified_files.extend(files);
            }
```
to:
```rust
            Ecosystem::Python => {
                let files = python::bump_pyproject_version(project_root, &self.next)?;
                if files.is_empty() {
                    debug!("pyproject.toml has no version field, skipping");
                }
                modified_files.extend(files);
            }
```

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

- [ ] **Step 7: Run bump tests**

Run (with `dangerouslyDisableSandbox: true`):
```bash
cargo nextest run -p scrat-core bump::
```
Expected: same test count as baseline, all passing.

- [ ] **Step 8: Append to commit.txt**

Use the Edit tool to APPEND:

```
No behavior change.

* refactor(bump): extract bump_pyproject_version to bump/python.rs

Body moved verbatim from bump/mod.rs to bump/python.rs. Dispatch in
ReadyBump::execute updated to call python::bump_pyproject_version. No
behavior change.
```

(Fresh-skeleton fallback if `gtxt` ran since Task 6.)

- [ ] **Step 9: Commit via gtxt**

Clay runs `gtxt`. Wait before Task 8.

---

### Task 8: Extract `bump_ruby_version` + helpers + private tests → `bump/ruby.rs`

**Files:**
- Create: `crates/scrat-core/src/bump/ruby.rs`
- Modify: `crates/scrat-core/src/bump/mod.rs`

**Final move — the biggest and most complex extraction in Phase 2.** Ruby's implementation has five functions and nineteen private-helper tests. All of them move to `bump/ruby.rs` together. Code-quality review runs on this task specifically to catch any test or helper that was missed.

The five functions to move:
1. `bump_ruby_version` — public-to-parent entry point (public API tests stay in `bump/mod.rs::tests` because they go through `ReadyBump::execute`)
2. `update_ruby_version_file` — walks a single `version.rb` file
3. `update_gemspec_version_file` — walks a single `*.gemspec` file
4. `replace_ruby_version_line` — the `VERSION = "..."` line parser (80 lines of byte-level matching)
5. `replace_gemspec_version_line` — the `<receiver>.version = "..."` line parser

The nineteen private tests to move (all currently live in `bump/mod.rs::tests`, lines ~1432–1643):
- Ruby line parser tests (9): `ruby_version_double_quoted`, `ruby_version_single_quoted`, `ruby_version_with_freeze_suffix`, `ruby_version_no_indent`, `ruby_version_extra_whitespace`, `ruby_version_equality_check_rejected`, `ruby_version_comment_rejected`, `ruby_version_suffix_identifier_rejected`, `ruby_version_unrelated_line_rejected`
- Gemspec line parser tests (5): `gemspec_spec_version_literal`, `gemspec_short_receiver`, `gemspec_constant_reference_rejected`, `gemspec_other_attribute_rejected`, `gemspec_versioned_attribute_rejected`
- Integration tests on `bump_ruby_version` (5): `bump_ruby_updates_version_rb_under_lib`, `bump_ruby_updates_gemspec_literal`, `bump_ruby_skips_gemspec_constant_reference`, `bump_ruby_returns_empty_when_nothing_found`, `bump_ruby_finds_nested_version_rb`

All other tests (execute_*, plan_bump_*, resolve_*, BumpError display, BumpOutcome serialization, ReadyBump clone) stay in `bump/mod.rs::tests` because they exercise the public API or test non-Ruby concerns.

- [ ] **Step 1: Create `crates/scrat-core/src/bump/ruby.rs`**

Create the file with this exact content:

```rust
//! Ruby ecosystem version bumping.
//!
//! Updates every `lib/**/version.rb` file that has a `VERSION = "..."`
//! assignment, plus any top-level `*.gemspec` that contains a literal
//! `<receiver>.version = "..."` assignment. Constant references like
//! `spec.version = MyGem::VERSION` are intentionally skipped so the
//! `version.rb` file remains the source of truth.
//!
//! The byte-level line parsers preserve indentation, quote style, and
//! trailing content (e.g. `.freeze`, comments) so the rewrite is
//! minimally invasive.

use camino::{Utf8Path, Utf8PathBuf};
use semver::Version;
use tracing::debug;

use super::{BumpError, BumpResult};

/// Bump Ruby project versions. Updates every `lib/**/version.rb` file that
/// has a `VERSION = "..."` assignment, plus any top-level `*.gemspec` that
/// contains a literal `<spec>.version = "..."` line.
///
/// Returns the paths (relative to `project_root`) of files that were
/// actually modified. Returns an empty `Vec` if no standard Ruby version
/// files were found — callers may fall back to `[[version_files]]`.
pub(super) fn bump_ruby_version(
    project_root: &Utf8Path,
    version: &Version,
) -> BumpResult<Vec<String>> {
    let new_version = version.to_string();
    let mut modified = Vec::new();

    // 1. lib/**/version.rb — the canonical location for gem versions.
    let lib_dir = project_root.join("lib");
    if lib_dir.is_dir() {
        let pattern = format!("{lib_dir}/**/version.rb");
        let paths = glob::glob(&pattern).map_err(|e| BumpError::ToolFailed {
            tool: "ruby".into(),
            message: format!("glob pattern error: {e}"),
        })?;
        for entry in paths {
            let path = entry.map_err(|e| BumpError::ToolFailed {
                tool: "ruby".into(),
                message: format!("glob entry error: {e}"),
            })?;
            let path = Utf8PathBuf::from_path_buf(path).map_err(|p| BumpError::ToolFailed {
                tool: "ruby".into(),
                message: format!("non-UTF-8 path: {}", p.display()),
            })?;
            if update_ruby_version_file(&path, &new_version)? {
                let rel = path
                    .strip_prefix(project_root)
                    .map(Utf8Path::to_path_buf)
                    .unwrap_or_else(|_| path.clone());
                modified.push(rel.to_string());
            }
        }
    }

    // 2. *.gemspec — only update literal `<x>.version = "..."` assignments;
    //    skip `spec.version = MyGem::VERSION` constant references.
    let read_dir =
        std::fs::read_dir(project_root.as_std_path()).map_err(|e| BumpError::ToolFailed {
            tool: "ruby".into(),
            message: format!("failed to read project root: {e}"),
        })?;
    for entry in read_dir {
        let entry = entry.map_err(|e| BumpError::ToolFailed {
            tool: "ruby".into(),
            message: format!("read_dir entry error: {e}"),
        })?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("gemspec") {
            continue;
        }
        let path = Utf8PathBuf::from_path_buf(path).map_err(|p| BumpError::ToolFailed {
            tool: "ruby".into(),
            message: format!("non-UTF-8 gemspec path: {}", p.display()),
        })?;
        if update_gemspec_version_file(&path, &new_version)? {
            let rel = path
                .strip_prefix(project_root)
                .map(Utf8Path::to_path_buf)
                .unwrap_or_else(|_| path.clone());
            modified.push(rel.to_string());
        }
    }

    debug!(files = ?modified, "ruby version bump complete");
    Ok(modified)
}

/// Rewrite a Ruby `VERSION = "..."` assignment in-place.
/// Returns `true` if the file was modified.
fn update_ruby_version_file(path: &Utf8Path, new_version: &str) -> BumpResult<bool> {
    let content =
        std::fs::read_to_string(path.as_std_path()).map_err(|e| BumpError::ToolFailed {
            tool: "ruby".into(),
            message: format!("failed to read {path}: {e}"),
        })?;

    let mut changed = false;
    let mut out_lines: Vec<String> = Vec::with_capacity(content.lines().count());
    for line in content.lines() {
        if let Some(replaced) = replace_ruby_version_line(line, new_version) {
            if replaced != line {
                changed = true;
            }
            out_lines.push(replaced);
        } else {
            out_lines.push(line.to_string());
        }
    }

    if !changed {
        return Ok(false);
    }

    let mut out = out_lines.join("\n");
    if content.ends_with('\n') {
        out.push('\n');
    }
    std::fs::write(path.as_std_path(), out).map_err(|e| BumpError::ToolFailed {
        tool: "ruby".into(),
        message: format!("failed to write {path}: {e}"),
    })?;
    Ok(true)
}

/// Rewrite `<x>.version = "..."` lines in a gemspec.
///
/// Only touches literal string assignments — leaves constant references
/// like `spec.version = MyGem::VERSION` alone so the version.rb update
/// remains the source of truth.
fn update_gemspec_version_file(path: &Utf8Path, new_version: &str) -> BumpResult<bool> {
    let content =
        std::fs::read_to_string(path.as_std_path()).map_err(|e| BumpError::ToolFailed {
            tool: "ruby".into(),
            message: format!("failed to read {path}: {e}"),
        })?;

    let mut changed = false;
    let mut out_lines: Vec<String> = Vec::with_capacity(content.lines().count());
    for line in content.lines() {
        if let Some(replaced) = replace_gemspec_version_line(line, new_version) {
            if replaced != line {
                changed = true;
            }
            out_lines.push(replaced);
        } else {
            out_lines.push(line.to_string());
        }
    }

    if !changed {
        return Ok(false);
    }

    let mut out = out_lines.join("\n");
    if content.ends_with('\n') {
        out.push('\n');
    }
    std::fs::write(path.as_std_path(), out).map_err(|e| BumpError::ToolFailed {
        tool: "ruby".into(),
        message: format!("failed to write {path}: {e}"),
    })?;
    Ok(true)
}

/// Replace the literal in a `VERSION = "x.y.z"` (or `'x.y.z'`) line.
///
/// Preserves indentation, the receiver (bare `VERSION`, or `self::VERSION`),
/// quote style, and anything trailing (e.g. `.freeze`, comments).
/// Returns `None` if the line isn't a VERSION assignment.
fn replace_ruby_version_line(line: &str, new_version: &str) -> Option<String> {
    let bytes = line.as_bytes();
    let mut i = 0;

    // Skip leading whitespace
    while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
        i += 1;
    }
    // Skip comment lines
    if i < bytes.len() && bytes[i] == b'#' {
        return None;
    }
    // Must start with `VERSION` as a standalone token.
    if !line[i..].starts_with("VERSION") {
        return None;
    }
    // Ensure `VERSION` isn't a suffix of another identifier (e.g. `FOO_VERSION`).
    if i > 0 {
        let prev = bytes[i - 1];
        if prev.is_ascii_alphanumeric() || prev == b'_' {
            return None;
        }
    }
    i += "VERSION".len();
    // Next char must be whitespace or '='
    if i >= bytes.len() {
        return None;
    }
    let next_byte = bytes[i];
    if next_byte != b' ' && next_byte != b'\t' && next_byte != b'=' {
        return None;
    }
    // Find '='
    while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
        i += 1;
    }
    if i >= bytes.len() || bytes[i] != b'=' {
        return None;
    }
    i += 1;
    // Reject '=='
    if i < bytes.len() && bytes[i] == b'=' {
        return None;
    }
    // Skip whitespace
    while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
        i += 1;
    }
    // Expect an opening quote
    if i >= bytes.len() {
        return None;
    }
    let quote = bytes[i];
    if quote != b'"' && quote != b'\'' {
        return None;
    }
    i += 1;
    let content_start = i;
    while i < bytes.len() && bytes[i] != quote {
        // Reject embedded backslash escapes — version strings don't use them
        if bytes[i] == b'\\' {
            return None;
        }
        i += 1;
    }
    if i >= bytes.len() {
        return None;
    }
    let content_end = i;

    let mut result = String::with_capacity(line.len() + new_version.len());
    result.push_str(&line[..content_start]);
    result.push_str(new_version);
    result.push_str(&line[content_end..]);
    Some(result)
}

/// Replace the literal in `<x>.version = "y.z"` lines in a gemspec.
///
/// Matches `<receiver>.version` where `<receiver>` is an identifier
/// (typically `spec`, `s`, `gem`, `Gem::Specification.new do |spec|` →
/// `spec`). Returns `None` for constant references like
/// `spec.version = MyGem::VERSION`, so the version.rb update remains the
/// source of truth.
fn replace_gemspec_version_line(line: &str, new_version: &str) -> Option<String> {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
        i += 1;
    }
    if i < bytes.len() && bytes[i] == b'#' {
        return None;
    }
    // Parse receiver — an identifier starting with letter or underscore.
    let receiver_start = i;
    if i >= bytes.len() || !(bytes[i].is_ascii_alphabetic() || bytes[i] == b'_') {
        return None;
    }
    while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
        i += 1;
    }
    if receiver_start == i {
        return None;
    }
    // Expect `.version`
    if !line[i..].starts_with(".version") {
        return None;
    }
    i += ".version".len();
    // `.version` must be a complete token (not e.g. `.versioned`).
    if i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
        return None;
    }
    // Skip whitespace
    while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
        i += 1;
    }
    // Expect '='
    if i >= bytes.len() || bytes[i] != b'=' {
        return None;
    }
    i += 1;
    if i < bytes.len() && bytes[i] == b'=' {
        return None;
    }
    while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
        i += 1;
    }
    // Expect a quote — if the next char isn't a quote, it's a constant
    // reference (MyGem::VERSION). Skip.
    if i >= bytes.len() {
        return None;
    }
    let quote = bytes[i];
    if quote != b'"' && quote != b'\'' {
        return None;
    }
    i += 1;
    let content_start = i;
    while i < bytes.len() && bytes[i] != quote {
        if bytes[i] == b'\\' {
            return None;
        }
        i += 1;
    }
    if i >= bytes.len() {
        return None;
    }
    let content_end = i;

    let mut result = String::with_capacity(line.len() + new_version.len());
    result.push_str(&line[..content_start]);
    result.push_str(new_version);
    result.push_str(&line[content_end..]);
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── ruby version line replacement ────────────────────────

    #[test]
    fn ruby_version_double_quoted() {
        let line = r#"  VERSION = "1.2.3""#;
        assert_eq!(
            replace_ruby_version_line(line, "2.0.0"),
            Some(r#"  VERSION = "2.0.0""#.to_string())
        );
    }

    #[test]
    fn ruby_version_single_quoted() {
        let line = "  VERSION = '1.2.3'";
        assert_eq!(
            replace_ruby_version_line(line, "2.0.0"),
            Some("  VERSION = '2.0.0'".to_string())
        );
    }

    #[test]
    fn ruby_version_with_freeze_suffix() {
        let line = r#"  VERSION = "1.2.3".freeze"#;
        assert_eq!(
            replace_ruby_version_line(line, "2.0.0"),
            Some(r#"  VERSION = "2.0.0".freeze"#.to_string())
        );
    }

    #[test]
    fn ruby_version_no_indent() {
        let line = r#"VERSION = "1.2.3""#;
        assert_eq!(
            replace_ruby_version_line(line, "2.0.0"),
            Some(r#"VERSION = "2.0.0""#.to_string())
        );
    }

    #[test]
    fn ruby_version_extra_whitespace() {
        let line = r#"    VERSION   =   "1.2.3""#;
        assert_eq!(
            replace_ruby_version_line(line, "2.0.0"),
            Some(r#"    VERSION   =   "2.0.0""#.to_string())
        );
    }

    #[test]
    fn ruby_version_equality_check_rejected() {
        // VERSION == "1.2.3" is a comparison, not an assignment.
        let line = r#"if VERSION == "1.2.3""#;
        assert_eq!(replace_ruby_version_line(line, "2.0.0"), None);
    }

    #[test]
    fn ruby_version_comment_rejected() {
        let line = r#"# VERSION = "1.2.3""#;
        assert_eq!(replace_ruby_version_line(line, "2.0.0"), None);
    }

    #[test]
    fn ruby_version_suffix_identifier_rejected() {
        // FOO_VERSION is a different identifier.
        let line = r#"FOO_VERSION = "1.2.3""#;
        assert_eq!(replace_ruby_version_line(line, "2.0.0"), None);
    }

    #[test]
    fn ruby_version_unrelated_line_rejected() {
        assert_eq!(replace_ruby_version_line("puts 'hello'", "2.0.0"), None);
        assert_eq!(replace_ruby_version_line("", "2.0.0"), None);
    }

    // ── gemspec version line replacement ─────────────────────

    #[test]
    fn gemspec_spec_version_literal() {
        let line = r#"  spec.version = "1.2.3""#;
        assert_eq!(
            replace_gemspec_version_line(line, "2.0.0"),
            Some(r#"  spec.version = "2.0.0""#.to_string())
        );
    }

    #[test]
    fn gemspec_short_receiver() {
        let line = r#"  s.version = "1.2.3""#;
        assert_eq!(
            replace_gemspec_version_line(line, "2.0.0"),
            Some(r#"  s.version = "2.0.0""#.to_string())
        );
    }

    #[test]
    fn gemspec_constant_reference_rejected() {
        // Don't touch constant references — version.rb is the source of truth.
        let line = r#"  spec.version = MyGem::VERSION"#;
        assert_eq!(replace_gemspec_version_line(line, "2.0.0"), None);
    }

    #[test]
    fn gemspec_other_attribute_rejected() {
        let line = r#"  spec.name = "my_gem""#;
        assert_eq!(replace_gemspec_version_line(line, "2.0.0"), None);
    }

    #[test]
    fn gemspec_versioned_attribute_rejected() {
        // `.versioned` is not `.version`.
        let line = r#"  spec.versioned = "true""#;
        assert_eq!(replace_gemspec_version_line(line, "2.0.0"), None);
    }

    // ── ruby version file integration ────────────────────────

    #[test]
    fn bump_ruby_updates_version_rb_under_lib() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = camino::Utf8Path::from_path(tmp.path()).unwrap();
        let lib_dir = root.join("lib/my_gem");
        std::fs::create_dir_all(lib_dir.as_std_path()).unwrap();
        let version_rb = lib_dir.join("version.rb");
        std::fs::write(
            version_rb.as_std_path(),
            "module MyGem\n  VERSION = \"0.1.0\"\nend\n",
        )
        .unwrap();

        let modified = bump_ruby_version(root, &Version::new(0, 2, 0)).unwrap();
        assert_eq!(modified.len(), 1);
        assert!(modified[0].ends_with("version.rb"));

        let new_content = std::fs::read_to_string(version_rb.as_std_path()).unwrap();
        assert!(new_content.contains("VERSION = \"0.2.0\""));
        assert!(!new_content.contains("0.1.0"));
    }

    #[test]
    fn bump_ruby_updates_gemspec_literal() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = camino::Utf8Path::from_path(tmp.path()).unwrap();
        let gemspec = root.join("my_gem.gemspec");
        std::fs::write(
            gemspec.as_std_path(),
            "Gem::Specification.new do |spec|\n  \
             spec.name = \"my_gem\"\n  \
             spec.version = \"0.1.0\"\nend\n",
        )
        .unwrap();

        let modified = bump_ruby_version(root, &Version::new(0, 2, 0)).unwrap();
        assert_eq!(modified.len(), 1);
        assert!(modified[0].ends_with(".gemspec"));

        let new_content = std::fs::read_to_string(gemspec.as_std_path()).unwrap();
        assert!(new_content.contains(r#"spec.version = "0.2.0""#));
    }

    #[test]
    fn bump_ruby_skips_gemspec_constant_reference() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = camino::Utf8Path::from_path(tmp.path()).unwrap();
        // version.rb is the real source of truth
        let lib_dir = root.join("lib/my_gem");
        std::fs::create_dir_all(lib_dir.as_std_path()).unwrap();
        std::fs::write(
            lib_dir.join("version.rb").as_std_path(),
            "module MyGem\n  VERSION = \"0.1.0\"\nend\n",
        )
        .unwrap();
        // gemspec uses constant reference
        std::fs::write(
            root.join("my_gem.gemspec").as_std_path(),
            "Gem::Specification.new do |spec|\n  spec.version = MyGem::VERSION\nend\n",
        )
        .unwrap();

        let modified = bump_ruby_version(root, &Version::new(0, 2, 0)).unwrap();
        assert_eq!(modified.len(), 1, "only version.rb should be modified");
        assert!(modified[0].ends_with("version.rb"));

        // Gemspec untouched
        let gemspec_content =
            std::fs::read_to_string(root.join("my_gem.gemspec").as_std_path()).unwrap();
        assert!(gemspec_content.contains("spec.version = MyGem::VERSION"));
    }

    #[test]
    fn bump_ruby_returns_empty_when_nothing_found() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = camino::Utf8Path::from_path(tmp.path()).unwrap();
        let modified = bump_ruby_version(root, &Version::new(0, 2, 0)).unwrap();
        assert!(modified.is_empty());
    }

    #[test]
    fn bump_ruby_finds_nested_version_rb() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = camino::Utf8Path::from_path(tmp.path()).unwrap();
        let lib_dir = root.join("lib/my_gem/core");
        std::fs::create_dir_all(lib_dir.as_std_path()).unwrap();
        std::fs::write(
            lib_dir.join("version.rb").as_std_path(),
            "module MyGem\n  module Core\n    VERSION = \"1.0.0\".freeze\n  end\nend\n",
        )
        .unwrap();

        let modified = bump_ruby_version(root, &Version::new(1, 1, 0)).unwrap();
        assert_eq!(modified.len(), 1);
        let content = std::fs::read_to_string(lib_dir.join("version.rb").as_std_path()).unwrap();
        assert!(content.contains(r#"VERSION = "1.1.0".freeze"#));
    }
}
```

Template notes for Ruby-specific content:
- Imports include `camino::{Utf8Path, Utf8PathBuf}` because the helpers use both. Other sibling files only need `Utf8Path`.
- `use super::{BumpError, BumpResult};` — same as other sibling files.
- The `#[cfg(test)] mod tests` block imports `use super::*;` — this gives the tests access to `bump_ruby_version`, `update_ruby_version_file`, `update_gemspec_version_file`, `replace_ruby_version_line`, `replace_gemspec_version_line`, `BumpError`, `BumpResult`, and `Version` (via the re-export in the parent scope).
- The tests use `camino::Utf8Path::from_path(...)` directly — the test body references `camino::` explicitly because the module-level `use camino::{Utf8Path, Utf8PathBuf};` already imports the types, but the tests use the `from_path` associated function which is found through the full path. (This matches the existing test pattern in `bump.rs::tests`.)
- The private helpers `update_ruby_version_file`, `update_gemspec_version_file`, `replace_ruby_version_line`, `replace_gemspec_version_line` stay `fn` (no visibility modifier) — they're module-private within `bump::ruby`. Only `bump_ruby_version` is `pub(super)`.

- [ ] **Step 2: Add `mod ruby;` to `bump/mod.rs`**

Add `mod ruby;` in alphabetical order:

```rust
mod node;
mod php;
mod python;
mod ruby;
mod rust;
```

- [ ] **Step 3: Delete the Ruby helpers from `bump/mod.rs`**

Remove all five Ruby functions from `bump/mod.rs`:
1. `bump_ruby_version` (around current line 488–551 before Task 8 — adjust for post-harmonization offsets)
2. `update_ruby_version_file` (around 555–588)
3. `update_gemspec_version_file` (around 595–628)
4. `replace_ruby_version_line` (around 635–710)
5. `replace_gemspec_version_line` (around 719–790)

Also remove the `/// Rewrite ...` doc comments above each helper.

After this deletion, `bump/mod.rs` has NO local `fn bump_<ecosystem>_version` or Ruby-helper functions. The coordinator (plan_bump, resolve_interactive, resolve_strategy, current_or_zero, generate_changelog) and the types (BumpPlan, ReadyBump, BumpOutcome, BumpError) remain, along with the `impl ReadyBump { fn execute }` block.

- [ ] **Step 4: Delete the Ruby-specific tests from `bump/mod.rs::tests`**

In the `#[cfg(test)] mod tests` block at the bottom of `bump/mod.rs`, remove the following test functions (all currently live there):

**Ruby line parser tests** (around lines 1432–1503):
- `ruby_version_double_quoted`
- `ruby_version_single_quoted`
- `ruby_version_with_freeze_suffix`
- `ruby_version_no_indent`
- `ruby_version_extra_whitespace`
- `ruby_version_equality_check_rejected`
- `ruby_version_comment_rejected`
- `ruby_version_suffix_identifier_rejected`
- `ruby_version_unrelated_line_rejected`

**Gemspec line parser tests** (around lines 1505–1543):
- `gemspec_spec_version_literal`
- `gemspec_short_receiver`
- `gemspec_constant_reference_rejected`
- `gemspec_other_attribute_rejected`
- `gemspec_versioned_attribute_rejected`

**Ruby version file integration tests** (around lines 1545–1643):
- `bump_ruby_updates_version_rb_under_lib`
- `bump_ruby_updates_gemspec_literal`
- `bump_ruby_skips_gemspec_constant_reference`
- `bump_ruby_returns_empty_when_nothing_found`
- `bump_ruby_finds_nested_version_rb`

Also remove the three `// ── ruby version line replacement ──`, `// ── gemspec version line replacement ──`, and `// ── ruby version file integration ──` divider comments that separate these sections in the current test file.

**Do NOT remove** any test that exercises `execute()`, `plan_bump()`, `resolve_interactive()`, `resolve_strategy()`, `BumpOutcome` serialization, or `BumpError` display. Those stay in `bump/mod.rs::tests` because they test the public API.

**Do NOT remove** the `use crate::ecosystem::{ChangelogTool, DetectedTools};` import at the top of the test module — it's still needed by the `rust_detection()` helper function and several non-Ruby tests.

- [ ] **Step 5: Update the Ruby dispatch arm in `ReadyBump::execute`**

Change:
```rust
            Ecosystem::Ruby => {
                let files = bump_ruby_version(project_root, &self.next)?;
```
to:
```rust
            Ecosystem::Ruby => {
                let files = ruby::bump_ruby_version(project_root, &self.next)?;
```

The rest of the Ruby arm (the empty-and-no-version_files error check and the `modified_files.extend(files)` call) stays unchanged.

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
Expected: no warnings. Common warnings to watch for:
- Unused `use camino::Utf8PathBuf;` in `bump/mod.rs` — if nothing else in `bump/mod.rs` uses `Utf8PathBuf` after the Ruby deletion, this import must be removed.
- Unused `glob::glob` dependency — check if any other code in bump/mod.rs still uses `glob` after the Ruby deletion; if not, the `glob` dep may become dead (but this affects Cargo.toml, not the file, so it's fine to leave for now).

- [ ] **Step 8: Run bump tests**

Run (with `dangerouslyDisableSandbox: true`):
```bash
cargo nextest run -p scrat-core bump::
```
Expected: same test count as baseline, all passing. Tests moved to `bump::ruby::tests` still appear in the `bump::` namespace scan.

- [ ] **Step 9: Verify the test count breakdown**

Run (with `dangerouslyDisableSandbox: true`):
```bash
cargo nextest run -p scrat-core bump::ruby::
```
Expected: 19 tests pass (9 ruby line parser + 5 gemspec line parser + 5 integration). This confirms the test move was complete.

Also run:
```bash
cargo nextest run -p scrat-core bump::tests::
```
Expected: ~25 tests pass (the public-API tests that stayed in `bump/mod.rs::tests`).

The total across both should equal the Task 1 Step 4 baseline count. If the total dropped, one or more tests were accidentally dropped during the move — investigate before continuing.

- [ ] **Step 10: Append to commit.txt**

Use the Edit tool to APPEND a new sub-bullet:

```
No behavior change.

* refactor(bump): extract bump_ruby_version and helpers to bump/ruby.rs

Moves bump_ruby_version, update_ruby_version_file,
update_gemspec_version_file, replace_ruby_version_line, and
replace_gemspec_version_line to bump/ruby.rs. Dispatch in
ReadyBump::execute updated to call ruby::bump_ruby_version.

Moves the 19 Ruby-specific private-helper tests (9 line parser +
5 gemspec line parser + 5 integration) to bump/ruby.rs::tests. The
~25 public-API tests (execute_*, plan_bump_*, resolve_*, BumpError
display, BumpOutcome serialization) stay in bump/mod.rs::tests.

This completes the extraction of all per-ecosystem bump helpers into
sibling files. bump/mod.rs now contains only the coordinator
(plan_bump, plan_bump_with_detection, resolve_interactive,
resolve_strategy, current_or_zero), the types (BumpPlan, ReadyBump,
InteractiveBump, BumpOutcome, BumpError), ReadyBump::execute with its
uniform dispatch, generate_changelog, and the public-API tests.

No behavior change. All bump::tests still pass.
```

(Fresh-skeleton fallback if `gtxt` ran since Task 7.)

- [ ] **Step 11: Commit via gtxt**

Clay runs `gtxt`. Wait before Task 9.

---

### Task 9: Final verification and PR

**Files:** none (verification + PR)

- [ ] **Step 1: Verify `bump/mod.rs` has shrunk meaningfully**

Run:
```bash
wc -l crates/scrat-core/src/bump/*.rs
```

Expected shape (not exact numbers — the key checks are the relative sizes, not specific line counts):

- `bump/mod.rs` should be in the **950–1050 line range**, down from 1644. It has lost ~430 lines of per-ecosystem helper bodies (rust ~34 + node ~36 + composer ~34 + pyproject ~43 + ruby ~63 + ruby helpers ~170 + ruby tests ~210 = ~590) and gained 5 lines of `mod` declarations. The remaining ~1014 lines are the coordinator functions, type definitions, `execute()`, `generate_changelog`, and the ~25 public-API tests.
- `bump/rust.rs` should be **~55 lines**: module doc, imports, `pub(super) fn bump_rust_version` with verbatim body.
- `bump/node.rs` should be **~60 lines**.
- `bump/php.rs` should be **~55 lines**.
- `bump/python.rs` should be **~65 lines**.
- `bump/ruby.rs` should be **~510 lines**: module doc, imports, 5 functions, and `#[cfg(test)] mod tests` with 19 tests.
- The workspace total across all `bump/*.rs` files should be **around 1700–1750 lines** — slightly higher than the pre-refactor 1644 because each sibling file now carries its own doc comment, `use` block, and (for ruby) test module.

Red flags that mean something went wrong:
- `bump/mod.rs` is still over 1100 lines → a helper or test was missed
- A sibling file (except `ruby.rs`) is over 100 lines → something other than the function body leaked in
- `bump/ruby.rs` is under 400 lines → one or more Ruby helpers/tests didn't fully copy over
- `bump/ruby.rs` is over 600 lines → non-Ruby content leaked in

- [ ] **Step 2: Verify all six new files exist**

Run:
```bash
ls crates/scrat-core/src/bump/
```
Expected:
```
mod.rs
node.rs
php.rs
python.rs
ruby.rs
rust.rs
```

- [ ] **Step 3: Verify workspace still builds**

Run (with `dangerouslyDisableSandbox: true`):
```bash
cargo check --workspace
```
Expected: clean build across the whole workspace (scrat-core + scrat + xtask).

- [ ] **Step 4: Verify workspace clippy is clean**

Run (with `dangerouslyDisableSandbox: true`):
```bash
cargo clippy --workspace --all-targets -- -D warnings
```
Expected: no warnings anywhere in the workspace.

- [ ] **Step 5: Ask Clay before running the full workspace test suite**

Say: "Phase 2 refactor is complete on branch `refactor/ecosystem-modules-phase-2`. All per-task bump tests pass. Before opening the PR, I want to run the full workspace test suite (`cargo nextest run --workspace` or `just test`) — may I proceed, or would you prefer a narrower check?"

Wait for Clay's answer before running.

- [ ] **Step 6: Run whichever test suite Clay approves**

Run the command Clay approves (with `dangerouslyDisableSandbox: true`). Expected: all tests pass, no regressions.

- [ ] **Step 7: Push the branch and open the PR**

Once all verification is green, offer Clay the push:

"Phase 2 is ready to ship. Want me to run `git pm`, or would you prefer to run it yourself?"

If Clay approves the agent running it, run:
```bash
git pm
```

This runs push + open PR + auto-merge per Clay's workflow. When `git pm` prompts for the PR title and description, use:

**Title:**
```
refactor(bump): extract per-ecosystem version bump helpers into bump/ (phase 2)
```

**Body:**
```
## Summary

Phase 2 of the ecosystem-modules refactor. Extracts the five
per-ecosystem version bump helpers into sibling files under a new
`crates/scrat-core/src/bump/` module directory, matching the pattern
validated in phase 1 (PR #37). Harmonizes the per-ecosystem helper
return type to `BumpResult<Vec<String>>` so the dispatch in
`ReadyBump::execute` becomes uniform.

Pure restructuring — no behavior change, no new tests, no trait yet.

## What changed

- `crates/scrat-core/src/bump.rs` → `crates/scrat-core/src/bump/mod.rs`
  (git mv, blame preserved).
- New files: `bump/{rust,node,php,python,ruby}.rs` — one per
  ecosystem with a version-file update path. Go and Swift have no
  bump helpers because their versions live entirely in git tags;
  Generic has no project files.
- Harmonized return types: `bump_rust_version` (was `()`),
  `bump_node_version` (was `bool`), `bump_composer_version` (was
  `bool`), and `bump_pyproject_version` (was `bool`) all now return
  `BumpResult<Vec<String>>`, matching `bump_ruby_version`'s existing
  shape and `BumpOutcome.modified_files`.
- Rewrote the `ReadyBump::execute` dispatch so every ecosystem arm
  follows the uniform
  `let files = helper(...)?; modified_files.extend(files);` pattern.
  PHP/Python silent-skip and Ruby error-on-empty-and-no-version_files
  policy checks are preserved in the dispatch.
- Moved Ruby's 19 private-helper tests (9 line parser + 5 gemspec
  line parser + 5 integration) to `bump/ruby.rs::tests`. The ~25
  public-API tests stay in `bump/mod.rs::tests`.
- `bump/mod.rs` shrunk from 1644 LOC to ~1010 LOC. The retained
  contents: coordinator (plan_bump, plan_bump_with_detection,
  resolve_interactive, resolve_strategy, current_or_zero), types
  (BumpPlan, ReadyBump, InteractiveBump, BumpOutcome, BumpError),
  `ReadyBump::execute` with its uniform dispatch, `generate_changelog`,
  and the public-API tests.

## What did NOT change

- The `BumpPlan`, `ReadyBump`, `InteractiveBump`, `BumpOutcome`,
  `BumpError`, and `BumpResult` public types.
- Any per-ecosystem version bump behavior — bodies were moved
  verbatim after the return-type harmonization.
- Helper signatures. `bump_rust_version` still takes
  `&ProjectDetection`; the others don't. That asymmetry stays — phase
  4 will harmonize signatures as part of trait introduction.
- Function names. `bump_rust_version`, `bump_node_version`, etc. keep
  their current names. Shortening to `bump_rust` etc. is scope creep
  for this phase.
- `crates/scrat-core/src/lib.rs`. The `pub mod bump;` declaration
  at line 38 resolves to `bump/mod.rs` automatically.
- `deps.rs` and `preflight.rs`. Those are phases 3–4.

## Why phase 2 only

This is the second of four planned phases that will ultimately
collapse ecosystem scatter across the scrat-core crate into a single
`ecosystem/<name>.rs` module tree implementing a unified
`EcosystemDriver` trait. Each phase ships independently and
reversibly. Phase 2 validates the file-per-ecosystem pattern on a
larger file (1644 LOC vs phase 1's 582 LOC) with more per-ecosystem
variation before we commit to any trait design. The trait signature
will be designed in phase 4 from observed usage, not on a whiteboard.

Plan document:
`record/superpowers/plans/2026-04-10-ecosystem-modules-phase-2-bump.md`

## Test plan

- [x] `cargo check --workspace` — clean
- [x] `cargo clippy --workspace --all-targets -- -D warnings` — clean
- [x] `cargo nextest run -p scrat-core bump::` — all baseline bump
  tests pass (19 in `bump::ruby::tests`, ~25 in `bump::tests`)
- [x] Full workspace test suite — all passing (per approval in Task 9
  Step 5)
```

---

## Self-review notes

The plan was reviewed against the scope defined at the top. Coverage check:

1. **Every per-ecosystem bump helper moved.** Tasks 4–8 cover rust, node, php, python, ruby — the five ecosystems with a version-file update path in `bump.rs` today. Go and Swift have no helpers (their versions live entirely in git tags). Generic has no helper (no project files). The `Ecosystem::Go | Ecosystem::Swift` and `Ecosystem::Generic` arms in the dispatch remain short-circuits with debug logs.

2. **Return-type harmonization.** Task 3 covers the atomic refactor: `()` and `bool` returns normalize to `Vec<String>`, matching Ruby's existing shape and `BumpOutcome.modified_files`. The `ReadyBump::execute` dispatch is rewritten so every arm follows the same shape. PHP/Python silent-skip and Ruby error-on-empty policies are preserved as dispatch-level checks because they consume non-ecosystem state.

3. **File structure change.** Task 2 covers the `git mv bump.rs → bump/mod.rs` rename as its own commit so blame history is preserved cleanly. The `pub mod bump;` declaration in `lib.rs:38` resolves to either layout — zero lib.rs changes.

4. **Tests.** No new tests. The 19 Ruby private-helper tests move to `bump/ruby.rs::tests` in Task 8. The ~25 public-API tests stay in `bump/mod.rs::tests`. Total bump-namespace test count stays constant. This matches the "no behavior change" scope rule. Task 8 has a dedicated step (Step 9) that verifies the test count breakdown after the move.

5. **Clay's git workflow.** Every commit step writes/appends to `commit.txt` and waits for Clay to run `gtxt`. The PR is opened via `git pm` (and Task 9 Step 7 offers Clay the choice of running it himself). No `git commit` calls from the worker.

6. **Test-running caution.** Per-task tests are narrowed to `cargo nextest run -p scrat-core bump::` (small, fast — ~44 tests). The full workspace suite only runs at the end, and only after asking Clay first.

7. **Consistency across sibling files.** Every sibling file uses the same template: module doc comment, imports block (grouped as `std::`, external crates, `super::`, `crate::`), `pub(super) fn` signature verbatim from the harmonized `bump/mod.rs`, function body verbatim. Only `bump/rust.rs` has the `crate::ecosystem::ProjectDetection` import because it's the only helper that takes `&ProjectDetection`. Only `bump/ruby.rs` has `use camino::{Utf8Path, Utf8PathBuf};` (others only need `Utf8Path`) and a `#[cfg(test)] mod tests` block.

8. **Scope discipline.** No signature normalization (stays as-is). No function renaming (stays as `bump_<ecosystem>_version`). No trait introduction. No `lib.rs` edits. No changes to `deps.rs` or `preflight.rs`. No new tests.

9. **Landmine awareness.** Phase 1's landmines that apply to Phase 2: inline `use` imports in function bodies (none exist in `bump.rs`, good), cargo sandbox flag required (flagged in Conventions), `use semver::Version;` direct import works because `semver` is a direct Cargo.toml dep. Phase 2-specific landmine: the Ruby extraction is big (5 functions + 19 tests), so Task 8 gets extra care with Step 9 (test count breakdown verification).

The plan is self-contained. A worker with zero project context can execute it by following each step literally.

---

## Review optimization pattern (from Phase 1)

Phase 1 validated an optimized review pattern that saved ~4 reviewer dispatches without losing coverage. Phase 2 reuses it:

- **Spec review** on every task (Tasks 1–9) — catches dispatch errors, missed deletions, wrong import paths, off-by-one file placements.
- **Code-quality review** only on:
  - **Task 3** — the atomic refactor (harmonization) is the only non-mechanical change in Phase 2, and the first time the new return-type convention is applied across all helpers.
  - **Task 4** — first repetition of the mechanical extraction template (rust.rs). Catches template drift before Tasks 5–7 multiply it.
  - **Task 8** — final move (ruby.rs). The biggest, most complex extraction with helpers + private tests moved together. High-risk for missed content.
  - **Task 9** — final PR. Catches any cross-file regressions the per-task reviews missed.

Middle mechanical tasks (Tasks 5, 6, 7 — node, php, python) skip code-quality review. The first-repetition review in Task 4 validates the template, and the middle tasks are identical shape — code-quality signal flatlines on repetitions. This is worth proposing to Clay upfront in the execution handoff.

---

## Execution handoff

Plan complete and saved to `record/superpowers/plans/2026-04-10-ecosystem-modules-phase-2-bump.md`. Two execution options:

**1. Subagent-Driven (recommended)** — I dispatch a fresh subagent per task, review between tasks, fast iteration. Best for catching scope drift early and keeping the main conversation context clean. Would reuse the Phase 1 optimization: spec review on every task, code-quality review only on Tasks 3, 4, 8, 9.

**2. Inline Execution** — Execute tasks in this session using executing-plans, batch execution with checkpoints. Best if you want to follow along step-by-step and make course corrections in real time.

Which approach?
