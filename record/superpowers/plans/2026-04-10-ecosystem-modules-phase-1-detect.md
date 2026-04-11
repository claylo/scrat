# Ecosystem Modules Refactor — Phase 1: Finish `detect/`

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extract the six remaining per-ecosystem detection helpers (`detect_node`, `detect_go`, `detect_php`, `detect_python`, `detect_ruby`, `detect_swift`) from `crates/scrat-core/src/detect/mod.rs` into sibling files (`detect/<name>.rs`), matching the existing `detect/rust.rs` pattern. Normalize the dispatch so `build_detection_for` takes `project_root` uniformly and the inlined Rust-arm duplication disappears.

**Architecture:** This is Phase 1 of a four-phase refactor that will eventually collapse the ecosystem-by-ecosystem scatter in `bump.rs` (1644 LOC), `deps.rs` (1343 LOC), and `preflight.rs` (1641 LOC) into a single `crates/scrat-core/src/ecosystem/<name>.rs` module tree implementing a unified `EcosystemDriver` trait. Phase 1 is pure restructuring of `detect/` — no trait yet, no behavior changes, no cross-file concerns. It validates the file-per-ecosystem pattern on the smallest of the monster files before we commit to trait design.

**Tech Stack:** Rust (scrat-core library crate). No new dependencies.

---

## The full arc (context, not in scope for this plan)

| Phase | Goal | Output |
|-------|------|--------|
| **1 (this plan)** | Finish `detect/` split | `detect/{node,go,php,python,ruby,swift}.rs` + normalized dispatch; Rust-arm duplication eliminated |
| **2** | Extract `bump/` with `VersionBumper` trait + harmonized `BumpOutcome` return type | `bump/<name>.rs` per ecosystem; unified return shape `{ files_changed: Vec<Utf8PathBuf>, tool_invoked: Option<&'static str> }` |
| **3** | Extract `deps/` with `LockfileDiffParser` trait | `deps/<name>.rs` per ecosystem |
| **4** | Unify into `ecosystem/<name>.rs` with single `EcosystemDriver` trait that merges `VersionBumper` + `LockfileDiffParser` + detect + registry auth | Single file per ecosystem implementing the unified trait; `bump/`, `deps/`, `detect/` directories collapsed into `ecosystem/` |

Phases 2–4 will be planned as separate documents after each phase completes and we have observed how the abstraction feels under real use. The destination trait sketch from the design conversation — for posterity:

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

The reasoning for the incremental approach: traits designed on a whiteboard tend to either over-specify (every method required in every ecosystem whether it makes sense or not) or under-specify (missing escape hatches for per-ecosystem weirdness — e.g., Ruby's multi-file `Vec<String>` return). Designing from usage, not armchair, is safer.

---

## Out of scope for Phase 1

- **Any behavior change.** Each ecosystem's detection logic is moved verbatim.
- **Adding new tests.** The existing top-level dispatch tests in `detect/mod.rs::tests` provide the refactor safety net. Per-ecosystem test expansion is deferred to Phase 4.
- **Touching `bump.rs`, `deps.rs`, `preflight.rs`.** Those are Phases 2–4.
- **Changing the `Ecosystem` enum, `DetectedTools`, or any public API.** Phase 1 is strictly backward compatible.
- **Introducing a trait.** No `EcosystemDriver` or `Detector` trait in this phase. Just files and a uniform dispatch function signature.

---

## Conventions used in this plan

- **Commits via `commit.txt` — APPEND, do not overwrite:** Each task's commit step APPENDS a sub-bullet section to `commit.txt` at the repo root. It does **not** overwrite the existing file. Clay runs `gtxt` (alias: `git commit -F commit.txt && rm commit.txt`) periodically — sometimes after every task, sometimes after batching several. When `gtxt` runs, the entire accumulated `commit.txt` becomes one bundled commit, and the file is deleted. The next task that produces a commit must re-create the skeleton. The worker does **not** run `git commit` directly.
- **Bundled-commit format:** `commit.txt` follows the structure used in `f706dc9` and `974deb4` — one top-level subject line, a brief opening body paragraph, then multiple `* type(scope): subject` sub-bullet sections (each with its own body paragraph at column 0, NOT indented under the `*`). Each task contributes one sub-bullet section.
- **Test cadence:** Full workspace test runs can be slow on this machine. Each task runs `cargo check -p scrat-core` and `cargo clippy -p scrat-core --all-targets -- -D warnings` (both fast). Running `cargo nextest run -p scrat-core detect::` is fine — the detect test module is small (15 tests). Running the full workspace suite requires asking Clay first.
- **Branch:** `refactor/ecosystem-modules-phase-1`. One branch, several bundled commits via `gtxt`, one PR at the end.

---

### Task 1: Branch setup and baseline verification

**Files:** none (git + verification only)

- [ ] **Step 1: Create feature branch**

Run:
```bash
git checkout -b refactor/ecosystem-modules-phase-1
```

Expected: `Switched to a new branch 'refactor/ecosystem-modules-phase-1'`.

- [ ] **Step 2: Verify clean build on the baseline**

Run:
```bash
cargo check -p scrat-core
```
Expected: clean build, no errors, no warnings, exit code 0.

- [ ] **Step 3: Verify clippy is clean on the baseline**

Run:
```bash
cargo clippy -p scrat-core --all-targets -- -D warnings
```
Expected: no warnings, exit code 0.

- [ ] **Step 4: Record baseline detect-module test count**

Run:
```bash
cargo nextest run -p scrat-core detect::
```
Expected: all tests pass. Record the exact number — you will compare against this at Task 9.

- [ ] **Step 5: Confirm starting line counts**

Run:
```bash
wc -l crates/scrat-core/src/detect/mod.rs crates/scrat-core/src/detect/rust.rs
```
Expected output (approximately):
```
     582 crates/scrat-core/src/detect/mod.rs
      85 crates/scrat-core/src/detect/rust.rs
     667 total
```
Record these numbers. Task 9 will verify `detect/mod.rs` has shrunk to roughly the 300-line range.

---

### Task 2: Normalize dispatch — `build_detection_for` takes `project_root`; Rust-arm duplication eliminated

**Files:**
- Modify: `crates/scrat-core/src/detect/mod.rs`

This task is the only non-mechanical change in Phase 1. It changes three things in one atomic refactor:

1. **`build_detection_for` gains a `project_root: &Utf8Path` parameter** so it can delegate to per-ecosystem helpers uniformly. Currently it only takes `ecosystem` and `version_strategy`.
2. **The Rust arm in `build_detection_for` is collapsed** from ~30 lines of duplicated detection logic into a single call to `rust::detect_rust(project_root, version_strategy)`. This eliminates the copy-paste wart that exists today because `build_detection_for` couldn't call `rust::detect_rust` without a `project_root`.
3. **All six other local helpers (`detect_node`, `detect_go`, `detect_php`, `detect_python`, `detect_ruby`, `detect_swift`) gain a `_project_root: &Utf8Path` parameter** (underscore-prefixed because unused, matching the existing `detect/rust.rs` convention). Their bodies are unchanged.

After this task, every per-ecosystem detection helper has the same `(project_root, version_strategy)` signature. That uniform shape is what makes Tasks 3–8 pure mechanical file moves.

The callers `resolve_detection`, `detect_project`, and `build_detection` are also updated to call `build_detection_for(project_root, ecosystem, version_strategy)` uniformly, dropping the `if ecosystem == Ecosystem::Rust` special case that currently branches to `rust::detect_rust` directly.

- [ ] **Step 1: Apply the atomic refactor to `detect/mod.rs`**

Read the current `crates/scrat-core/src/detect/mod.rs`. Make the following changes together (they interlock — splitting them produces intermediate states that don't compile).

**1a. Update the signature of `detect_node`** (currently around lines 114–155). The body stays unchanged; only the function signature line changes.

Replace:
```rust
fn detect_node(version_strategy: VersionStrategy) -> ProjectDetection {
```
with:
```rust
fn detect_node(_project_root: &Utf8Path, version_strategy: VersionStrategy) -> ProjectDetection {
```

**1b. Apply the identical signature change to `detect_go`, `detect_php`, `detect_python`, `detect_ruby`, `detect_swift`.** Each function's body stays unchanged. Only the signature gains `_project_root: &Utf8Path,` as the first parameter.

**1c. Replace the entire `build_detection_for` function** (currently around lines 339–383) with this new version:

```rust
/// Map an ecosystem + version strategy to a [`ProjectDetection`].
///
/// Single source of truth for ecosystem → tool defaults. Used by
/// `detect_project`, `resolve_detection`, and `build_detection`.
fn build_detection_for(
    project_root: &Utf8Path,
    ecosystem: Ecosystem,
    version_strategy: VersionStrategy,
) -> ProjectDetection {
    match ecosystem {
        Ecosystem::Rust => rust::detect_rust(project_root, version_strategy),
        Ecosystem::Node => detect_node(project_root, version_strategy),
        Ecosystem::Go => detect_go(project_root, version_strategy),
        Ecosystem::Php => detect_php(project_root, version_strategy),
        Ecosystem::Python => detect_python(project_root, version_strategy),
        Ecosystem::Ruby => detect_ruby(project_root, version_strategy),
        Ecosystem::Swift => detect_swift(project_root, version_strategy),
        Ecosystem::Generic => ProjectDetection::generic(version_strategy),
    }
}
```

Notes:
- The `use crate::ecosystem::DetectedTools;` line that was previously inside the function is **removed** (no longer needed, since no ecosystem arm uses it inline).
- The ~30-line inlined Rust arm is collapsed to a single call.

**1d. Simplify `resolve_detection`** (currently around lines 36–56). Remove the `if ecosystem == Ecosystem::Rust` branch and call `build_detection_for` uniformly. Replace:

```rust
pub fn resolve_detection(
    project_root: &Utf8Path,
    config: &crate::config::Config,
) -> Option<ProjectDetection> {
    // Config override takes priority
    if let Some(ref project) = config.project
        && let Some(ecosystem) = project.project_type
    {
        debug!(%ecosystem, "using ecosystem from config override");
        let version_strategy = detect_version_strategy(project_root);
        let detection = if ecosystem == Ecosystem::Rust {
            rust::detect_rust(project_root, version_strategy)
        } else {
            build_detection_for(ecosystem, version_strategy)
        };
        return Some(detection);
    }

    // Fall back to auto-detection
    detect_project(project_root)
}
```
with:
```rust
pub fn resolve_detection(
    project_root: &Utf8Path,
    config: &crate::config::Config,
) -> Option<ProjectDetection> {
    // Config override takes priority
    if let Some(ref project) = config.project
        && let Some(ecosystem) = project.project_type
    {
        debug!(%ecosystem, "using ecosystem from config override");
        let version_strategy = detect_version_strategy(project_root);
        return Some(build_detection_for(project_root, ecosystem, version_strategy));
    }

    // Fall back to auto-detection
    detect_project(project_root)
}
```

**1e. Simplify `detect_project`** (currently around lines 62–76). Remove the Rust special case. Replace:

```rust
pub fn detect_project(project_root: &Utf8Path) -> Option<ProjectDetection> {
    let ecosystem = detect_ecosystem(project_root)?;
    debug!(%ecosystem, "detected ecosystem");

    let version_strategy = detect_version_strategy(project_root);
    debug!(%version_strategy, "detected version strategy");

    let detection = if ecosystem == Ecosystem::Rust {
        rust::detect_rust(project_root, version_strategy)
    } else {
        build_detection_for(ecosystem, version_strategy)
    };
    Some(detection)
}
```
with:
```rust
pub fn detect_project(project_root: &Utf8Path) -> Option<ProjectDetection> {
    let ecosystem = detect_ecosystem(project_root)?;
    debug!(%ecosystem, "detected ecosystem");

    let version_strategy = detect_version_strategy(project_root);
    debug!(%version_strategy, "detected version strategy");

    Some(build_detection_for(project_root, ecosystem, version_strategy))
}
```

**1f. Update `build_detection`** (currently around lines 330–333) to pass `project_root` through. Replace:

```rust
pub fn build_detection(project_root: &Utf8Path, ecosystem: Ecosystem) -> ProjectDetection {
    let version_strategy = detect_version_strategy(project_root);
    build_detection_for(ecosystem, version_strategy)
}
```
with:
```rust
pub fn build_detection(project_root: &Utf8Path, ecosystem: Ecosystem) -> ProjectDetection {
    let version_strategy = detect_version_strategy(project_root);
    build_detection_for(project_root, ecosystem, version_strategy)
}
```

- [ ] **Step 2: Verify compilation**

Run:
```bash
cargo check -p scrat-core
```
Expected: clean build, no errors. If this fails, one of the signature updates was missed.

- [ ] **Step 3: Verify clippy is clean**

Run:
```bash
cargo clippy -p scrat-core --all-targets -- -D warnings
```
Expected: no warnings. A common warning to watch for: if you forgot the underscore prefix on a new `_project_root` parameter, clippy will complain about the unused variable.

- [ ] **Step 4: Run detect tests**

Run:
```bash
cargo nextest run -p scrat-core detect::
```
Expected: same test count as Task 1 Step 4, all passing. If the count changed, something is wrong — you accidentally moved or deleted a test.

- [ ] **Step 5: Write commit.txt**

Create `commit.txt` at the repo root with this content:

```
refactor(detect): normalize build_detection_for dispatch

- Add `project_root: &Utf8Path` as the first parameter of
  `build_detection_for`, matching the shape of `rust::detect_rust`.
- Collapse the duplicated Rust-arm in `build_detection_for` — it now
  delegates to `rust::detect_rust(project_root, version_strategy)`
  directly, eliminating ~25 lines of copy-pasted detection logic.
- Update the six local per-ecosystem helpers (detect_node, detect_go,
  detect_php, detect_python, detect_ruby, detect_swift) to accept
  `_project_root: &Utf8Path` as the first parameter. Bodies unchanged.
- Drop the `if ecosystem == Ecosystem::Rust` special case in both
  `resolve_detection` and `detect_project` — all ecosystems now route
  through `build_detection_for` uniformly.

Preparatory step for extracting each ecosystem's detection logic into
its own sibling file (detect/node.rs, detect/go.rs, etc.) — part of
phase 1 of the ecosystem-modules refactor.

No behavior change. All detect::tests still pass.
```

- [ ] **Step 6: Commit via gtxt**

Clay will run `gtxt` to consume `commit.txt`. Wait for that to happen before proceeding to Task 3.

---

### Task 3: Extract `detect_node` → `detect/node.rs`

**Files:**
- Create: `crates/scrat-core/src/detect/node.rs`
- Modify: `crates/scrat-core/src/detect/mod.rs`

Mechanical move. The body of `detect_node` is copied verbatim into a new file, and `detect/mod.rs` loses the local definition and gains a `mod node;` declaration plus a `node::detect_node(...)` dispatch.

- [ ] **Step 1: Create `crates/scrat-core/src/detect/node.rs`**

Create the file with this exact content:

```rust
//! Node.js ecosystem detection.
//!
//! Probes `PATH` for `npm`/`yarn`/`pnpm` and picks a sensible package
//! manager for test/build/publish. The version bump is always a direct
//! `package.json` edit — scrat is not a lockfile manager.

use camino::Utf8Path;
use tracing::debug;

use super::has_binary;
use crate::ecosystem::{DetectedTools, Ecosystem, ProjectDetection, VersionStrategy};

/// Detect Node.js project tooling and build a [`ProjectDetection`].
pub(super) fn detect_node(
    _project_root: &Utf8Path,
    version_strategy: VersionStrategy,
) -> ProjectDetection {
    let has_npm = has_binary("npm");
    let has_yarn = has_binary("yarn");
    let has_pnpm = has_binary("pnpm");
    debug!(has_npm, has_yarn, has_pnpm, "probed Node tools");

    let (test_cmd, build_cmd, publish_cmd) = if has_pnpm {
        (
            "pnpm test".to_string(),
            "pnpm run build".to_string(),
            Some("pnpm publish".to_string()),
        )
    } else if has_yarn {
        (
            "yarn test".to_string(),
            "yarn build".to_string(),
            Some("yarn publish".to_string()),
        )
    } else {
        (
            "npm test".to_string(),
            "npm run build".to_string(),
            has_npm.then(|| "npm publish".to_string()),
        )
    };

    let changelog_tool = version_strategy.changelog_tool();

    ProjectDetection {
        ecosystem: Ecosystem::Node,
        version_strategy,
        tools: DetectedTools {
            test_cmd,
            build_cmd,
            publish_cmd,
            bump_cmd: None, // handled via direct package.json edit
            changelog_tool,
        },
    }
}
```

- [ ] **Step 2: Add `mod node;` to `detect/mod.rs`**

Near the top of `crates/scrat-core/src/detect/mod.rs`, find the existing `mod rust;` declaration and add `mod node;` immediately after it. The module declarations should be kept in alphabetical order as they accumulate:

```rust
mod node;
mod rust;
```

- [ ] **Step 3: Delete the local `detect_node` function from `detect/mod.rs`**

Remove the entire `fn detect_node(_project_root: &Utf8Path, version_strategy: VersionStrategy) -> ProjectDetection { ... }` function definition from `detect/mod.rs`. That body now lives exclusively in `detect/node.rs`.

- [ ] **Step 4: Update the dispatch arm in `build_detection_for`**

In `build_detection_for`, change the Node arm from:
```rust
Ecosystem::Node => detect_node(project_root, version_strategy),
```
to:
```rust
Ecosystem::Node => node::detect_node(project_root, version_strategy),
```

- [ ] **Step 5: Verify compilation**

Run:
```bash
cargo check -p scrat-core
```
Expected: clean build.

- [ ] **Step 6: Verify clippy is clean**

Run:
```bash
cargo clippy -p scrat-core --all-targets -- -D warnings
```
Expected: no warnings.

- [ ] **Step 7: Run detect tests**

Run:
```bash
cargo nextest run -p scrat-core detect::
```
Expected: same test count as baseline, all passing. The top-level `detect_node_ecosystem` test in `detect/mod.rs::tests` still exercises Node detection through the public API — verifying that the move preserved behavior.

- [ ] **Step 8: Write commit.txt**

Create `commit.txt` at the repo root:

```
refactor(detect): extract detect_node to detect/node.rs

Body moved verbatim from detect/mod.rs to detect/node.rs. Dispatch in
build_detection_for updated to call node::detect_node. No behavior
change.
```

- [ ] **Step 9: Commit via gtxt**

Clay runs `gtxt`. Wait for completion before Task 4.

---

### Task 4: Extract `detect_go` → `detect/go.rs`

**Files:**
- Create: `crates/scrat-core/src/detect/go.rs`
- Modify: `crates/scrat-core/src/detect/mod.rs`

- [ ] **Step 1: Create `crates/scrat-core/src/detect/go.rs`**

Create the file with this exact content:

```rust
//! Go ecosystem detection.
//!
//! Probes `PATH` for `go`. Go modules have no publish step — versioning
//! lives entirely in git tags — so `publish_cmd` and `bump_cmd` are both
//! `None`.

use camino::Utf8Path;
use tracing::debug;

use super::has_binary;
use crate::ecosystem::{DetectedTools, Ecosystem, ProjectDetection, VersionStrategy};

/// Detect Go project tooling and build a [`ProjectDetection`].
pub(super) fn detect_go(
    _project_root: &Utf8Path,
    version_strategy: VersionStrategy,
) -> ProjectDetection {
    let has_go = has_binary("go");
    debug!(has_go, "probed Go tools");

    let changelog_tool = version_strategy.changelog_tool();
    ProjectDetection {
        ecosystem: Ecosystem::Go,
        version_strategy,
        tools: DetectedTools {
            test_cmd: if has_go {
                "go test ./...".into()
            } else {
                String::new()
            },
            build_cmd: if has_go {
                "go build ./...".into()
            } else {
                String::new()
            },
            publish_cmd: None,
            bump_cmd: None, // Go modules version lives in git tags
            changelog_tool,
        },
    }
}
```

- [ ] **Step 2: Add `mod go;` to `detect/mod.rs`**

Add `mod go;` to the module declarations, keeping alphabetical order:

```rust
mod go;
mod node;
mod rust;
```

- [ ] **Step 3: Delete the local `detect_go` function from `detect/mod.rs`**

Remove the entire `fn detect_go(...) -> ProjectDetection { ... }` function definition.

- [ ] **Step 4: Update the dispatch arm in `build_detection_for`**

Change:
```rust
Ecosystem::Go => detect_go(project_root, version_strategy),
```
to:
```rust
Ecosystem::Go => go::detect_go(project_root, version_strategy),
```

- [ ] **Step 5: Verify compilation**

Run:
```bash
cargo check -p scrat-core
```
Expected: clean build.

- [ ] **Step 6: Verify clippy is clean**

Run:
```bash
cargo clippy -p scrat-core --all-targets -- -D warnings
```
Expected: no warnings.

- [ ] **Step 7: Run detect tests**

Run:
```bash
cargo nextest run -p scrat-core detect::
```
Expected: same test count as baseline, all passing.

- [ ] **Step 8: Write commit.txt**

Create `commit.txt`:

```
refactor(detect): extract detect_go to detect/go.rs

Body moved verbatim from detect/mod.rs to detect/go.rs. Dispatch in
build_detection_for updated to call go::detect_go. No behavior change.
```

- [ ] **Step 9: Commit via gtxt**

Clay runs `gtxt`. Wait before Task 5.

---

### Task 5: Extract `detect_php` → `detect/php.rs`

**Files:**
- Create: `crates/scrat-core/src/detect/php.rs`
- Modify: `crates/scrat-core/src/detect/mod.rs`

- [ ] **Step 1: Create `crates/scrat-core/src/detect/php.rs`**

Create the file with this exact content:

```rust
//! PHP / Composer ecosystem detection.
//!
//! Probes `PATH` for `composer`. PHP version bumping is done directly
//! on `composer.json` (when a `version` field exists), so `bump_cmd` is
//! `None`.

use camino::Utf8Path;
use tracing::debug;

use super::has_binary;
use crate::ecosystem::{DetectedTools, Ecosystem, ProjectDetection, VersionStrategy};

/// Detect PHP/Composer project tooling and build a [`ProjectDetection`].
pub(super) fn detect_php(
    _project_root: &Utf8Path,
    version_strategy: VersionStrategy,
) -> ProjectDetection {
    let has_composer = has_binary("composer");
    debug!(has_composer, "probed PHP tools");

    let changelog_tool = version_strategy.changelog_tool();
    ProjectDetection {
        ecosystem: Ecosystem::Php,
        version_strategy,
        tools: DetectedTools {
            test_cmd: if has_composer {
                "composer test".into()
            } else {
                String::new()
            },
            build_cmd: String::new(),
            publish_cmd: None,
            bump_cmd: None, // PHP bump is done directly in composer.json
            changelog_tool,
        },
    }
}
```

- [ ] **Step 2: Add `mod php;` to `detect/mod.rs`**

```rust
mod go;
mod node;
mod php;
mod rust;
```

- [ ] **Step 3: Delete the local `detect_php` function from `detect/mod.rs`**

Remove the entire `fn detect_php(...) -> ProjectDetection { ... }` function definition.

- [ ] **Step 4: Update the dispatch arm in `build_detection_for`**

Change:
```rust
Ecosystem::Php => detect_php(project_root, version_strategy),
```
to:
```rust
Ecosystem::Php => php::detect_php(project_root, version_strategy),
```

- [ ] **Step 5: Verify compilation**

```bash
cargo check -p scrat-core
```
Expected: clean build.

- [ ] **Step 6: Verify clippy is clean**

```bash
cargo clippy -p scrat-core --all-targets -- -D warnings
```
Expected: no warnings.

- [ ] **Step 7: Run detect tests**

```bash
cargo nextest run -p scrat-core detect::
```
Expected: same test count as baseline, all passing.

- [ ] **Step 8: Write commit.txt**

```
refactor(detect): extract detect_php to detect/php.rs

Body moved verbatim from detect/mod.rs to detect/php.rs. Dispatch in
build_detection_for updated to call php::detect_php. No behavior change.
```

- [ ] **Step 9: Commit via gtxt**

Clay runs `gtxt`. Wait before Task 6.

---

### Task 6: Extract `detect_python` → `detect/python.rs`

**Files:**
- Create: `crates/scrat-core/src/detect/python.rs`
- Modify: `crates/scrat-core/src/detect/mod.rs`

- [ ] **Step 1: Create `crates/scrat-core/src/detect/python.rs`**

Create the file with this exact content:

```rust
//! Python ecosystem detection.
//!
//! Probes `PATH` for `uv`, `pytest`, `python`/`python3`, and `twine`.
//! Version bumping is done directly on `pyproject.toml` (when a
//! `[project] version` exists), so `bump_cmd` is `None`.

use camino::Utf8Path;
use tracing::debug;

use super::has_binary;
use crate::ecosystem::{DetectedTools, Ecosystem, ProjectDetection, VersionStrategy};

/// Detect Python project tooling and build a [`ProjectDetection`].
pub(super) fn detect_python(
    _project_root: &Utf8Path,
    version_strategy: VersionStrategy,
) -> ProjectDetection {
    let has_uv = has_binary("uv");
    let has_pytest = has_binary("pytest");
    let has_python = has_binary("python3") || has_binary("python");
    let has_twine = has_binary("twine");
    debug!(
        has_uv,
        has_pytest, has_python, has_twine, "probed Python tools"
    );

    let test_cmd = if has_uv {
        "uv run pytest".into()
    } else if has_pytest {
        "pytest".into()
    } else {
        String::new()
    };
    let build_cmd = if has_uv {
        "uv build".into()
    } else if has_python {
        "python -m build".into()
    } else {
        String::new()
    };
    let publish_cmd = if has_uv {
        Some("uv publish".into())
    } else if has_twine {
        Some("twine upload dist/*".into())
    } else {
        None
    };

    let changelog_tool = version_strategy.changelog_tool();
    ProjectDetection {
        ecosystem: Ecosystem::Python,
        version_strategy,
        tools: DetectedTools {
            test_cmd,
            build_cmd,
            publish_cmd,
            bump_cmd: None, // Python bump is done directly in pyproject.toml
            changelog_tool,
        },
    }
}
```

- [ ] **Step 2: Add `mod python;` to `detect/mod.rs`**

```rust
mod go;
mod node;
mod php;
mod python;
mod rust;
```

- [ ] **Step 3: Delete the local `detect_python` function from `detect/mod.rs`**

Remove the entire `fn detect_python(...) -> ProjectDetection { ... }` function definition.

- [ ] **Step 4: Update the dispatch arm in `build_detection_for`**

Change:
```rust
Ecosystem::Python => detect_python(project_root, version_strategy),
```
to:
```rust
Ecosystem::Python => python::detect_python(project_root, version_strategy),
```

- [ ] **Step 5: Verify compilation**

```bash
cargo check -p scrat-core
```
Expected: clean build.

- [ ] **Step 6: Verify clippy is clean**

```bash
cargo clippy -p scrat-core --all-targets -- -D warnings
```
Expected: no warnings.

- [ ] **Step 7: Run detect tests**

```bash
cargo nextest run -p scrat-core detect::
```
Expected: same test count as baseline, all passing.

- [ ] **Step 8: Write commit.txt**

```
refactor(detect): extract detect_python to detect/python.rs

Body moved verbatim from detect/mod.rs to detect/python.rs. Dispatch
in build_detection_for updated to call python::detect_python. No
behavior change.
```

- [ ] **Step 9: Commit via gtxt**

Clay runs `gtxt`. Wait before Task 7.

---

### Task 7: Extract `detect_ruby` → `detect/ruby.rs`

**Files:**
- Create: `crates/scrat-core/src/detect/ruby.rs`
- Modify: `crates/scrat-core/src/detect/mod.rs`

- [ ] **Step 1: Create `crates/scrat-core/src/detect/ruby.rs`**

Create the file with this exact content:

```rust
//! Ruby ecosystem detection.
//!
//! Probes `PATH` for `bundle`, `rake`, and `gem`. Version bumping is
//! handled via `lib/**/version.rb` and gemspec walkers in the bump
//! module, so `bump_cmd` is `None`.

use camino::Utf8Path;
use tracing::debug;

use super::has_binary;
use crate::ecosystem::{DetectedTools, Ecosystem, ProjectDetection, VersionStrategy};

/// Detect Ruby project tooling and build a [`ProjectDetection`].
pub(super) fn detect_ruby(
    _project_root: &Utf8Path,
    version_strategy: VersionStrategy,
) -> ProjectDetection {
    let has_bundle = has_binary("bundle");
    let has_rake = has_binary("rake");
    let has_gem = has_binary("gem");
    debug!(has_bundle, has_rake, has_gem, "probed Ruby tools");

    let test_cmd = if has_bundle && has_rake {
        "bundle exec rake test".into()
    } else if has_rake {
        "rake test".into()
    } else {
        String::new()
    };
    let build_cmd = if has_gem {
        "gem build".into()
    } else {
        String::new()
    };
    let publish_cmd = has_gem.then(|| "gem push".to_string());

    let changelog_tool = version_strategy.changelog_tool();
    ProjectDetection {
        ecosystem: Ecosystem::Ruby,
        version_strategy,
        tools: DetectedTools {
            test_cmd,
            build_cmd,
            publish_cmd,
            bump_cmd: None, // handled via lib/**/version.rb + gemspec
            changelog_tool,
        },
    }
}
```

- [ ] **Step 2: Add `mod ruby;` to `detect/mod.rs`**

```rust
mod go;
mod node;
mod php;
mod python;
mod ruby;
mod rust;
```

- [ ] **Step 3: Delete the local `detect_ruby` function from `detect/mod.rs`**

Remove the entire `fn detect_ruby(...) -> ProjectDetection { ... }` function definition.

- [ ] **Step 4: Update the dispatch arm in `build_detection_for`**

Change:
```rust
Ecosystem::Ruby => detect_ruby(project_root, version_strategy),
```
to:
```rust
Ecosystem::Ruby => ruby::detect_ruby(project_root, version_strategy),
```

- [ ] **Step 5: Verify compilation**

```bash
cargo check -p scrat-core
```
Expected: clean build.

- [ ] **Step 6: Verify clippy is clean**

```bash
cargo clippy -p scrat-core --all-targets -- -D warnings
```
Expected: no warnings.

- [ ] **Step 7: Run detect tests**

```bash
cargo nextest run -p scrat-core detect::
```
Expected: same test count as baseline, all passing.

- [ ] **Step 8: Write commit.txt**

```
refactor(detect): extract detect_ruby to detect/ruby.rs

Body moved verbatim from detect/mod.rs to detect/ruby.rs. Dispatch in
build_detection_for updated to call ruby::detect_ruby. No behavior
change.
```

- [ ] **Step 9: Commit via gtxt**

Clay runs `gtxt`. Wait before Task 8.

---

### Task 8: Extract `detect_swift` → `detect/swift.rs`

**Files:**
- Create: `crates/scrat-core/src/detect/swift.rs`
- Modify: `crates/scrat-core/src/detect/mod.rs`

Last ecosystem move. After this task, `detect/mod.rs` contains zero per-ecosystem detection bodies.

- [ ] **Step 1: Create `crates/scrat-core/src/detect/swift.rs`**

Create the file with this exact content:

```rust
//! Swift / SwiftPM ecosystem detection.
//!
//! Probes `PATH` for `swift`. SwiftPM has no publish step — versioning
//! lives in git tags — so `publish_cmd` and `bump_cmd` are both `None`.

use camino::Utf8Path;
use tracing::debug;

use super::has_binary;
use crate::ecosystem::{DetectedTools, Ecosystem, ProjectDetection, VersionStrategy};

/// Detect Swift project tooling and build a [`ProjectDetection`].
pub(super) fn detect_swift(
    _project_root: &Utf8Path,
    version_strategy: VersionStrategy,
) -> ProjectDetection {
    let has_swift = has_binary("swift");
    debug!(has_swift, "probed Swift tools");

    let changelog_tool = version_strategy.changelog_tool();
    ProjectDetection {
        ecosystem: Ecosystem::Swift,
        version_strategy,
        tools: DetectedTools {
            test_cmd: if has_swift {
                "swift test".into()
            } else {
                String::new()
            },
            build_cmd: if has_swift {
                "swift build -c release".into()
            } else {
                String::new()
            },
            publish_cmd: None, // SwiftPM publishes via git tags
            bump_cmd: None,
            changelog_tool,
        },
    }
}
```

- [ ] **Step 2: Add `mod swift;` to `detect/mod.rs`**

```rust
mod go;
mod node;
mod php;
mod python;
mod ruby;
mod rust;
mod swift;
```

- [ ] **Step 3: Delete the local `detect_swift` function from `detect/mod.rs`**

Remove the entire `fn detect_swift(...) -> ProjectDetection { ... }` function definition. At this point, `detect/mod.rs` should contain no local `detect_<ecosystem>` functions at all.

- [ ] **Step 4: Update the dispatch arm in `build_detection_for`**

Change:
```rust
Ecosystem::Swift => detect_swift(project_root, version_strategy),
```
to:
```rust
Ecosystem::Swift => swift::detect_swift(project_root, version_strategy),
```

At this point, every arm of `build_detection_for` either delegates to a sibling module (`rust::`, `node::`, `go::`, `php::`, `python::`, `ruby::`, `swift::`) or returns a generic project detection directly. The dispatch function should look like this in its entirety:

```rust
/// Map an ecosystem + version strategy to a [`ProjectDetection`].
///
/// Single source of truth for ecosystem → tool defaults. Used by
/// `detect_project`, `resolve_detection`, and `build_detection`.
fn build_detection_for(
    project_root: &Utf8Path,
    ecosystem: Ecosystem,
    version_strategy: VersionStrategy,
) -> ProjectDetection {
    match ecosystem {
        Ecosystem::Rust => rust::detect_rust(project_root, version_strategy),
        Ecosystem::Node => node::detect_node(project_root, version_strategy),
        Ecosystem::Go => go::detect_go(project_root, version_strategy),
        Ecosystem::Php => php::detect_php(project_root, version_strategy),
        Ecosystem::Python => python::detect_python(project_root, version_strategy),
        Ecosystem::Ruby => ruby::detect_ruby(project_root, version_strategy),
        Ecosystem::Swift => swift::detect_swift(project_root, version_strategy),
        Ecosystem::Generic => ProjectDetection::generic(version_strategy),
    }
}
```

- [ ] **Step 5: Verify compilation**

```bash
cargo check -p scrat-core
```
Expected: clean build.

- [ ] **Step 6: Verify clippy is clean**

```bash
cargo clippy -p scrat-core --all-targets -- -D warnings
```
Expected: no warnings.

- [ ] **Step 7: Run detect tests**

```bash
cargo nextest run -p scrat-core detect::
```
Expected: same test count as baseline, all passing.

- [ ] **Step 8: Write commit.txt**

```
refactor(detect): extract detect_swift to detect/swift.rs

Body moved verbatim from detect/mod.rs to detect/swift.rs. Dispatch
in build_detection_for updated to call swift::detect_swift.

This completes the extraction of all six remaining per-ecosystem
detection helpers into sibling files. detect/mod.rs now contains
only the coordinator (resolve_detection, detect_project,
detect_ecosystem, build_detection_for, build_detection), shared
helpers (has_binary, detect_version_strategy, check_tool_version,
parse_version_from_output, MIN_GIT_CLIFF_VERSION, ToolVersionCheck),
and the top-level dispatch tests.
```

- [ ] **Step 9: Commit via gtxt**

Clay runs `gtxt`. Wait before Task 9.

---

### Task 9: Final verification and PR

**Files:** none (verification + PR)

- [ ] **Step 1: Verify `detect/mod.rs` has shrunk meaningfully**

Run:
```bash
wc -l crates/scrat-core/src/detect/*.rs
```

Expected shape (not exact numbers — the key checks are the relative sizes, not specific line counts):

- `detect/mod.rs` should be in the **300–360 line range**, down from 582. It has lost ~209 lines of per-ecosystem helper bodies and ~29 lines from the collapsed Rust-arm duplication, and gained 6 lines of `mod` declarations.
- Each new sibling file (`node.rs`, `go.rs`, `php.rs`, `python.rs`, `ruby.rs`, `swift.rs`) should be **30–65 lines**, consisting of: module-level doc comment, imports block, function signature, and the verbatim body from the original helper.
- `detect/rust.rs` should be **unchanged at 85 lines**.
- The workspace total across all `detect/*.rs` files should be **around 700–730 lines** — slightly higher than the pre-refactor total because each sibling file now carries its own doc comment and `use` block.

Red flags that mean something went wrong:
- `detect/mod.rs` is still over 500 lines → you forgot to delete one of the old helper functions
- A sibling file is under 25 lines → the function body didn't fully copy over
- A sibling file is over 100 lines → something other than the function body leaked in
- `detect/rust.rs` changed → Phase 1 is not supposed to touch it at all

- [ ] **Step 2: Verify all six new files exist**

Run:
```bash
ls crates/scrat-core/src/detect/
```
Expected:
```
go.rs
mod.rs
node.rs
php.rs
python.rs
ruby.rs
rust.rs
swift.rs
```

- [ ] **Step 3: Verify workspace still builds**

Run:
```bash
cargo check --workspace
```
Expected: clean build across the whole workspace (scrat-core + scrat + xtask).

- [ ] **Step 4: Verify workspace clippy is clean**

Run:
```bash
cargo clippy --workspace --all-targets -- -D warnings
```
Expected: no warnings anywhere in the workspace.

- [ ] **Step 5: Ask Clay before running the full workspace test suite**

Say: "Phase 1 refactor is complete on branch `refactor/ecosystem-modules-phase-1`. All per-task detect tests pass. Before opening the PR, I want to run the full workspace test suite (`cargo nextest run --workspace` or `just test`) — may I proceed, or would you prefer a narrower check?"

Wait for Clay's answer before running.

- [ ] **Step 6: Run whichever test suite Clay approves**

Run the command Clay approves. Expected: all tests pass, no regressions.

- [ ] **Step 7: Push the branch and open the PR**

Once all verification is green, push with Clay's flow:
```bash
git pm
```

This runs push + open PR + auto-merge per Clay's workflow. When `git pm` prompts for the PR title and description, use:

**Title:**
```
refactor(detect): extract per-ecosystem detection into sibling files (phase 1)
```

**Body:**
```
## Summary

Phase 1 of the ecosystem-modules refactor. Extracts the six remaining
per-ecosystem detection helpers into sibling files under
`crates/scrat-core/src/detect/`, matching the existing `detect/rust.rs`
pattern. Normalizes `build_detection_for` dispatch so every ecosystem
routes through a uniform `(project_root, version_strategy)` signature,
eliminating the ~25-line copy-pasted Rust arm that previously duplicated
the logic in `rust::detect_rust`.

Pure restructuring — no behavior change, no new tests, no trait yet.

## What changed

- New files: `detect/{node,go,php,python,ruby,swift}.rs` — one per
  ecosystem, each with a `pub(super) fn detect_<name>(project_root,
  version_strategy) -> ProjectDetection`.
- `build_detection_for` signature gained `project_root: &Utf8Path` as
  first parameter. The inlined Rust arm collapsed to a single
  `rust::detect_rust(...)` call.
- `resolve_detection` and `detect_project` dropped the
  `if ecosystem == Ecosystem::Rust` special-case branching — every
  ecosystem now routes through `build_detection_for` uniformly.
- `detect/mod.rs` shrunk from 582 LOC to ~300 LOC. The retained
  contents: coordinator functions, shared helpers (`has_binary`,
  `detect_version_strategy`, `check_tool_version`,
  `parse_version_from_output`, `MIN_GIT_CLIFF_VERSION`,
  `ToolVersionCheck`), and top-level dispatch tests.

## What did NOT change

- The `Ecosystem` enum, `DetectedTools`, `ProjectDetection`, and every
  other public API.
- Any per-ecosystem detection behavior — bodies were moved verbatim.
- `bump.rs`, `deps.rs`, `preflight.rs` — those are phases 2–4.

## Why phase 1 only

This is the first of four planned phases that will ultimately collapse
ecosystem scatter across the scrat-core crate into a single
`ecosystem/<name>.rs` module tree implementing a unified
`EcosystemDriver` trait. Each phase ships independently and reversibly.
Phase 1 (this PR) validates the file-per-ecosystem pattern on the
smallest of the monster files before we commit to any trait design.

Plan document:
`record/superpowers/plans/2026-04-10-ecosystem-modules-phase-1-detect.md`

## Test plan

- [x] `cargo check --workspace` — clean
- [x] `cargo clippy --workspace --all-targets -- -D warnings` — clean
- [x] `cargo nextest run -p scrat-core detect::` — all baseline detect
  tests pass
- [x] Full workspace test suite — all passing (per approval in Task 9
  Step 5)
```

---

## Self-review notes

The plan was reviewed against the scope defined at the top. Coverage check:

1. **Every ecosystem moved.** Tasks 3–8 cover node, go, php, python, ruby, swift — the six remaining ecosystems that live in `detect/mod.rs` today. `rust.rs` already exists and is untouched. Generic has no detection helper — it's handled inline in `build_detection_for` via `ProjectDetection::generic(version_strategy)`.
2. **Dispatch normalization.** Task 2 covers the `build_detection_for` signature change, the Rust-arm collapse, and the caller updates (`resolve_detection`, `detect_project`, `build_detection`).
3. **Tests.** No new tests — the top-level dispatch tests in `detect/mod.rs::tests` (e.g., `detect_node_ecosystem`, `detect_rust_ecosystem`) already exercise every ecosystem through the public API and catch any refactor regression. This matches the "no behavior change" scope rule.
4. **Clay's git workflow.** Every commit step writes `commit.txt` and waits for Clay to run `gtxt`. The PR is opened via `git pm`. No `git commit` calls from the worker.
5. **Test-running caution.** Per-task tests are narrowed to `cargo nextest run -p scrat-core detect::` (small, fast). The full workspace suite only runs at the end, and only after asking Clay first.
6. **Consistency.** Every sibling file uses the same template: module doc comment, imports block (`camino::Utf8Path`, `tracing::debug`, `super::has_binary`, `crate::ecosystem::*`), `pub(super) fn` with `_project_root: &Utf8Path` as the first parameter, function body verbatim from the original.

The plan is self-contained. A worker with zero project context can execute it by following each step literally.

---

## Execution handoff

Plan complete and saved to `record/superpowers/plans/2026-04-10-ecosystem-modules-phase-1-detect.md`. Two execution options:

**1. Subagent-Driven (recommended)** — I dispatch a fresh subagent per task, review between tasks, fast iteration. Best for catching scope drift early and keeping the main conversation context clean.

**2. Inline Execution** — Execute tasks in this session using executing-plans, batch execution with checkpoints. Best if you want to follow along step-by-step and make course corrections in real time.

Which approach?
