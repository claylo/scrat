# Ecosystem Modules Refactor — Phase 4: Unify into `ecosystem/`

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Collapse `bump/`, `deps/`, `detect/` directories and `preflight::check_registry_auth` into a single `ecosystem/<name>.rs` tree implementing a unified `EcosystemDriver` trait. Replace four runtime dispatch match tables with a single `Ecosystem::driver()` factory method. No behavior change.

**Architecture:** Per-ecosystem zero-sized unit struct drivers implement a 4-method trait (`detect`, `bump_version_files`, `parse_lockfile_diff`, `check_registry_auth`). Module layout splits types from driver machinery: `ecosystem/types.rs` holds the `Ecosystem` enum and data structs, `ecosystem/mod.rs` holds the trait + factory + shared lockfile-diff helpers at `pub(super)` visibility, and `ecosystem/<name>.rs` holds each driver's full implementation. Call sites collapse from 8-arm matches to `ecosystem.driver().method(...)` one-liners.

**Tech Stack:** Rust 2024 (scrat-core workspace crate). No new dependencies.

**Spec:** `record/superpowers/specs/2026-04-11-ecosystem-modules-phase-4-design.md`

---

## The full arc (context, not in scope for this plan)

| Phase | Goal | Output | Status |
|-------|------|--------|--------|
| **1** | Finish `detect/` split | `detect/{rust,node,go,php,python,ruby,swift}.rs` + normalized dispatch | **Complete** (PR #37, `0765242`) |
| **2** | Extract `bump/` with harmonized `BumpResult<Vec<String>>` return | `bump/{rust,node,php,python,ruby}.rs` per ecosystem; no trait (deliberate) | **Complete** (PR #38, `bbdd2ab`) |
| **3** | Extract `deps/` with `LockfileDiffParser` trait | `deps/{rust,node,go,php,python,ruby,swift}.rs`; first trait introduction | **Complete** (PR #40, `8c2cee3`) |
| **4 (this plan)** | Unify into `ecosystem/<name>.rs` with `EcosystemDriver` trait + absorb `check_registry_auth` | Single file per ecosystem; `bump/`, `deps/`, `detect/` directories collapsed; four match tables eliminated | **THIS PLAN — final phase of the arc** |

---

## Prerequisite (must land BEFORE Task 1)

The **comment-polish PR** addresses four refinement spots in Phase 3's `deps/` module surfaced during the final review. That PR must be merged to main before Phase 4 Task 1 starts. The polish scope:

- `crates/scrat-core/src/deps/node.rs:67-75` — add an inline comment linking the context-line fallback (which seeds `old_version` and `new_version` from an unchanged `"version":` line) to `deps/mod.rs:127-129` where `emit_change` suppresses equal from/to values.
- `crates/scrat-core/src/deps/mod.rs:194` — the `// ── JSON string extractor ───` divider is orphaned now that all per-ecosystem tests live in sibling files. Either add a matching `// ── TOML string extractor ───` divider above line 173 or drop the JSON one. Pick one and be consistent.
- `crates/scrat-core/src/deps/go.rs:27` and `crates/scrat-core/src/deps/ruby.rs:27` — currently use `content[1..]` and `&line[1..]` byte slicing. Rust/PHP/Swift use `.strip_prefix('-').or_else(|| line.strip_prefix('+'))`. Unify Go and Ruby to `strip_prefix` or document why they diverge.
- `crates/scrat-core/src/deps/mod.rs:23-49` — the `mod X;` and `pub use X::...;` pairs currently have a blank line between every pair. Group them (either all `mod` then all `pub use`, or pair them without blank lines).

Phase 4 Task 1 assumes this polish PR is merged. If it is NOT merged when Task 1 runs, stop and surface to Clay. Do not proceed.

---

## Out of scope for Phase 4

- **Any behavior change.** Every driver's detect probing, bump file-rewrite logic, lockfile diff state machine, and registry auth env-var check moves verbatim. No optimization, no "while I'm in there" cleanup, no refactoring beyond the unification itself.
- **The rest of `preflight.rs`.** Git status, release branch, remote sync, tool presence, gh auth, and tag availability checks are not ecosystem-specific. They stay in `preflight.rs` untouched. Only `check_registry_auth` is in scope.
- **Pluggable or runtime-registered drivers.** The `Ecosystem` enum stays closed. The `driver()` factory returns `&'static dyn EcosystemDriver` from a match statement — no registry, no map, no dynamic loading.
- **Touching `crates/scrat-core/src/pipeline.rs`, `crates/scrat-core/src/ship.rs`, `crates/scrat-core/src/notes.rs`, or any downstream consumer.** These files use the public API (`compute_deps`, `ReadyBump::execute`, `resolve_detection`) which stays signature-stable.
- **Adding new functionality.** No new trait methods beyond the four named. No new ecosystems (e.g., AgentSkill). No new config.
- **Fixing Node's "top-level dependencies only" parser behavior.** Intentional. The Phase 4 driver doc comment for Node must say "top-level only by design" but does NOT expand the parser.
- **Extracting shared TOML-package-diff helper for Python delegation.** Load-bearing decision preserved from Phase 3. `PythonDriver::parse_lockfile_diff` calls `RustDriver.parse_lockfile_diff(diff)` directly. Do NOT create a shared helper.
- **Changing `lockfile_path()` to a slice.** Every current ecosystem has exactly one canonical lockfile. Only `marker_file()` → `marker_files()` changes to slice shape in this plan.

---

## File structure after Phase 4

```
crates/scrat-core/src/
├── bump.rs          # Was bump/mod.rs. Orchestration only:
│                    #   plan_bump, plan_bump_with_detection, resolve_strategy,
│                    #   resolve_interactive, BumpPlan, ReadyBump, InteractiveBump,
│                    #   BumpError, BumpResult, BumpOutcome, generate_changelog.
│                    #   ReadyBump::execute dispatches via ecosystem.driver().
│                    #   25 public bump tests.
├── deps.rs          # Was deps/mod.rs. compute_deps only.
│                    #   Dispatch via ecosystem.driver().parse_lockfile_diff(&diff).
│                    #   0-2 optional dispatch smoke tests.
├── detect.rs        # Was detect/mod.rs. Coordinator + tool helpers:
│                    #   resolve_detection, detect_project, detect_ecosystem,
│                    #   build_detection, build_detection_for (dispatches via driver),
│                    #   detect_version_strategy, has_binary, check_tool_version,
│                    #   parse_version_from_output, ToolVersionCheck, MIN_GIT_CLIFF_VERSION.
│                    #   12 dispatch + tool-helper tests.
├── ecosystem/
│   ├── mod.rs       # EcosystemDriver trait definition +
│   │                #   pub(super) emit_change, extract_toml_string_value,
│   │                #   extract_json_string_value +
│   │                #   pub use types::* re-export +
│   │                #   4 shared-helper tests
│   ├── types.rs     # Ecosystem enum + Ecosystem::driver() factory +
│   │                #   Ecosystem::marker_files() method +
│   │                #   VersionStrategy, ChangelogTool, DetectedTools,
│   │                #   ProjectDetection structs +
│   │                #   10 types tests (display/serde/marker_files/bump_config)
│   ├── rust.rs      # RustDriver: 4-method impl + 11 tests
│   │                #   (3 detect + 7 deps + 1 preflight)
│   ├── node.rs      # NodeDriver: 4-method impl +
│   │                #   private helpers (extract_top_level_node_modules_name,
│   │                #   extract_json_version) + 16 tests (15 deps + 1 preflight)
│   ├── go.rs        # GoDriver: 4-method impl + 10 tests (9 deps + 1 preflight)
│   ├── php.rs       # PhpDriver: 4-method impl + 7 tests (deps only, no preflight arm)
│   ├── python.rs    # PythonDriver: 4-method impl + 4 tests (3 deps + 1 preflight)
│   │                #   parse_lockfile_diff DELEGATES to RustDriver — module-level
│   │                #   doc comment explains the incidental format match
│   ├── ruby.rs      # RubyDriver: 4-method impl + 26 tests (7 deps + 19 bump)
│   │                #   plus Ruby's private byte-walker helpers
│   ├── swift.rs     # SwiftDriver: 4-method impl + 7 tests (deps only)
│   └── generic.rs   # GenericDriver: 4-method impl with no-op returns +
│                    #   5 tests (1 preflight + 4 no-op contract)
├── preflight.rs     # check_registry_auth function DELETED; run_preflight calls
│                    #   det.ecosystem.driver().check_registry_auth() directly.
│                    #   ~53 tests (original minus 5 migrated registry_auth tests
│                    #   plus optional integration test)
└── ... (everything else unchanged)
```

Deleted directories: `crates/scrat-core/src/bump/`, `crates/scrat-core/src/deps/`, `crates/scrat-core/src/detect/`.

Deleted files (under those directories, 19 sibling files total):
- `bump/{rust,node,php,python,ruby}.rs` (5 files)
- `deps/{rust,node,go,php,python,ruby,swift}.rs` (7 files)
- `detect/{rust,node,go,php,python,ruby,swift}.rs` (7 files)

Deleted type: `crates/scrat-core/src/deps/mod.rs::LockfileDiffParser` trait (absorbed into `EcosystemDriver`).

Deleted function: `crates/scrat-core/src/preflight.rs::check_registry_auth` (absorbed into driver method).

---

## Test allocation

**Baseline captured at Task 1 Step 4:** 514 total scrat-core tests as of commit `8e8cf49` (or the commit-polish PR successor). Distribution across affected modules:

| Module | Current count | Phase 4 destination |
|---|---|---|
| `deps::rust::tests` | 7 | `ecosystem::rust::tests` |
| `deps::node::tests` | 15 | `ecosystem::node::tests` |
| `deps::go::tests` | 9 | `ecosystem::go::tests` |
| `deps::php::tests` | 7 | `ecosystem::php::tests` |
| `deps::python::tests` | 3 | `ecosystem::python::tests` |
| `deps::ruby::tests` | 7 | `ecosystem::ruby::tests` |
| `deps::swift::tests` | 7 | `ecosystem::swift::tests` |
| `deps::tests` (shared helpers) | 4 | `ecosystem::tests` (in `mod.rs`) |
| `bump::ruby::tests` (private Ruby bump) | 19 | `ecosystem::ruby::tests` |
| `bump::tests` (public bump) | 25 | `bump::tests` (in `bump.rs`) — unchanged |
| `detect::rust::tests` | 3 | `ecosystem::rust::tests` |
| `detect::tests` (dispatch + tool helpers) | 12 | `detect::tests` (in `detect.rs`) — unchanged |
| `ecosystem::tests` (current `ecosystem.rs::tests`) | 10 | `ecosystem::types::tests` |
| `preflight::tests::check_registry_auth_*` | 5 | Migrated to per-ecosystem files (see below) |

**Post-Phase-4 ecosystem file targets:**

| File | Sources | Expected count |
|---|---|---|
| `ecosystem::types::tests` | current `ecosystem.rs::tests` (10) | **10** |
| `ecosystem::tests` (in `mod.rs`) | `deps::tests` shared-helper tests (4: `extract_toml_string_value_basic`, `extract_toml_string_value_no_match`, `extract_json_string_value_basic`, `extract_json_string_value_no_match`) | **4** |
| `ecosystem::rust::tests` | `detect::rust::tests` (3) + `deps::rust::tests` (7) + `preflight::tests::check_registry_auth_rust` (1) | **11** |
| `ecosystem::node::tests` | `deps::node::tests` (15) + `preflight::tests::check_registry_auth_node` (1) | **16** |
| `ecosystem::go::tests` | `deps::go::tests` (9) + `preflight::tests::check_registry_auth_go_skips` (1) | **10** |
| `ecosystem::php::tests` | `deps::php::tests` (7) | **7** |
| `ecosystem::python::tests` | `deps::python::tests` (3) + `preflight::tests::check_registry_auth_python` (1) | **4** |
| `ecosystem::ruby::tests` | `deps::ruby::tests` (7) + `bump::ruby::tests` (19) | **26** |
| `ecosystem::swift::tests` | `deps::swift::tests` (7) | **7** |
| `ecosystem::generic::tests` | `preflight::tests::check_registry_auth_generic_skips` (1) + 4 new no-op contract tests | **5** |

**Post-Phase-4 orchestration file targets:**

| File | Sources | Expected count |
|---|---|---|
| `bump::tests` | current `bump::tests` (25 public) | **25** (unchanged) |
| `deps::tests` | possibly 0–2 new `compute_deps` dispatch smoke tests | **0–2** |
| `detect::tests` | current `detect::tests` (12 dispatch + tool helpers) | **12** (unchanged) |
| `preflight::tests` | current count minus 5 migrated registry_auth tests, plus 1 optional integration test verifying driver dispatch | **current − 5 + 0 or 1** |

**Total invariant:** workspace test count = baseline (514) + N, where N is the sum of new tests:
- `+4` for `ecosystem::generic::tests` no-op contract tests (REQUIRED, count is load-bearing)
- `+0 to +2` for `deps::tests` dispatch smoke tests (OPTIONAL, plan writer's call at Task 3)
- `+0 to +1` for `preflight::tests` integration test (OPTIONAL, plan writer's call at Task 6)

**Expected N ∈ [4, 7]**, so post-Phase-4 total is **518 to 521**. Task 7 Step 6 gates on this invariant.

---

## The `EcosystemDriver` trait

Defined in `crates/scrat-core/src/ecosystem/mod.rs`:

```rust
use camino::Utf8Path;
use semver::Version;

use crate::bump::BumpResult;
use crate::pipeline::DepChange;
use crate::preflight::CheckResult;
use crate::ecosystem::types::{ProjectDetection, VersionStrategy};

/// Per-ecosystem behavior for release workflows.
///
/// Implemented by a zero-sized unit struct per ecosystem
/// (`RustDriver`, `NodeDriver`, `GenericDriver`, …). The `&self` receiver
/// carries no state today but reserves the slot for per-ecosystem state
/// attachment (e.g., `RustDriver { bump_cmd }`) without changing signatures.
///
/// Static data (`marker_files`, `lockfile_path`, `bump_config`) stays on
/// [`Ecosystem`](crate::ecosystem::Ecosystem) — this trait owns *behavior*,
/// not pure data lookups.
pub trait EcosystemDriver {
    /// Build a [`ProjectDetection`] by probing `PATH` and assembling
    /// the smart-default tool commands for this ecosystem.
    fn detect(
        &self,
        project_root: &Utf8Path,
        version_strategy: VersionStrategy,
    ) -> ProjectDetection;

    /// Rewrite on-disk version files for this ecosystem.
    ///
    /// Returns the repo-relative paths of files that were actually
    /// modified. Returns an empty `Vec` for ecosystems where the version
    /// lives in git tags (Go, Swift) or there is no project file to
    /// rewrite (Generic).
    ///
    /// The `&ProjectDetection` argument is load-bearing for Rust, which
    /// reads `detection.tools.bump_cmd` to find `cargo set-version`.
    /// Other drivers currently ignore it, but the parameter is passed
    /// uniformly so future drivers can opt in without signature churn.
    fn bump_version_files(
        &self,
        project_root: &Utf8Path,
        version: &Version,
        detection: &ProjectDetection,
    ) -> BumpResult<Vec<String>>;

    /// Parse a unified diff of this ecosystem's lockfile into
    /// [`DepChange`] entries.
    ///
    /// Infallible by convention: malformed input returns an empty `Vec`
    /// rather than an error, matching the "deps diff failure is
    /// non-fatal" contract established by [`compute_deps`](crate::deps::compute_deps).
    /// Implementations must sort the result by `DepChange.name` for
    /// deterministic output.
    fn parse_lockfile_diff(&self, diff: &str) -> Vec<DepChange>;

    /// Check registry auth for the publish phase.
    ///
    /// Uses fast env-var checks (no network). Returns a pre-populated
    /// "no registry for this ecosystem" passing `CheckResult` for Go, PHP,
    /// Swift, and Generic.
    fn check_registry_auth(&self) -> CheckResult;
}
```

The trait is NOT defined all at once. It grows method-by-method across Tasks 3–6:

- Task 3 adds the trait with only `parse_lockfile_diff`. All 8 drivers get created with only this method implemented.
- Task 4 adds `detect` to the trait. All 8 drivers get the method body added.
- Task 5 adds `bump_version_files`. All 8 drivers get the method body added.
- Task 6 adds `check_registry_auth`. All 8 drivers get the method body added.

Growing the trait this way means each intermediate state compiles cleanly (the trait definition always matches the set of implementations).

---

## The `Ecosystem::driver()` factory

Defined in `crates/scrat-core/src/ecosystem/types.rs` alongside the `Ecosystem` enum (added in Task 3):

```rust
// Added to the existing `impl Ecosystem` block in ecosystem/types.rs

impl Ecosystem {
    /// Return the [`EcosystemDriver`] implementation for this ecosystem.
    ///
    /// Drivers are zero-sized unit structs; the returned reference is
    /// `'static` and incurs no allocation.
    pub fn driver(self) -> &'static dyn super::EcosystemDriver {
        match self {
            Self::Rust    => &super::rust::RustDriver,
            Self::Node    => &super::node::NodeDriver,
            Self::Go      => &super::go::GoDriver,
            Self::Php     => &super::php::PhpDriver,
            Self::Python  => &super::python::PythonDriver,
            Self::Ruby    => &super::ruby::RubyDriver,
            Self::Swift   => &super::swift::SwiftDriver,
            Self::Generic => &super::generic::GenericDriver,
        }
    }
}
```

---

## `Ecosystem::marker_files()` — singular → slice

In Task 2, the existing `marker_file(self) -> Option<&'static str>` method on `Ecosystem` becomes `marker_files(self) -> &'static [&'static str]` returning a slice:

```rust
impl Ecosystem {
    /// Filenames that signal this ecosystem when any of them is found
    /// in a directory.
    ///
    /// Returns a slice to support ecosystems where multiple marker files
    /// can indicate the same project type — for example, a future
    /// `AgentSkill` variant might match `plugin.json`,
    /// `.claude-plugin/plugin.json`, and `.bito.yaml`. Every current
    /// ecosystem returns a single-element slice; [`Generic`](Self::Generic)
    /// returns an empty slice.
    pub const fn marker_files(self) -> &'static [&'static str] {
        match self {
            Self::Rust => &["Cargo.toml"],
            Self::Node => &["package.json"],
            Self::Go => &["go.mod"],
            Self::Php => &["composer.json"],
            Self::Python => &["pyproject.toml"],
            Self::Ruby => &["Gemfile"],
            Self::Swift => &["Package.swift"],
            Self::Generic => &[],
        }
    }
}
```

The inline doc comment about `AgentSkill` is **load-bearing** — it explains why the slice shape exists when every current caller returns one element. Do NOT strip or abbreviate the comment.

---

## Python delegation (load-bearing, documented in code)

`ecosystem/python.rs` MUST contain a module-level doc comment explaining that `parse_lockfile_diff` delegation to `RustDriver` is **intentional and incidental**, not a shared abstraction commitment. This comment is transplanted verbatim from the current `deps/python.rs:1-11`:

```rust
//! Python ecosystem driver (`uv.lock`, `pyproject.toml`).
//!
//! `parse_lockfile_diff` delegates literally to
//! [`super::rust::RustDriver`] because `uv.lock` currently uses the
//! same TOML `[[package]]` format as `Cargo.lock`. This is NOT a
//! commitment to a shared "TOML package diff" abstraction — it's an
//! incidental format match. If uv diverges from Cargo's lockfile
//! format in a future release, this module grows its own state
//! machine and stops delegating. Do NOT extract a shared
//! TOML-package-diff helper on the assumption that Python and Rust
//! will always share an implementation.
```

The impl body for `parse_lockfile_diff` is exactly one line:

```rust
fn parse_lockfile_diff(&self, diff: &str) -> Vec<DepChange> {
    super::rust::RustDriver.parse_lockfile_diff(diff)
}
```

---

## Ruby's caller-side "no files modified" exception

The current `ReadyBump::execute` in `bump/mod.rs` has a Ruby-specific post-call block that depends on **both** the bump helper's return value AND `ReadyBump::version_files`:

```rust
Ecosystem::Ruby => {
    let files = ruby::bump_ruby_version(project_root, &self.next)?;
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
```

Phase 4 Task 5 **preserves this check in `bump.rs::ReadyBump::execute` post-dispatch**, not inside `RubyDriver::bump_version_files`. The trait method just returns `Vec<String>` — empty is a valid state for the driver. The caller enforces the release-correctness rule:

```rust
// Inside ReadyBump::execute, after Task 5:
let ecosystem_files = self.detection.ecosystem.driver()
    .bump_version_files(project_root, &self.next, &self.detection)?;

// Ruby-specific post-dispatch check: if the driver found nothing AND
// there are no [[version_files]] configured, the release would tag
// without updating any file. Block it. This is a caller-layer release-
// correctness rule, not a driver concern.
if self.detection.ecosystem == Ecosystem::Ruby
    && ecosystem_files.is_empty()
    && self.version_files.is_empty()
{
    return Err(BumpError::ToolFailed {
        tool: "ruby".into(),
        message: "no lib/**/version.rb or gemspec with a literal version \
                  was found, and no `[[version_files]]` entries are \
                  configured — the release would be tagged without \
                  updating any file"
            .into(),
    });
}

modified_files.extend(ecosystem_files);
```

Do NOT push this check into `RubyDriver`. Task 5's code-quality review MUST verify this stays in `bump.rs`.

---

## Per-driver quirks to preserve (anti-refactor list)

These are the asymmetries Phase 4 must preserve. A well-meaning implementer would be tempted to "fix" each of them; every one is load-bearing. Worker prompts for Tasks 3–6 must repeat the relevant items.

| # | Quirk | Rule |
|---|---|---|
| 1 | **`PythonDriver::parse_lockfile_diff` one-line delegates to `RustDriver.parse_lockfile_diff(diff)`** | Preserve. Add module-level doc comment (see Python delegation section). Do NOT extract a shared helper. |
| 2 | **`RustDriver::bump_version_files` reads `detection.tools.bump_cmd`** | Preserve. Keep `detection: &ProjectDetection` parameter on the trait method. |
| 3 | **Ruby's "no files modified" check stays in `bump.rs::ReadyBump::execute`, post-dispatch** | Preserve. Do NOT push into `RubyDriver::bump_version_files`. |
| 4 | **`NodeDriver::parse_lockfile_diff` reports top-level dependencies only** | Preserve. Driver doc comment says "top-level only by design." |
| 5 | **`GenericDriver` implements every method with no-op/empty returns** | Preserve. Call sites must NOT match on `Ecosystem::Generic` to skip the driver. |
| 6 | **`RustDriver::bump_version_files` returns `Err(BumpError::NoBumpTool)` when `detection.tools.bump_cmd.is_none()`** | Preserve. Rust-exclusive failure mode. |

---

## Conventions used in this plan

- **Commits via `commit.txt` — APPEND, do not overwrite:** Each task's commit step APPENDS a sub-bullet section to `commit.txt` at the repo root using the Edit tool. It does NOT overwrite. Clay runs `gtxt` (alias: `git commit -F commit.txt && rm commit.txt`) at a cadence of his choosing — possibly once after all 7 tasks, possibly between them. When `gtxt` runs, the entire accumulated `commit.txt` becomes one bundled commit, and the file is deleted. The next task that produces a commit re-creates the skeleton. The worker does NOT run `git commit` directly.
- **Bundled-commit format:** `commit.txt` follows the structure used in `f706dc9`, `974deb4`, `bbdd2ab`, and `8c2cee3` — one top-level subject line, a brief opening body paragraph, then multiple `* type(scope): subject` sub-bullet sections (each with its own body paragraph at column 0, NOT indented under the `*`). Each task contributes one sub-bullet section.
- **Task 1 intro paragraph is REAL, not a placeholder.** Phase 2's Task 2 commit.txt included a literal `[body to be appended as each task completes]` placeholder that leaked into the merged squash commit `bbdd2ab` as its second line. For Phase 4, the Task 1 opening paragraph is a real, complete sentence (see Task 1 Step 5 below). DO NOT include placeholder text anywhere in `commit.txt`.
- **Sandbox flag:** Every `cargo`, `cargo nextest`, and `cargo clippy` invocation in the Bash tool must use `dangerouslyDisableSandbox: true` because sccache fails under sandbox mode. This is not optional.
- **Test cadence:** Full workspace test runs are slow. Each task runs `cargo check -p scrat-core` and `cargo clippy -p scrat-core --all-targets -- -D warnings` (both fast). Running `cargo nextest run -p scrat-core ecosystem::` or narrower module scopes is fine — they run in well under a second. The full workspace suite runs only at Task 7 Step 5.
- **Narrow test scope per task:** At the end of each atomic task (Tasks 3–6), run `cargo nextest run -p scrat-core ecosystem::` to exercise the extracted module's tests, then `cargo nextest run -p scrat-core` to confirm no cross-module regression in the scrat-core crate.
- **Branch:** `feat/ecosystem-modules-phase-4`. Created fresh at Task 1 — NOT continued from the `docs/ecosystem-modules-phase-4-design` branch where the spec landed. That branch will have been merged to main before Task 1 runs.
- **Module-level `use` imports only.** Every driver file places all `use` statements at module level, never inside function bodies. The only exception is inline `use` inside `#[cfg(test)] mod tests` functions.
- **Preserve existing imports exactly.** When moving code, lift any inline `use` statements to module level in the destination file.
- **Review optimization (validated thrice — Phases 1, 2, 3):** Spec review is dispatched or inline-held on every task. Code-quality review runs ONLY on (a) Task 3, the biggest extraction and trait-establishing task; (b) Task 5, the second-biggest with semantic asymmetries (Ruby caller-side check, Rust `&detection`); and (c) Task 7, the final PR. Middle tasks (2, 4, 6) get spec review only. This saves ~3 code-quality dispatches per phase without losing coverage.
- **Atomic task discipline:** Tasks 2-6 are each single-commit atomic units where intermediate substeps do NOT compile. Do not stop partway. Do not split across commits. Run `cargo check` only at the END of each task unless a sub-step explicitly says otherwise.
- **Mixed-bash cascade awareness:** Do not chain existence-check commands (`ls commit.txt 2>&1`) with other bash calls in the same parallel batch. Exit 1 from a failed existence check will cancel the sibling bash commands. Use `|| true` or isolate the check into its own call.

---

### Task 1: Branch setup and baseline capture

**Files:**
- None (git + verification + one file write)

- [ ] **Step 1: Confirm prerequisite PR is merged**

Run:
```bash
git log --oneline origin/main -5
```
Expected: one of the recent commits is the comment-polish PR that addressed the four refinement spots in Phase 3's `deps/` module (see the Prerequisite section above). If you cannot find such a commit, STOP and surface to Clay before continuing.

- [ ] **Step 2: Confirm working tree is clean on main**

Run:
```bash
git checkout main && git pull && git status
```
Expected: `On branch main`, `Your branch is up to date with 'origin/main'.`, `nothing to commit, working tree clean`.

- [ ] **Step 3: Create feature branch**

Run:
```bash
git checkout -b feat/ecosystem-modules-phase-4
```
Expected: `Switched to a new branch 'feat/ecosystem-modules-phase-4'`.

- [ ] **Step 4: Capture test count baseline**

Run (with `dangerouslyDisableSandbox: true`):
```bash
cargo nextest list -p scrat-core 2>&1 | grep -E "^scrat-core " | wc -l
```
Expected: integer output. Record the exact number (the plan assumes **514** based on commit `8e8cf49`; if the prerequisite polish PR added or removed tests, the number may differ slightly). You will compare against this at Task 7 Step 6.

Also run (with `dangerouslyDisableSandbox: true`):
```bash
cargo nextest run -p scrat-core
```
Expected: all tests pass. The exact count should match Step 4's list output. If any tests fail or the counts differ, STOP and investigate — the baseline is not clean.

- [ ] **Step 5: Write `commit.txt` with real intro paragraph**

Create `commit.txt` at the repo root with this EXACT content (note: the opening paragraph is a real complete sentence — it is NOT a placeholder):

```
refactor(ecosystem): unify bump, deps, detect into ecosystem/ with EcosystemDriver trait

Phase 4 (final) of the ecosystem-modules refactor. Collapses bump/,
deps/, and detect/ directories plus preflight::check_registry_auth
into a single ecosystem/<name>.rs file per ecosystem implementing a
unified EcosystemDriver trait. Four runtime dispatch match tables
collapse to Ecosystem::driver().method() one-liners. Completes the
arc started with Phase 1 (detect/, PR #37), Phase 2 (bump/, PR #38),
and Phase 3 (deps/ + LockfileDiffParser trait, PR #40).

No behavior change. Every driver's detect probing, bump file-rewrite
logic, lockfile diff state machine, and registry auth env-var check
moves verbatim. Ruby's caller-side "no files modified" exception
stays in ReadyBump::execute post-dispatch as a release-correctness
rule, not inside RubyDriver.
```

- [ ] **Step 6: Verify commit.txt content**

Run:
```bash
cat commit.txt
```
Expected: the exact text from Step 5. Confirm there is no placeholder text anywhere (no `[...]`, no `TBD`, no `TODO`).

- [ ] **Step 7: Stop — do NOT commit yet**

Do NOT run `git commit` or `gtxt`. `commit.txt` accumulates across all 7 tasks. Clay runs `gtxt` at his discretion, possibly only once at the end.

---

### Task 2: Module layout split — `ecosystem.rs` → `ecosystem/{mod.rs, types.rs}` + `marker_files()` (ATOMIC)

**Files:**
- Move: `crates/scrat-core/src/ecosystem.rs` → `crates/scrat-core/src/ecosystem/types.rs` (via `git mv`)
- Create: `crates/scrat-core/src/ecosystem/mod.rs` (new)
- Modify: `crates/scrat-core/src/detect/mod.rs:86-95` (update `detect_ecosystem` loop for slice)
- Modify: `crates/scrat-core/src/ecosystem/types.rs:56-67` (rename `marker_file` → `marker_files`, return slice)
- Modify: `crates/scrat-core/src/ecosystem/types.rs:243-245` (update `ecosystem_marker_files` test assertions for slice shape)

**This task is atomic.** The file move, new `mod.rs` creation, `marker_file` → `marker_files` rename, and `detect_ecosystem` loop update must happen together. Intermediate substeps may not compile.

No trait is introduced in this task. No driver files are created. The goal is to establish the `ecosystem/` directory layout and the multi-marker support, leaving everything else untouched.

- [ ] **Step 1: Create the `ecosystem/` directory**

Run:
```bash
mkdir crates/scrat-core/src/ecosystem
```
Expected: directory created silently.

- [ ] **Step 2: Move `ecosystem.rs` into the new directory as `types.rs`**

Run:
```bash
git mv crates/scrat-core/src/ecosystem.rs crates/scrat-core/src/ecosystem/types.rs
```
Expected: `git mv` runs silently. `git status` should now show:
```
renamed:    crates/scrat-core/src/ecosystem.rs -> crates/scrat-core/src/ecosystem/types.rs
```

- [ ] **Step 3: Create `ecosystem/mod.rs` with a minimal re-export**

Create `crates/scrat-core/src/ecosystem/mod.rs` with this content:

```rust
//! Ecosystem types, drivers, and smart defaults for release workflows.
//!
//! This module groups per-ecosystem logic (detection, version bumping,
//! dependency diff parsing, registry auth) behind the [`EcosystemDriver`]
//! trait. Types live in `types.rs`; each per-ecosystem driver lives in
//! its own file (e.g., `rust.rs`, `node.rs`). `EcosystemDriver` and the
//! `Ecosystem::driver()` factory grow across Phase 4's tasks.

mod types;

pub use types::*;
```

At this stage `types.rs` still contains the entire verbatim content of the old `ecosystem.rs`. No trait, no driver sibling files, nothing else.

- [ ] **Step 4: Rename `marker_file` to `marker_files` in `ecosystem/types.rs`**

Edit `crates/scrat-core/src/ecosystem/types.rs` lines 52-67. Find this block:

```rust
impl Ecosystem {
    /// Filename that signals this ecosystem when found in a directory.
    ///
    /// Returns `None` for [`Generic`](Self::Generic) which has no marker file.
    pub const fn marker_file(self) -> Option<&'static str> {
        match self {
            Self::Rust => Some("Cargo.toml"),
            Self::Node => Some("package.json"),
            Self::Go => Some("go.mod"),
            Self::Php => Some("composer.json"),
            Self::Python => Some("pyproject.toml"),
            Self::Ruby => Some("Gemfile"),
            Self::Swift => Some("Package.swift"),
            Self::Generic => None,
        }
    }
```

Replace with:

```rust
impl Ecosystem {
    /// Filenames that signal this ecosystem when any of them is found
    /// in a directory.
    ///
    /// Returns a slice to support ecosystems where multiple marker files
    /// can indicate the same project type — for example, a future
    /// `AgentSkill` variant might match `plugin.json`,
    /// `.claude-plugin/plugin.json`, and `.bito.yaml`. Every current
    /// ecosystem returns a single-element slice; [`Generic`](Self::Generic)
    /// returns an empty slice.
    pub const fn marker_files(self) -> &'static [&'static str] {
        match self {
            Self::Rust => &["Cargo.toml"],
            Self::Node => &["package.json"],
            Self::Go => &["go.mod"],
            Self::Php => &["composer.json"],
            Self::Python => &["pyproject.toml"],
            Self::Ruby => &["Gemfile"],
            Self::Swift => &["Package.swift"],
            Self::Generic => &[],
        }
    }
```

The load-bearing doc comment about `AgentSkill` is mandatory. Do NOT abbreviate or strip it — it documents why the slice shape exists when every current caller returns one element.

- [ ] **Step 5: Update the `ecosystem_marker_files` test assertions**

In `crates/scrat-core/src/ecosystem/types.rs` test module, find the test function (around lines 241-246):

```rust
    #[test]
    fn ecosystem_marker_files() {
        assert_eq!(Ecosystem::Rust.marker_file(), Some("Cargo.toml"));
        assert_eq!(Ecosystem::Node.marker_file(), Some("package.json"));
        assert_eq!(Ecosystem::Generic.marker_file(), None);
    }
```

Replace with:

```rust
    #[test]
    fn ecosystem_marker_files() {
        assert_eq!(Ecosystem::Rust.marker_files(), &["Cargo.toml"]);
        assert_eq!(Ecosystem::Node.marker_files(), &["package.json"]);
        assert_eq!(Ecosystem::Generic.marker_files(), &[] as &[&str]);
    }
```

The `&[] as &[&str]` cast is needed because the empty slice literal needs a type hint for the comparison.

- [ ] **Step 6: Update `detect::detect_ecosystem` to loop the inner slice**

Edit `crates/scrat-core/src/detect/mod.rs` lines 82-95. Find the function:

```rust
/// Identify the ecosystem by scanning for marker files.
///
/// Only checks [`Ecosystem::AUTO_DETECTABLE`] variants (those with marker
/// files). [`Ecosystem::Generic`] is never auto-detected.
fn detect_ecosystem(project_root: &Utf8Path) -> Option<Ecosystem> {
    for ecosystem in Ecosystem::AUTO_DETECTABLE {
        if let Some(marker) = ecosystem.marker_file()
            && project_root.join(marker).is_file()
        {
            return Some(*ecosystem);
        }
    }
    None
}
```

Replace with:

```rust
/// Identify the ecosystem by scanning for marker files.
///
/// Only checks [`Ecosystem::AUTO_DETECTABLE`] variants (those with marker
/// files). [`Ecosystem::Generic`] is never auto-detected. An ecosystem
/// that returns multiple marker files from [`Ecosystem::marker_files`]
/// matches on the first marker present in `project_root`.
fn detect_ecosystem(project_root: &Utf8Path) -> Option<Ecosystem> {
    for ecosystem in Ecosystem::AUTO_DETECTABLE {
        for marker in ecosystem.marker_files() {
            if project_root.join(marker).is_file() {
                return Some(*ecosystem);
            }
        }
    }
    None
}
```

- [ ] **Step 7: Verify no other callers of `marker_file` exist**

Run:
```bash
rg 'marker_file\b' crates/
```
Expected: the only hit is `marker_files` (the new plural name) or inside this task's edits. If any call to `marker_file()` (singular) remains, fix it by updating to `marker_files()` + taking `.first()` or iterating, matching the context. The design-time audit found exactly 5 singular callers: 1 definition, 3 test assertions, 1 `detect_ecosystem` loop — all handled in this task's prior steps.

- [ ] **Step 8: Verify compilation**

Run (with `dangerouslyDisableSandbox: true`):
```bash
cargo check -p scrat-core
```
Expected: clean build, exit 0. If this fails, the task's atomic unit is broken — inspect errors and fix before proceeding.

- [ ] **Step 9: Verify clippy is clean**

Run (with `dangerouslyDisableSandbox: true`):
```bash
cargo clippy -p scrat-core --all-targets -- -D warnings
```
Expected: no warnings, exit 0.

- [ ] **Step 10: Run targeted tests**

Run (with `dangerouslyDisableSandbox: true`):
```bash
cargo nextest run -p scrat-core ecosystem:: detect::
```
Expected: all tests pass. `ecosystem::` tests should still number 10 (from the old `ecosystem.rs::tests` module, now at `ecosystem::types::tests`). `detect::` tests should still number 15.

- [ ] **Step 11: Run full scrat-core test suite**

Run (with `dangerouslyDisableSandbox: true`):
```bash
cargo nextest run -p scrat-core
```
Expected: 514 tests pass (or whatever the Task 1 Step 4 baseline reported). No regressions.

- [ ] **Step 12: Append Task 2 section to `commit.txt`**

Use the Edit tool to APPEND to `commit.txt` (do NOT overwrite). Find the last line of the existing content and add a blank line, then the new section. The new content to append:

```

* refactor(ecosystem): split ecosystem.rs into ecosystem/{mod.rs, types.rs}

Pure filesystem restructure plus marker_file → marker_files (Option
→ slice). The git mv preserves blame history for the types module.
ecosystem/mod.rs is a three-line file that declares and re-exports
types — it will grow the EcosystemDriver trait and driver dispatch
in subsequent tasks.

marker_files returns a slice to support future multi-marker
ecosystems (e.g., AgentSkill variants matching plugin.json,
.claude-plugin/plugin.json, or .bito.yaml). Every current
ecosystem returns a single-element slice; Generic returns empty.
The change is fully mechanical at call sites — the only caller is
detect::detect_ecosystem, which now loops the inner slice.

No behavior change. All 514 scrat-core tests still pass.
```

- [ ] **Step 13: Stop — do NOT commit yet**

`commit.txt` now contains Task 1's intro paragraph plus Task 2's sub-bullet section. Continue to Task 3.

---

### Task 3: Introduce `EcosystemDriver` trait with `parse_lockfile_diff` + migrate `deps/` (ATOMIC — BIGGEST TASK — code quality review required)

**Files:**
- Modify: `crates/scrat-core/src/ecosystem/mod.rs` (add trait + shared helpers + tests)
- Modify: `crates/scrat-core/src/ecosystem/types.rs` (add `Ecosystem::driver()` factory)
- Create: `crates/scrat-core/src/ecosystem/rust.rs` (new)
- Create: `crates/scrat-core/src/ecosystem/node.rs` (new)
- Create: `crates/scrat-core/src/ecosystem/go.rs` (new)
- Create: `crates/scrat-core/src/ecosystem/php.rs` (new)
- Create: `crates/scrat-core/src/ecosystem/python.rs` (new)
- Create: `crates/scrat-core/src/ecosystem/ruby.rs` (new)
- Create: `crates/scrat-core/src/ecosystem/swift.rs` (new)
- Create: `crates/scrat-core/src/ecosystem/generic.rs` (new)
- Modify: `crates/scrat-core/src/deps/mod.rs` (delete `LockfileDiffParser` trait, update `compute_deps` dispatch)
- Delete: `crates/scrat-core/src/deps/{rust,node,go,php,python,ruby,swift}.rs` (7 files)
- Move: `crates/scrat-core/src/deps/mod.rs` → `crates/scrat-core/src/deps.rs` (via `git mv`)
- Delete: `crates/scrat-core/src/deps/` directory

**This task is atomic and is the single largest unit in the refactor.** All driver files must be created and the trait must be fully wired in one atomic change. Substeps do not compile in isolation.

**Review:** Requires code-quality review (dispatched subagent) after completion. This task establishes the driver pattern that Tasks 4–6 will replicate.

**Anti-refactors for this task (worker prompt must repeat):**
- `PythonDriver::parse_lockfile_diff` MUST delegate to `RustDriver.parse_lockfile_diff(diff)` as a one-line impl. Module-level doc comment explains why. Do NOT extract a shared helper.
- `NodeDriver::parse_lockfile_diff` reports top-level dependencies only. Driver doc comment says "top-level only by design."
- `GenericDriver::parse_lockfile_diff` returns `Vec::new()`. It is a full driver, not a special-cased null.

- [ ] **Step 1: Add `EcosystemDriver` trait (with only `parse_lockfile_diff`) and shared helpers to `ecosystem/mod.rs`**

Replace the entire content of `crates/scrat-core/src/ecosystem/mod.rs` with:

```rust
//! Ecosystem types, drivers, and smart defaults for release workflows.
//!
//! This module groups per-ecosystem logic (detection, version bumping,
//! dependency diff parsing, registry auth) behind the [`EcosystemDriver`]
//! trait. Types live in `types.rs`; each per-ecosystem driver lives in
//! its own file (e.g., `rust.rs`, `node.rs`).
//!
//! The trait grows across Phase 4's tasks. At this task's completion it
//! contains only `parse_lockfile_diff`. Subsequent tasks add `detect`,
//! `bump_version_files`, and `check_registry_auth` methods.

mod types;

pub mod generic;
pub mod go;
pub mod node;
pub mod php;
pub mod python;
pub mod ruby;
pub mod rust;
pub mod swift;

pub use types::*;

use crate::pipeline::DepChange;

/// Per-ecosystem behavior for release workflows.
///
/// Implemented by a zero-sized unit struct per ecosystem
/// (`RustDriver`, `NodeDriver`, `GenericDriver`, …). The `&self` receiver
/// carries no state today but reserves the slot for per-ecosystem state
/// attachment (e.g., `RustDriver { bump_cmd }`) without changing signatures.
///
/// Static data (`marker_files`, `lockfile_path`, `bump_config`) stays on
/// [`Ecosystem`] — this trait owns *behavior*, not pure data lookups.
pub trait EcosystemDriver {
    /// Parse a unified diff of this ecosystem's lockfile into
    /// [`DepChange`] entries.
    ///
    /// Infallible by convention: malformed input returns an empty `Vec`
    /// rather than an error, matching the existing "deps diff failure is
    /// non-fatal" contract established by [`compute_deps`](crate::deps::compute_deps).
    /// Implementations must sort the result by `DepChange.name` for
    /// deterministic output.
    fn parse_lockfile_diff(&self, diff: &str) -> Vec<DepChange>;
}

// ─── Shared lockfile-diff helpers ────────────────────────────────

/// Emit a `DepChange` if we have a name and at least one version.
///
/// Skips if both versions are present but equal (no actual change).
pub(super) fn emit_change(
    changes: &mut Vec<DepChange>,
    name: &Option<String>,
    old_version: &Option<String>,
    new_version: &Option<String>,
) {
    let Some(name) = name else { return };

    // Need at least one version to be interesting
    if old_version.is_none() && new_version.is_none() {
        return;
    }

    // Skip if versions are equal (no change)
    if old_version.is_some() && old_version == new_version {
        return;
    }

    changes.push(DepChange {
        name: name.clone(),
        from: old_version.clone(),
        to: new_version.clone(),
    });
}

/// Extract a TOML string value for a given key.
///
/// Matches lines like `key = "value"` and returns `value`.
pub(super) fn extract_toml_string_value(line: &str, key: &str) -> Option<String> {
    let line = line.trim();
    let rest = line.strip_prefix(key)?;
    let rest = rest.trim_start();
    let rest = rest.strip_prefix('=')?;
    let rest = rest.trim_start();
    let rest = rest.strip_prefix('"')?;
    let value = rest.strip_suffix('"')?;
    Some(value.to_string())
}

/// Extract a JSON string value for a given key.
///
/// Matches lines like `"key": "value"` or `"key": "value",` and returns `value`.
pub(super) fn extract_json_string_value(line: &str, key: &str) -> Option<String> {
    let line = line.trim();
    let rest = line.strip_prefix('"')?;
    let rest = rest.strip_prefix(key)?;
    let rest = rest.strip_prefix('"')?;
    let rest = rest.trim_start();
    let rest = rest.strip_prefix(':')?;
    let rest = rest.trim_start();
    let rest = rest.strip_prefix('"')?;
    let value = rest.strip_suffix(',').unwrap_or(rest);
    let value = value.strip_suffix('"')?;
    Some(value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_toml_string_value_basic() {
        assert_eq!(
            extract_toml_string_value(r#"name = "serde""#, "name"),
            Some("serde".into())
        );
        assert_eq!(
            extract_toml_string_value(r#"version = "1.0.0""#, "version"),
            Some("1.0.0".into())
        );
    }

    #[test]
    fn extract_toml_string_value_no_match() {
        assert_eq!(
            extract_toml_string_value(r#"source = "registry""#, "name"),
            None
        );
        assert_eq!(extract_toml_string_value("not a toml line", "name"), None);
    }

    #[test]
    fn extract_json_string_value_basic() {
        assert_eq!(
            extract_json_string_value(r#""name": "vendor/lib""#, "name"),
            Some("vendor/lib".into())
        );
        assert_eq!(
            extract_json_string_value(r#""version": "1.0.0","#, "version"),
            Some("1.0.0".into())
        );
    }

    #[test]
    fn extract_json_string_value_no_match() {
        assert_eq!(
            extract_json_string_value(r#""source": "git""#, "name"),
            None
        );
        assert_eq!(extract_json_string_value("not a json line", "name"), None);
    }
}
```

- [ ] **Step 2: Add `Ecosystem::driver()` factory to `ecosystem/types.rs`**

Open `crates/scrat-core/src/ecosystem/types.rs`. Find the existing `impl Ecosystem { ... }` block (the one containing `marker_files`, `lockfile_path`, `AUTO_DETECTABLE`, `ALL`, `bump_config`). Append a new inherent method at the END of that impl block (before its closing `}`):

```rust
    /// Return the [`EcosystemDriver`](super::EcosystemDriver) implementation
    /// for this ecosystem.
    ///
    /// Drivers are zero-sized unit structs; the returned reference is
    /// `'static` and incurs no allocation.
    pub fn driver(self) -> &'static dyn super::EcosystemDriver {
        match self {
            Self::Rust    => &super::rust::RustDriver,
            Self::Node    => &super::node::NodeDriver,
            Self::Go      => &super::go::GoDriver,
            Self::Php     => &super::php::PhpDriver,
            Self::Python  => &super::python::PythonDriver,
            Self::Ruby    => &super::ruby::RubyDriver,
            Self::Swift   => &super::swift::SwiftDriver,
            Self::Generic => &super::generic::GenericDriver,
        }
    }
```

The match arm formatting mirrors Phase 3's `compute_deps` dispatch style.

- [ ] **Step 3: Create `ecosystem/rust.rs` by transplanting `deps/rust.rs` + renaming**

Read the current `crates/scrat-core/src/deps/rust.rs` to get the full file content. Create `crates/scrat-core/src/ecosystem/rust.rs` with this content (adapt structure to match what was in `deps/rust.rs`, renaming `RustLockfileParser` to `RustDriver` and `impl LockfileDiffParser` to `impl EcosystemDriver`, and updating the method name from `parse_diff` to `parse_lockfile_diff`):

```rust
//! Rust ecosystem driver (Cargo.toml / Cargo.lock).
//!
//! Implements [`EcosystemDriver`] for Rust projects. Currently wires
//! `parse_lockfile_diff` against `Cargo.lock` as a TOML state machine
//! on `[[package]]` blocks. `detect`, `bump_version_files`, and
//! `check_registry_auth` will be added in Tasks 4, 5, and 6
//! respectively.

use super::{EcosystemDriver, emit_change, extract_toml_string_value};
use crate::pipeline::DepChange;

/// Rust ecosystem driver.
pub struct RustDriver;

impl EcosystemDriver for RustDriver {
    /// Parse a unified diff of `Cargo.lock` into dependency changes.
    ///
    /// State machine tracking per-`[[package]]` blocks:
    /// - `name` from any `name = "..."` line (context, removed, or added)
    /// - `old_version` from `-version = "..."` lines
    /// - `new_version` from `+version = "..."` lines
    ///
    /// At each `[[package]]` boundary or EOF, emits a [`DepChange`] if
    /// we have a name and at least one version that changed.
    fn parse_lockfile_diff(&self, diff: &str) -> Vec<DepChange> {
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

    // All 7 tests from deps/rust.rs::tests copied verbatim, updated to
    // call RustDriver.parse_lockfile_diff(diff) instead of
    // RustLockfileParser.parse_diff(diff).

    // parse_cargo_lock_diff_update
    // parse_cargo_lock_diff_added
    // parse_cargo_lock_diff_removed
    // parse_cargo_lock_diff_mixed
    // parse_cargo_lock_diff_empty
    // parse_cargo_lock_diff_no_version_change
    // parse_cargo_lock_diff_sorted
}
```

Then copy the 7 test functions verbatim from the current `deps/rust.rs::tests` module into `ecosystem/rust.rs::tests`, replacing `RustLockfileParser.parse_diff(diff)` with `RustDriver.parse_lockfile_diff(diff)`.

- [ ] **Step 4: Create `ecosystem/node.rs` by transplanting `deps/node.rs` + renaming**

Read the current `crates/scrat-core/src/deps/node.rs`. Create `crates/scrat-core/src/ecosystem/node.rs` following the same pattern as Step 3:

1. Module doc comment describes this as "Node ecosystem driver (`package.json` / `package-lock.json`)" and notes that `parse_lockfile_diff` reports **top-level dependencies only** — this is intentional by design, NOT a stub or a bug to fix.
2. `pub struct NodeDriver;`
3. `impl EcosystemDriver for NodeDriver { fn parse_lockfile_diff(...) { /* ...body from deps/node.rs... */ } }` — method body copied verbatim from `deps/node.rs::NodeLockfileParser::parse_diff`.
4. Private helpers `extract_top_level_node_modules_name` and `extract_json_version` travel as private `fn` items in the same file (they are Node-exclusive, not shared primitives).
5. Test module with all 15 tests from `deps/node.rs::tests` copied verbatim, replacing `NodeLockfileParser.parse_diff(...)` with `NodeDriver.parse_lockfile_diff(...)`. Also copy the 4 `extract_top_level_node_modules_name_*` tests and 2 `extract_json_version_*` tests verbatim (they test private helpers that moved with the driver).
6. Imports include `use super::{EcosystemDriver, emit_change, extract_json_string_value}; use crate::pipeline::DepChange;` and any other imports needed for the parser body.

The critical doc comment for Node's driver module:

```rust
//! Node ecosystem driver (`package.json` / `package-lock.json`).
//!
//! `parse_lockfile_diff` reports **top-level dependencies only** —
//! this is intentional. npm lockfile v2/v3 carries thousands of
//! transitive entries; release notes want direct deps. The parser
//! walks top-level `packages` entries via a JSON state machine and
//! ignores nested `node_modules/*` subpackages. This is NOT a stub
//! or partial implementation; it is the design.
```

- [ ] **Step 5: Create `ecosystem/go.rs` by transplanting `deps/go.rs` + renaming**

Read `crates/scrat-core/src/deps/go.rs`. Create `crates/scrat-core/src/ecosystem/go.rs` following the same pattern: module doc comment, `pub struct GoDriver;`, `impl EcosystemDriver for GoDriver`, method body copied verbatim from `GoLockfileParser::parse_diff`, 9 tests copied verbatim with the `GoLockfileParser.parse_diff(...)` → `GoDriver.parse_lockfile_diff(...)` rename.

Module doc comment:

```rust
//! Go ecosystem driver (`go.mod`).
//!
//! `parse_lockfile_diff` walks `go.mod` as a line-oriented
//! collect-and-merge pass, tracking `require` and `replace` entries.
//! Go version bumping is a no-op — versions live in git tags, not
//! in any file that scrat rewrites.
```

- [ ] **Step 6: Create `ecosystem/php.rs` by transplanting `deps/php.rs` + renaming**

Read `crates/scrat-core/src/deps/php.rs`. Create `crates/scrat-core/src/ecosystem/php.rs` following the pattern: `pub struct PhpDriver;`, method body copied verbatim from `PhpLockfileParser::parse_diff`, 7 tests copied verbatim with renaming.

Module doc comment:

```rust
//! PHP ecosystem driver (`composer.json` / `composer.lock`).
//!
//! `parse_lockfile_diff` walks `composer.lock` as a JSON state
//! machine keyed on `"name"` (namespaced vendor/package form).
```

- [ ] **Step 7: Create `ecosystem/python.rs` with delegation to `RustDriver`**

Read `crates/scrat-core/src/deps/python.rs`. Create `crates/scrat-core/src/ecosystem/python.rs` with this EXACT content:

```rust
//! Python ecosystem driver (`pyproject.toml` / `uv.lock`).
//!
//! `parse_lockfile_diff` delegates literally to
//! [`super::rust::RustDriver`] because `uv.lock` currently uses the
//! same TOML `[[package]]` format as `Cargo.lock`. This is NOT a
//! commitment to a shared "TOML package diff" abstraction — it's an
//! incidental format match. If uv diverges from Cargo's lockfile
//! format in a future release, this module grows its own state
//! machine and stops delegating. Do NOT extract a shared
//! TOML-package-diff helper on the assumption that Python and Rust
//! will always share an implementation.

use super::EcosystemDriver;
use crate::pipeline::DepChange;

/// Python ecosystem driver.
pub struct PythonDriver;

impl EcosystemDriver for PythonDriver {
    /// Delegates to [`super::rust::RustDriver::parse_lockfile_diff`]
    /// because `uv.lock` uses the same TOML `[[package]]` format as
    /// `Cargo.lock`. See the module-level doc comment for the
    /// landmine this comment protects against.
    fn parse_lockfile_diff(&self, diff: &str) -> Vec<DepChange> {
        super::rust::RustDriver.parse_lockfile_diff(diff)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 3 tests from deps/python.rs::tests copied verbatim, updated to
    // call PythonDriver.parse_lockfile_diff(diff):
    // - parse_uv_lock_diff_update
    // - parse_uv_lock_diff_added
    // - parse_uv_lock_diff_skips_header
}
```

Copy the 3 test functions from `deps/python.rs::tests` verbatim, replacing `PythonLockfileParser.parse_diff(...)` with `PythonDriver.parse_lockfile_diff(...)`.

**The one-line delegation body and the module-level doc comment are load-bearing.** Do NOT rewrite to inline the state machine. Do NOT extract a shared helper.

- [ ] **Step 8: Create `ecosystem/ruby.rs` by transplanting `deps/ruby.rs` + renaming**

Read `crates/scrat-core/src/deps/ruby.rs`. Create `crates/scrat-core/src/ecosystem/ruby.rs` with the driver pattern and 7 tests.

Module doc comment:

```rust
//! Ruby ecosystem driver (`Gemfile` / `Gemfile.lock`).
//!
//! `parse_lockfile_diff` walks `Gemfile.lock` as a collect-and-merge
//! pass on 4-space-indented gem lines. Gem version bumping (added
//! in Task 5) walks `lib/**/version.rb` files and gemspec literal
//! assignments.
```

Note that this task only adds `parse_lockfile_diff`. The Ruby bump logic from `bump/ruby.rs` (548 LOC, 19 private tests) is NOT added until Task 5.

- [ ] **Step 9: Create `ecosystem/swift.rs` by transplanting `deps/swift.rs` + renaming**

Read `crates/scrat-core/src/deps/swift.rs`. Create `crates/scrat-core/src/ecosystem/swift.rs` with the driver pattern and 7 tests.

Module doc comment:

```rust
//! Swift ecosystem driver (`Package.swift` / `Package.resolved`).
//!
//! `parse_lockfile_diff` walks `Package.resolved` as a JSON state
//! machine keyed on `"identity"`. Swift version bumping is a no-op
//! — versions live in git tags.
```

- [ ] **Step 10: Create `ecosystem/generic.rs` with no-op `parse_lockfile_diff`**

Create `crates/scrat-core/src/ecosystem/generic.rs` with this content:

```rust
//! Generic ecosystem driver.
//!
//! A first-class driver with no-op/empty implementations of every
//! [`EcosystemDriver`] method. Generic is selected interactively
//! when auto-detection finds no marker files, or via
//! `project.type = "generic"` in config. It skips ecosystem-specific
//! behavior but still participates in changelog, git commit/tag/push,
//! GitHub release, and hook execution.
//!
//! Call sites must NOT match on `Ecosystem::Generic` to skip the
//! driver — they trust the no-op bodies here. This is the pattern
//! that makes `ecosystem.driver().method(...)` dispatch uniform
//! across all variants.

use super::EcosystemDriver;
use crate::pipeline::DepChange;

/// Generic ecosystem driver.
pub struct GenericDriver;

impl EcosystemDriver for GenericDriver {
    /// No lockfile — returns an empty `Vec`.
    fn parse_lockfile_diff(&self, _diff: &str) -> Vec<DepChange> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generic_parse_lockfile_diff_returns_empty() {
        let changes = GenericDriver.parse_lockfile_diff("any input");
        assert!(changes.is_empty());
    }
}
```

The `detect`, `bump_version_files`, and `check_registry_auth` impls are added in Tasks 4, 5, and 6 respectively. The additional 3 no-op contract tests for those methods also land in Tasks 4, 5, and 6.

- [ ] **Step 11: Update `deps/mod.rs::compute_deps` to dispatch through driver**

Open `crates/scrat-core/src/deps/mod.rs`. Find the `compute_deps` function (currently around lines 74-108) and its 8-arm match table (lines 95-104). Replace the match table with a single driver call.

Before:

```rust
    let changes = match ecosystem {
        Ecosystem::Rust => RustLockfileParser.parse_diff(&diff),
        Ecosystem::Node => NodeLockfileParser.parse_diff(&diff),
        Ecosystem::Go => GoLockfileParser.parse_diff(&diff),
        Ecosystem::Php => PhpLockfileParser.parse_diff(&diff),
        Ecosystem::Python => PythonLockfileParser.parse_diff(&diff),
        Ecosystem::Ruby => RubyLockfileParser.parse_diff(&diff),
        Ecosystem::Swift => SwiftLockfileParser.parse_diff(&diff),
        Ecosystem::Generic => Vec::new(),
    };
```

After:

```rust
    let changes = ecosystem.driver().parse_lockfile_diff(&diff);
```

Also delete the imports for the old `*LockfileParser` types and the `LockfileDiffParser` trait import — they no longer exist. Keep `use crate::ecosystem::Ecosystem;` and other needed imports.

- [ ] **Step 12: Delete `LockfileDiffParser` trait + per-ecosystem parser `pub use` re-exports from `deps/mod.rs`**

Still in `crates/scrat-core/src/deps/mod.rs`:

- Delete the `LockfileDiffParser` trait definition (currently around lines 51-70).
- Delete the `mod rust;`, `mod node;`, `mod go;`, `mod php;`, `mod python;`, `mod ruby;`, `mod swift;` module declarations (currently around lines 23-49).
- Delete the `pub use rust::RustLockfileParser;` and sibling `pub use` statements (same lines).
- Delete the shared helper functions `emit_change`, `extract_toml_string_value`, `extract_json_string_value` (they now live in `ecosystem/mod.rs`).
- Delete the 4 shared-helper tests in `deps/mod.rs::tests` (they moved to `ecosystem::tests` in Step 1).

`deps/mod.rs` should now be very small: the `compute_deps` function, its module doc comment, imports, and nothing else. No trait, no per-ecosystem mods, no shared helpers, no tests (or only the 0-2 optional smoke tests you elected to add at the plan writer's discretion per the Test allocation section — skip these if you prefer to keep `deps.rs` minimal).

- [ ] **Step 13: Delete the 7 `deps/<name>.rs` sibling files**

Run:
```bash
git rm crates/scrat-core/src/deps/rust.rs crates/scrat-core/src/deps/node.rs crates/scrat-core/src/deps/go.rs crates/scrat-core/src/deps/php.rs crates/scrat-core/src/deps/python.rs crates/scrat-core/src/deps/ruby.rs crates/scrat-core/src/deps/swift.rs
```
Expected: `git rm` reports 7 files removed.

- [ ] **Step 14: Promote `deps/mod.rs` to `deps.rs` at crate root**

Run:
```bash
git mv crates/scrat-core/src/deps/mod.rs crates/scrat-core/src/deps.rs
```
Expected: `git mv` runs silently.

- [ ] **Step 15: Verify `deps/` directory is empty, then delete it**

Run:
```bash
ls crates/scrat-core/src/deps/ 2>&1 || echo "already gone"
```

If the directory still exists and is empty, remove it:
```bash
rmdir crates/scrat-core/src/deps/
```

If the directory is already gone (`ls` reported "No such file or directory"), skip the `rmdir`.

- [ ] **Step 16: Verify compilation**

Run (with `dangerouslyDisableSandbox: true`):
```bash
cargo check -p scrat-core
```
Expected: clean build, exit 0. If this fails, the atomic unit is broken — inspect errors and fix before proceeding.

- [ ] **Step 17: Verify clippy is clean**

Run (with `dangerouslyDisableSandbox: true`):
```bash
cargo clippy -p scrat-core --all-targets -- -D warnings
```
Expected: no warnings, exit 0.

- [ ] **Step 18: Run ecosystem tests with distribution check**

Run (with `dangerouslyDisableSandbox: true`):
```bash
cargo nextest run -p scrat-core ecosystem::
```
Expected: per-file distribution matches the Test allocation table:
- `ecosystem::types::tests` → 10
- `ecosystem::tests` → 4
- `ecosystem::rust::tests` → 7
- `ecosystem::node::tests` → 15
- `ecosystem::go::tests` → 9
- `ecosystem::php::tests` → 7
- `ecosystem::python::tests` → 3
- `ecosystem::ruby::tests` → 7
- `ecosystem::swift::tests` → 7
- `ecosystem::generic::tests` → 1 (the no-op contract test added in Step 10)

Total ecosystem tests at end of Task 3: 10 + 4 + 7 + 15 + 9 + 7 + 3 + 7 + 7 + 1 = **70**.

- [ ] **Step 19: Run `deps::` tests (should be 0 or the optional smoke tests)**

Run (with `dangerouslyDisableSandbox: true`):
```bash
cargo nextest run -p scrat-core deps::
```
Expected: 0 tests if you chose to leave `deps.rs` without smoke tests, or 1-2 tests if you added optional `compute_deps` dispatch smoke tests.

- [ ] **Step 20: Run full scrat-core test suite**

Run (with `dangerouslyDisableSandbox: true`):
```bash
cargo nextest run -p scrat-core
```
Expected: all 515-517 tests pass (baseline 514 + 1 required generic no-op test + 0-2 optional deps smoke tests = 515-517). No regressions.

- [ ] **Step 21: Append Task 3 section to `commit.txt`**

Use the Edit tool to APPEND to `commit.txt`:

```

* refactor(ecosystem): introduce EcosystemDriver trait and migrate deps/ into ecosystem/

Defines the EcosystemDriver trait in ecosystem/mod.rs with the
parse_lockfile_diff method absorbed from Phase 3's LockfileDiffParser
(mechanical method rename). Creates 8 driver files under ecosystem/
— one per ecosystem — each implementing the trait's single method
via verbatim transplantation of Phase 3's state machines. Python's
driver delegates to RustDriver because uv.lock currently uses the
same TOML format as Cargo.lock, with a module-level doc comment
preserving the landmine Phase 3 established.

deps/ directory collapses to deps.rs at the crate root, shedding
its per-ecosystem siblings (rust/node/go/php/python/ruby/swift) and
the LockfileDiffParser trait. compute_deps's 8-arm match table
becomes a single ecosystem.driver().parse_lockfile_diff(&diff) call.
Shared helpers (emit_change, extract_toml_string_value,
extract_json_string_value) move to ecosystem/mod.rs at pub(super).

Ecosystem::driver() factory method added to ecosystem/types.rs
returning &'static dyn EcosystemDriver for each variant.

55 per-ecosystem deps tests distribute into ecosystem/<name>.rs
tests; 4 shared-helper tests move to ecosystem/mod.rs::tests; 1
new GenericDriver no-op contract test added.
```

- [ ] **Step 22: Stop — code quality review required**

Task 3 is complete. Before Task 4 starts, the subagent-driven-development harness MUST dispatch a code-quality reviewer on the diff produced by this task. The reviewer's checklist:

1. `ecosystem/python.rs::PythonDriver::parse_lockfile_diff` is a single line: `super::rust::RustDriver.parse_lockfile_diff(diff)`. Nothing else.
2. `ecosystem/node.rs` module doc comment says "top-level only by design" explicitly.
3. `ecosystem/mod.rs` shared helpers are at `pub(super)` visibility, not `pub`.
4. No `LockfileDiffParser` trait remains anywhere in the codebase.
5. Test distribution matches Step 18's expected counts.
6. No test-name collisions within any `ecosystem::*::tests` module.
7. `deps.rs` has no per-ecosystem logic — only the `compute_deps` dispatch one-liner.

If the reviewer surfaces any issues, fix them inline before proceeding to Task 4.

---

### Task 4: Add `detect` method to `EcosystemDriver` + migrate `detect/` (ATOMIC)

**Files:**
- Modify: `crates/scrat-core/src/ecosystem/mod.rs` (add `detect` to trait)
- Modify: `crates/scrat-core/src/ecosystem/rust.rs` (add `detect` method)
- Modify: `crates/scrat-core/src/ecosystem/node.rs` (add `detect` method)
- Modify: `crates/scrat-core/src/ecosystem/go.rs` (add `detect` method)
- Modify: `crates/scrat-core/src/ecosystem/php.rs` (add `detect` method)
- Modify: `crates/scrat-core/src/ecosystem/python.rs` (add `detect` method)
- Modify: `crates/scrat-core/src/ecosystem/ruby.rs` (add `detect` method)
- Modify: `crates/scrat-core/src/ecosystem/swift.rs` (add `detect` method)
- Modify: `crates/scrat-core/src/ecosystem/generic.rs` (add `detect` method + 1 no-op contract test)
- Modify: `crates/scrat-core/src/detect/mod.rs` (rewrite `build_detection_for` via driver)
- Delete: `crates/scrat-core/src/detect/{rust,node,go,php,python,ruby,swift}.rs` (7 files)
- Move: `crates/scrat-core/src/detect/mod.rs` → `crates/scrat-core/src/detect.rs`
- Delete: `crates/scrat-core/src/detect/` directory

**This task is atomic.** All driver `detect` impls must be added simultaneously with the trait method addition.

**Review:** Spec review inline (mechanical replication of the Task 3 template). No dispatched code-quality review.

- [ ] **Step 1: Add `detect` method to the trait in `ecosystem/mod.rs`**

Find the `pub trait EcosystemDriver { ... }` block in `crates/scrat-core/src/ecosystem/mod.rs`. Add a new method AFTER `parse_lockfile_diff` (inside the trait block):

```rust
    /// Build a [`ProjectDetection`] by probing `PATH` and assembling
    /// the smart-default tool commands for this ecosystem.
    fn detect(
        &self,
        project_root: &Utf8Path,
        version_strategy: VersionStrategy,
    ) -> ProjectDetection;
```

Add the needed imports at the top of `ecosystem/mod.rs`:

```rust
use camino::Utf8Path;
// ProjectDetection and VersionStrategy are already re-exported via `pub use types::*;`
// but the trait definition in mod.rs still needs them in scope:
use types::{ProjectDetection, VersionStrategy};
```

- [ ] **Step 2: Add `detect` impl to `ecosystem/rust.rs`**

Read the current `crates/scrat-core/src/detect/rust.rs`. In `ecosystem/rust.rs`, add `detect` as an additional method inside the existing `impl EcosystemDriver for RustDriver { ... }` block. The method body is copied verbatim from `detect::rust::detect_rust` — the existing function signature `(_project_root: &Utf8Path, version_strategy: VersionStrategy) -> ProjectDetection` is already trait-compatible.

After the change, the Rust driver impl block should contain both `parse_lockfile_diff` (from Task 3) and `detect` (added now):

```rust
impl EcosystemDriver for RustDriver {
    fn detect(
        &self,
        _project_root: &Utf8Path,
        version_strategy: VersionStrategy,
    ) -> ProjectDetection {
        // body copied verbatim from detect/rust.rs::detect_rust
        let has_cargo = has_binary("cargo");
        // ... (full body)
    }

    fn parse_lockfile_diff(&self, diff: &str) -> Vec<DepChange> {
        // unchanged from Task 3
        // ...
    }
}
```

Add needed imports to `ecosystem/rust.rs`:

```rust
use camino::Utf8Path;
use tracing::debug;

use crate::detect::has_binary;
use crate::ecosystem::{DetectedTools, Ecosystem, ProjectDetection, VersionStrategy};
```

Also append the 3 tests from `detect/rust.rs::tests` into `ecosystem/rust.rs::tests`, replacing `detect_rust(...)` calls with `RustDriver.detect(...)` calls. The 3 test names: `rust_detection_basic`, `rust_changelog_tool_wired_from_strategy`, `rust_interactive_strategy_has_no_changelog_tool`.

- [ ] **Step 3: Add `detect` impl to `ecosystem/node.rs`**

Read `crates/scrat-core/src/detect/node.rs`. In `ecosystem/node.rs`, add `detect` as an additional method in the existing `impl EcosystemDriver for NodeDriver { ... }` block, copying the body verbatim from `detect::node::detect_node`. Add imports as needed.

- [ ] **Step 4: Add `detect` impls to `ecosystem/{go,php,python,ruby,swift}.rs`**

For each of the remaining 5 ecosystems, read the current `detect/<name>.rs` file and add the `detect` method to the corresponding driver's `impl EcosystemDriver` block. All five files follow the same pattern: method body copied verbatim, imports added at module level.

- [ ] **Step 5: Add `detect` impl to `ecosystem/generic.rs`**

In `crates/scrat-core/src/ecosystem/generic.rs`, add the `detect` method to the existing `impl EcosystemDriver for GenericDriver` block. Body returns `ProjectDetection::generic(version_strategy)`:

```rust
    fn detect(
        &self,
        _project_root: &Utf8Path,
        version_strategy: VersionStrategy,
    ) -> ProjectDetection {
        ProjectDetection::generic(version_strategy)
    }
```

Add imports to `ecosystem/generic.rs`:

```rust
use camino::Utf8Path;

use crate::ecosystem::{ProjectDetection, VersionStrategy};
```

Also append a new no-op contract test to `ecosystem::generic::tests`:

```rust
    #[test]
    fn generic_detect_returns_generic_project_detection() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = camino::Utf8Path::from_path(tmp.path()).unwrap();
        let detection = GenericDriver.detect(root, VersionStrategy::Interactive);
        assert_eq!(detection.ecosystem, Ecosystem::Generic);
        assert_eq!(detection.version_strategy, VersionStrategy::Interactive);
        assert_eq!(detection.tools.test_cmd, "");
        assert_eq!(detection.tools.build_cmd, "");
        assert!(detection.tools.publish_cmd.is_none());
    }
```

(Remember to add `use crate::ecosystem::Ecosystem;` if not already imported.)

- [ ] **Step 6: Rewrite `detect/mod.rs::build_detection_for` to dispatch via driver**

Open `crates/scrat-core/src/detect/mod.rs`. Find `build_detection_for` (currently around lines 129-144):

Before:

```rust
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

After:

```rust
fn build_detection_for(
    project_root: &Utf8Path,
    ecosystem: Ecosystem,
    version_strategy: VersionStrategy,
) -> ProjectDetection {
    ecosystem.driver().detect(project_root, version_strategy)
}
```

Also delete the now-unused `mod rust; mod node; ...` declarations at the top of `detect/mod.rs` (currently around lines 20-26).

- [ ] **Step 7: Delete the 7 `detect/<name>.rs` sibling files**

Run:
```bash
git rm crates/scrat-core/src/detect/rust.rs crates/scrat-core/src/detect/node.rs crates/scrat-core/src/detect/go.rs crates/scrat-core/src/detect/php.rs crates/scrat-core/src/detect/python.rs crates/scrat-core/src/detect/ruby.rs crates/scrat-core/src/detect/swift.rs
```
Expected: 7 files removed.

- [ ] **Step 8: Promote `detect/mod.rs` to `detect.rs` at crate root**

Run:
```bash
git mv crates/scrat-core/src/detect/mod.rs crates/scrat-core/src/detect.rs
```

Then remove the now-empty directory:
```bash
rmdir crates/scrat-core/src/detect/ 2>/dev/null || true
```

- [ ] **Step 9: Verify compilation, clippy, tests**

Run in sequence (all with `dangerouslyDisableSandbox: true`):

```bash
cargo check -p scrat-core
cargo clippy -p scrat-core --all-targets -- -D warnings
cargo nextest run -p scrat-core ecosystem::
cargo nextest run -p scrat-core detect::
cargo nextest run -p scrat-core
```

Expected after Task 4:
- `ecosystem::rust::tests`: 10 (was 7 at end of Task 3 + 3 detect tests)
- `ecosystem::generic::tests`: 2 (was 1 at end of Task 3 + 1 new detect no-op test)
- Other ecosystem drivers: unchanged from Task 3 counts
- `detect::tests`: 12 (unchanged — dispatch + tool helpers stay in `detect.rs`)
- Total scrat-core: baseline + 1 (Task 3's generic no-op) + 1 (Task 4's generic no-op) + 0-2 (optional Task 3 smoke tests) = 516-519

- [ ] **Step 10: Append Task 4 section to `commit.txt`**

Use the Edit tool to APPEND:

```

* refactor(ecosystem): absorb detect/ into EcosystemDriver

Grows the EcosystemDriver trait with a detect method matching the
existing detect_<name>(project_root, version_strategy) -> ProjectDetection
signature. Each driver's impl block gains a detect method body copied
verbatim from the corresponding detect/<name>.rs helper. Generic's
detect returns ProjectDetection::generic(version_strategy).

detect/ directory collapses to detect.rs at crate root.
build_detection_for's 8-arm match becomes
ecosystem.driver().detect(...) one-liner. The 7 detect sibling
files are deleted; detect::rust's 3 tests move to
ecosystem::rust::tests. One new GenericDriver detect no-op contract
test added.
```

- [ ] **Step 11: Stop — continue to Task 5**

Task 4 uses spec review only (inline controller verification). No dispatched code-quality review required.

---

### Task 5: Add `bump_version_files` method to `EcosystemDriver` + migrate `bump/` (ATOMIC — SECOND-BIGGEST TASK — code quality review required)

**Files:**
- Modify: `crates/scrat-core/src/ecosystem/mod.rs` (add `bump_version_files` to trait)
- Modify: `crates/scrat-core/src/ecosystem/{rust,node,php,python,ruby}.rs` (add `bump_version_files` method body from current `bump/<name>.rs`)
- Modify: `crates/scrat-core/src/ecosystem/{go,swift,generic}.rs` (add no-op `bump_version_files`)
- Modify: `crates/scrat-core/src/bump/mod.rs` (rewrite `ReadyBump::execute` dispatch, preserve Ruby post-dispatch check)
- Delete: `crates/scrat-core/src/bump/{rust,node,php,python,ruby}.rs` (5 files)
- Move: `crates/scrat-core/src/bump/mod.rs` → `crates/scrat-core/src/bump.rs`
- Delete: `crates/scrat-core/src/bump/` directory

**This task is atomic.** All 8 drivers gain the method simultaneously with the trait addition and the dispatch rewrite.

**Review:** Requires code-quality review (dispatched subagent). Second-biggest task, and the one with semantic asymmetries (Rust `&detection`, Ruby caller-side check).

**Anti-refactors for this task (worker prompt must repeat):**
- The `detection: &ProjectDetection` parameter on `bump_version_files` is load-bearing for Rust. Keep it on the trait method signature. Other drivers ignore it but still accept it.
- **Ruby's "no files modified" check stays in `bump.rs::ReadyBump::execute`, post-dispatch.** Do NOT push this check into `RubyDriver::bump_version_files`. The check depends on `ReadyBump::version_files` which the driver cannot see.
- `RustDriver::bump_version_files` returns `Err(BumpError::NoBumpTool)` when `detection.tools.bump_cmd.is_none()`. Preserve this Rust-exclusive failure mode.

- [ ] **Step 1: Add `bump_version_files` method to the trait in `ecosystem/mod.rs`**

In `crates/scrat-core/src/ecosystem/mod.rs`, add to the `pub trait EcosystemDriver { ... }` block (after `detect`, before `parse_lockfile_diff`):

```rust
    /// Rewrite on-disk version files for this ecosystem.
    ///
    /// Returns the repo-relative paths of files that were actually
    /// modified. Returns an empty `Vec` for ecosystems where the version
    /// lives in git tags (Go, Swift) or there is no project file to
    /// rewrite (Generic).
    ///
    /// The `&ProjectDetection` argument is load-bearing for Rust, which
    /// reads `detection.tools.bump_cmd` to find `cargo set-version`.
    /// Other drivers currently ignore it, but the parameter is passed
    /// uniformly so future drivers can opt in without signature churn.
    fn bump_version_files(
        &self,
        project_root: &Utf8Path,
        version: &Version,
        detection: &ProjectDetection,
    ) -> BumpResult<Vec<String>>;
```

Add needed imports at the top of `ecosystem/mod.rs`:

```rust
use semver::Version;

use crate::bump::BumpResult;
```

- [ ] **Step 2: Add `bump_version_files` impl to `ecosystem/rust.rs`**

Read the current `crates/scrat-core/src/bump/rust.rs`. In `ecosystem/rust.rs`, add `bump_version_files` as an additional method to the existing `impl EcosystemDriver for RustDriver` block. Body copied verbatim from `bump::rust::bump_rust_version`:

```rust
    fn bump_version_files(
        &self,
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

Add imports:

```rust
use std::process::Command;

use semver::Version;

use crate::bump::{BumpError, BumpResult};
```

- [ ] **Step 3: Add `bump_version_files` impls to `ecosystem/{node,php,python}.rs`**

For each of node, php, python:
1. Read the current `bump/<name>.rs`.
2. In `ecosystem/<name>.rs`, add `bump_version_files` to the existing `impl EcosystemDriver` block with the body copied verbatim from `bump::<name>::bump_<name>_version`.
3. Even though these drivers don't read `detection`, keep the parameter in the signature (trait requirement).
4. Add imports.

- [ ] **Step 4: Add `bump_version_files` impl to `ecosystem/ruby.rs`**

Read `crates/scrat-core/src/bump/ruby.rs` (~548 LOC, the biggest single driver body). In `ecosystem/ruby.rs`:

1. Add `bump_version_files` to the existing `impl EcosystemDriver` block with the body copied verbatim from `bump::ruby::bump_ruby_version`.
2. Copy ALL private helpers (`update_ruby_version_file`, `update_gemspec_version_file`, `replace_ruby_version_line`, `replace_gemspec_version_line`) into `ecosystem/ruby.rs` as private `fn` items (they are Ruby-exclusive).
3. Copy ALL 19 private tests from `bump::ruby::tests` into `ecosystem::ruby::tests`, appending to the 7 deps tests already there from Task 3. Total: 26 tests in `ecosystem::ruby::tests`.
4. Check for test name collisions between the existing 7 deps tests (`parse_gemfile_lock_diff_*`) and the 19 bump tests (various names like `ruby_version_*`, `gemspec_*`, `bump_ruby_*`). Ruby's bump tests use `ruby_version_*`, `gemspec_*`, `bump_ruby_*` prefixes while deps tests use `parse_gemfile_lock_diff_*` — no expected collisions, but verify empirically before completing the task.
5. Add imports.

- [ ] **Step 5: Add no-op `bump_version_files` to `ecosystem/{go,swift,generic}.rs`**

For Go and Swift, add to the existing `impl EcosystemDriver` block:

```rust
    fn bump_version_files(
        &self,
        _project_root: &Utf8Path,
        _version: &Version,
        _detection: &ProjectDetection,
    ) -> BumpResult<Vec<String>> {
        tracing::debug!("version lives in git tags, no file to bump");
        Ok(Vec::new())
    }
```

For Generic, add:

```rust
    fn bump_version_files(
        &self,
        _project_root: &Utf8Path,
        _version: &Version,
        _detection: &ProjectDetection,
    ) -> BumpResult<Vec<String>> {
        tracing::debug!("generic ecosystem — no project files to bump");
        Ok(Vec::new())
    }
```

Add the needed imports (`use semver::Version;`, `use crate::bump::BumpResult;`) to go.rs, swift.rs, generic.rs.

Append a no-op contract test to `ecosystem::generic::tests`:

```rust
    #[test]
    fn generic_bump_version_files_returns_empty() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = camino::Utf8Path::from_path(tmp.path()).unwrap();
        let detection = ProjectDetection::generic(VersionStrategy::Interactive);
        let files = GenericDriver
            .bump_version_files(root, &semver::Version::new(1, 0, 0), &detection)
            .unwrap();
        assert!(files.is_empty());
    }
```

- [ ] **Step 6: Rewrite `ReadyBump::execute` dispatch in `bump/mod.rs` (preserve Ruby post-dispatch check)**

Open `crates/scrat-core/src/bump/mod.rs`. Find `ReadyBump::execute` method (currently around lines 267-360). Locate the 8-arm `match self.detection.ecosystem { ... }` block that dispatches to per-ecosystem helpers.

Replace the entire match block with driver dispatch PLUS the Ruby post-dispatch check:

Before:

```rust
        // Update version in project files (Generic has no project files to update)
        match self.detection.ecosystem {
            Ecosystem::Rust => {
                let files = rust::bump_rust_version(project_root, &self.next, &self.detection)?;
                modified_files.extend(files);
            }
            Ecosystem::Node => {
                let files = node::bump_node_version(project_root, &self.next)?;
                modified_files.extend(files);
            }
            Ecosystem::Go | Ecosystem::Swift => {
                debug!(%self.detection.ecosystem, "version lives in git tags, no file to bump");
            }
            Ecosystem::Php => {
                let files = php::bump_composer_version(project_root, &self.next)?;
                if files.is_empty() {
                    debug!("composer.json has no version field, skipping");
                }
                modified_files.extend(files);
            }
            Ecosystem::Python => {
                let files = python::bump_pyproject_version(project_root, &self.next)?;
                if files.is_empty() {
                    debug!("pyproject.toml has no version field, skipping");
                }
                modified_files.extend(files);
            }
            Ecosystem::Ruby => {
                let files = ruby::bump_ruby_version(project_root, &self.next)?;
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

After:

```rust
        // Dispatch ecosystem-specific version file rewrite through the driver.
        // Every driver accepts (project_root, version, detection); only Rust
        // reads detection (for bump_cmd). Go, Swift, Generic return empty Vec.
        let ecosystem_files = self.detection.ecosystem.driver()
            .bump_version_files(project_root, &self.next, &self.detection)?;

        // Ruby-specific post-dispatch release-correctness check: if the driver
        // found nothing AND there are no [[version_files]] configured, the
        // release would tag without updating any file. Block it. This is a
        // caller-layer rule, not a driver concern — the driver itself does
        // not have visibility into [[version_files]] config.
        if self.detection.ecosystem == Ecosystem::Ruby
            && ecosystem_files.is_empty()
            && self.version_files.is_empty()
        {
            return Err(BumpError::ToolFailed {
                tool: "ruby".into(),
                message: "no lib/**/version.rb or gemspec with a literal version \
                          was found, and no `[[version_files]]` entries are \
                          configured — the release would be tagged without \
                          updating any file"
                    .into(),
            });
        }

        modified_files.extend(ecosystem_files);
```

Delete the now-unused `mod node; mod php; mod python; mod ruby; mod rust;` declarations at the top of `bump/mod.rs` (currently around lines 27-31).

- [ ] **Step 7: Delete the 5 `bump/<name>.rs` sibling files**

Run:
```bash
git rm crates/scrat-core/src/bump/rust.rs crates/scrat-core/src/bump/node.rs crates/scrat-core/src/bump/php.rs crates/scrat-core/src/bump/python.rs crates/scrat-core/src/bump/ruby.rs
```
Expected: 5 files removed.

- [ ] **Step 8: Promote `bump/mod.rs` to `bump.rs` at crate root**

Run:
```bash
git mv crates/scrat-core/src/bump/mod.rs crates/scrat-core/src/bump.rs
rmdir crates/scrat-core/src/bump/ 2>/dev/null || true
```

- [ ] **Step 9: Verify compilation, clippy, tests**

Run in sequence (all with `dangerouslyDisableSandbox: true`):

```bash
cargo check -p scrat-core
cargo clippy -p scrat-core --all-targets -- -D warnings
cargo nextest run -p scrat-core ecosystem::
cargo nextest run -p scrat-core bump::
cargo nextest run -p scrat-core
```

Expected after Task 5:
- `ecosystem::ruby::tests`: 26 (7 deps + 19 bump)
- `ecosystem::generic::tests`: 3 (was 2 at end of Task 4 + 1 new bump no-op test)
- `bump::tests` (in `bump.rs`): 25 (unchanged — the public bump tests)
- Total scrat-core: baseline + 1 (Task 3 generic no-op) + 1 (Task 4 generic no-op) + 1 (Task 5 generic no-op) + 0-2 (optional Task 3 smoke tests) = 517-520

- [ ] **Step 10: Run test-name collision check on Ruby module**

Run (with `dangerouslyDisableSandbox: true`):
```bash
cargo nextest list -p scrat-core ecosystem::ruby:: 2>&1 | sort | uniq -d
```
Expected: no output (empty). If any duplicate test names appear, rename the bump-side test in `ecosystem/ruby.rs::tests` to disambiguate (prefix with `bump_` if not already).

- [ ] **Step 11: Append Task 5 section to `commit.txt`**

Use the Edit tool to APPEND:

```

* refactor(ecosystem): absorb bump/ into EcosystemDriver

Grows the EcosystemDriver trait with a bump_version_files method
taking (project_root, version, &ProjectDetection). The &detection
parameter is load-bearing for Rust, which reads detection.tools.bump_cmd
to find cargo set-version; other drivers accept it uniformly but
currently ignore it. Go, Swift, and Generic drivers return an empty
Vec with a debug log.

Ruby's caller-side "no files modified" release-correctness check
stays in bump.rs::ReadyBump::execute post-dispatch. The driver itself
cannot see ReadyBump::version_files config, so the check must remain
at the caller layer. RubyDriver's bump_version_files returns an
empty Vec as a valid state; the caller enforces the rule.

bump/ directory collapses to bump.rs at crate root. The 5 per-ecosystem
sibling files (rust/node/php/python/ruby) are deleted. 19 private
Ruby bump tests move to ecosystem::ruby::tests (total 26 with deps).
One new GenericDriver bump no-op contract test added.
```

- [ ] **Step 12: Stop — code quality review required**

Task 5 is complete. Dispatch code-quality review. Reviewer checklist:

1. `ecosystem/rust.rs::RustDriver::bump_version_files` reads `detection.tools.bump_cmd` and returns `Err(BumpError::NoBumpTool)` when absent.
2. `bump.rs::ReadyBump::execute` contains the Ruby post-dispatch check; `ecosystem/ruby.rs::RubyDriver::bump_version_files` does NOT contain any fallback error logic for empty file lists.
3. The trait method signature includes `detection: &ProjectDetection` — every driver accepts it, even drivers that ignore it.
4. No `bump::<name>::bump_<name>_version` functions remain in the codebase.
5. `bump.rs::tests` still has 25 public tests (unchanged).
6. `ecosystem::ruby::tests` has 26 tests with no name collisions.

Fix any issues inline before proceeding to Task 6.

---

### Task 6: Add `check_registry_auth` method to `EcosystemDriver` + migrate from `preflight.rs` (ATOMIC)

**Files:**
- Modify: `crates/scrat-core/src/ecosystem/mod.rs` (add `check_registry_auth` to trait)
- Modify: `crates/scrat-core/src/ecosystem/{rust,node,python,ruby}.rs` (add real `check_registry_auth` impls)
- Modify: `crates/scrat-core/src/ecosystem/{go,php,swift,generic}.rs` (add no-op `check_registry_auth` impls)
- Modify: `crates/scrat-core/src/preflight.rs` (delete `check_registry_auth` function, update `run_preflight` to dispatch via driver)

**This task is atomic.** Trait addition, all 8 driver impls, and preflight dispatch update happen together.

**Review:** Spec review inline (mechanical replication).

- [ ] **Step 1: Add `check_registry_auth` method to the trait**

In `crates/scrat-core/src/ecosystem/mod.rs`, add to the trait (after `bump_version_files`):

```rust
    /// Check registry auth for the publish phase.
    ///
    /// Uses fast env-var checks (no network). Returns a pre-populated
    /// "no registry for this ecosystem" passing `CheckResult` for Go, PHP,
    /// Swift, and Generic.
    fn check_registry_auth(&self) -> CheckResult;
```

Add import:

```rust
use crate::preflight::CheckResult;
```

- [ ] **Step 2: Add real `check_registry_auth` impls to `ecosystem/{rust,node,python,ruby}.rs`**

Read the current `crates/scrat-core/src/preflight.rs::check_registry_auth` function (around lines 407-458). Each match arm becomes one driver method body.

For `ecosystem/rust.rs`, add:

```rust
    fn check_registry_auth(&self) -> CheckResult {
        let env_vars = vec!["CARGO_REGISTRY_TOKEN"];
        let registry_name = "crates.io";
        let login_hint = "set CARGO_REGISTRY_TOKEN or run `cargo login`";
        check_registry_auth_impl(&env_vars, registry_name, login_hint)
    }
```

For `ecosystem/node.rs`:

```rust
    fn check_registry_auth(&self) -> CheckResult {
        let env_vars = vec!["NPM_TOKEN", "NODE_AUTH_TOKEN"];
        let registry_name = "npm";
        let login_hint = "set NPM_TOKEN or run `npm login`";
        super::check_registry_auth_impl(&env_vars, registry_name, login_hint)
    }
```

For `ecosystem/python.rs`:

```rust
    fn check_registry_auth(&self) -> CheckResult {
        let env_vars = vec!["TWINE_PASSWORD", "PYPI_TOKEN"];
        let registry_name = "PyPI";
        let login_hint = "set TWINE_PASSWORD or PYPI_TOKEN";
        super::check_registry_auth_impl(&env_vars, registry_name, login_hint)
    }
```

For `ecosystem/ruby.rs`:

```rust
    fn check_registry_auth(&self) -> CheckResult {
        let env_vars = vec!["GEM_HOST_API_KEY"];
        let registry_name = "RubyGems";
        let login_hint = "set GEM_HOST_API_KEY or run `gem signin`";
        super::check_registry_auth_impl(&env_vars, registry_name, login_hint)
    }
```

Add the shared helper `check_registry_auth_impl` as `pub(super)` in `ecosystem/mod.rs`:

```rust
/// Shared registry-auth env-var check body.
///
/// Used by the real `check_registry_auth` impls in rust/node/python/ruby
/// drivers. Returns a passing `CheckResult` when any env var is set,
/// failing (with `--no-publish` skip flag) otherwise.
pub(super) fn check_registry_auth_impl(
    env_vars: &[&str],
    registry_name: &str,
    login_hint: &str,
) -> crate::preflight::CheckResult {
    let found = env_vars.iter().any(|v| std::env::var(v).is_ok());

    if found {
        crate::preflight::CheckResult {
            name: "Registry auth".into(),
            passed: true,
            message: format!("{registry_name} credentials found"),
            skip_flag: None,
        }
    } else {
        let vars = env_vars.join(" or ");
        crate::preflight::CheckResult {
            name: "Registry auth".into(),
            passed: false,
            message: format!("{vars} not set — {login_hint}"),
            skip_flag: Some("--no-publish".into()),
        }
    }
}
```

Add `use crate::preflight::CheckResult;` to each of the four ecosystem driver files.

- [ ] **Step 3: Add no-op `check_registry_auth` impls to `ecosystem/{go,php,swift,generic}.rs`**

For each of go.rs, php.rs, swift.rs, generic.rs, add to the existing `impl EcosystemDriver` block:

```rust
    fn check_registry_auth(&self) -> CheckResult {
        CheckResult {
            name: "Registry auth".into(),
            passed: true,
            message: "No registry publish for this ecosystem".into(),
            skip_flag: None,
        }
    }
```

Add `use crate::preflight::CheckResult;` to each.

Append a no-op contract test to `ecosystem::generic::tests`:

```rust
    #[test]
    fn generic_check_registry_auth_returns_no_registry() {
        let result = GenericDriver.check_registry_auth();
        assert!(result.passed);
        assert_eq!(result.message, "No registry publish for this ecosystem");
        assert!(result.skip_flag.is_none());
    }
```

- [ ] **Step 4: Update `preflight.rs::run_preflight` to dispatch via driver and delete the old function**

Open `crates/scrat-core/src/preflight.rs`. Find `run_preflight` (around line 101-130). The line that currently calls `check_registry_auth(det.ecosystem)`:

Before:

```rust
    // Check 8: Registry auth (needed for publish phase, ecosystem-specific)
    if /* ShipOptions say we're publishing */ {
        if let Some(ref det) = detection {
            checks.push(check_registry_auth(det.ecosystem));
        }
    }
```

After:

```rust
    // Check 8: Registry auth (needed for publish phase, ecosystem-specific)
    if /* ShipOptions say we're publishing */ {
        if let Some(ref det) = detection {
            checks.push(det.ecosystem.driver().check_registry_auth());
        }
    }
```

Then delete the `check_registry_auth` function body at lines 402-458.

- [ ] **Step 5: Migrate 5 `check_registry_auth_*` tests from `preflight.rs::tests` to per-ecosystem drivers**

Read `crates/scrat-core/src/preflight.rs::tests` and find the 5 registry_auth tests:
- `check_registry_auth_rust`
- `check_registry_auth_node`
- `check_registry_auth_python`
- `check_registry_auth_go_skips`
- `check_registry_auth_generic_skips`

Move each test into the corresponding `ecosystem::<name>::tests` module, adapting the call from the deleted `check_registry_auth(Ecosystem::Rust)` to `RustDriver.check_registry_auth()` (or the appropriate driver). The test assertions carry over unchanged.

Delete the 5 test functions from `preflight.rs::tests`.

- [ ] **Step 6: (Optional) Add preflight integration smoke test**

At your discretion, add a single integration test in `preflight.rs::tests` that asserts `run_preflight` dispatches through the driver correctly — e.g., verifying that `check_registry_auth` appears in the `CheckReport` with the expected name and passing state for a Rust detection. This is optional; skip if it feels redundant with the per-driver tests.

- [ ] **Step 7: Verify compilation, clippy, tests**

Run in sequence (all with `dangerouslyDisableSandbox: true`):

```bash
cargo check -p scrat-core
cargo clippy -p scrat-core --all-targets -- -D warnings
cargo nextest run -p scrat-core ecosystem::
cargo nextest run -p scrat-core preflight::
cargo nextest run -p scrat-core
```

Expected after Task 6:
- `ecosystem::rust::tests`: 11 (10 from Task 4 + 1 registry_auth from Task 6)
- `ecosystem::node::tests`: 16 (15 from Task 3 + 1 registry_auth)
- `ecosystem::go::tests`: 10 (9 from Task 3 + 1 registry_auth)
- `ecosystem::python::tests`: 4 (3 from Task 3 + 1 registry_auth)
- `ecosystem::generic::tests`: 5 (3 from Task 5 + 1 registry_auth migrated + 1 registry_auth no-op contract)
- `ecosystem::php::tests`: 7 (unchanged — no registry_auth arm)
- `ecosystem::ruby::tests`: 26 (unchanged — no registry_auth test existed)
- `ecosystem::swift::tests`: 7 (unchanged)
- `preflight::tests`: 53 (58 − 5 migrated) + 0 or 1 optional integration test
- Total scrat-core: baseline + 1 (Task 3 no-op) + 1 (Task 4 no-op) + 1 (Task 5 no-op) + 1 (Task 6 no-op) + 0-2 (optional Task 3 smoke) + 0-1 (optional Task 6 integration) = 518-521

- [ ] **Step 8: Append Task 6 section to `commit.txt`**

Use the Edit tool to APPEND:

```

* refactor(ecosystem): absorb check_registry_auth into EcosystemDriver

Grows the EcosystemDriver trait with a check_registry_auth method.
The 4-arm match table in preflight::check_registry_auth becomes per-
driver impls: Rust, Node, Python, and Ruby carry their own env-var
lists, registry names, and login hints. Go, PHP, Swift, and Generic
return a passing "no registry for this ecosystem" CheckResult via
a shared no-op pattern.

preflight::check_registry_auth function deleted. run_preflight
calls det.ecosystem.driver().check_registry_auth() directly. 5
registry_auth tests migrate from preflight::tests to per-ecosystem
files. One new GenericDriver check_registry_auth no-op contract
test added.
```

- [ ] **Step 9: Stop — continue to Task 7**

Task 6 uses spec review only. No dispatched code-quality review.

---

### Task 7: Final polish, verification, and PR prep (code quality review required)

**Files:**
- Modify: `crates/scrat-core/src/lib.rs` (rustdoc update mentioning `ecosystem::EcosystemDriver` and `Ecosystem::driver()`)
- Verify: `commit.txt` (complete message ready for `gtxt`)

**Review:** Requires code-quality review (dispatched subagent) — final PR review.

- [ ] **Step 1: Update `lib.rs` rustdoc module listing**

Open `crates/scrat-core/src/lib.rs`. Find the module listing in the crate-level rustdoc (currently around lines 1-35). Update the `- [`ecosystem`] - Ecosystem types and smart defaults` line to reflect the new scope:

```rust
//! - [`ecosystem`] - Ecosystem types, drivers, and the EcosystemDriver trait
```

If the rustdoc mentions `bump`, `deps`, or `detect` as separate modules, make sure it still reflects their post-Phase-4 shape (now top-level files rather than directories — the `pub mod` declarations don't change because Rust resolves either).

No structural changes to `lib.rs::pub mod` declarations — those continue to resolve to either `deps.rs` or `deps/mod.rs` automatically. Same for `bump`, `detect`, `ecosystem`.

- [ ] **Step 2: Verify final file structure matches the plan**

Run:
```bash
ls crates/scrat-core/src/
```

Expected output (alphabetical, with `ecosystem/` as a directory):
```
bump.rs
config.rs
deps.rs
detect.rs
ecosystem
error.rs
git.rs
hooks.rs
init.rs
lib.rs
notes.rs
observability.rs
pipeline.rs
preflight.rs
ship.rs
stats.rs
version
version_files.rs
```

Run:
```bash
ls crates/scrat-core/src/ecosystem/
```

Expected:
```
generic.rs
go.rs
mod.rs
node.rs
php.rs
python.rs
ruby.rs
rust.rs
swift.rs
types.rs
```

No `bump/`, `deps/`, `detect/` directories. No leftover sibling files.

- [ ] **Step 3: Run final compilation and clippy gates**

Run in sequence (with `dangerouslyDisableSandbox: true`):

```bash
cargo check -p scrat-core
cargo clippy -p scrat-core --all-targets -- -D warnings
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

All four commands must pass with exit 0. The workspace-level checks catch any downstream crates (scrat CLI, xtask) that may have stale imports.

- [ ] **Step 4: Run the full workspace test suite**

Run (with `dangerouslyDisableSandbox: true`):
```bash
cargo nextest run
```

Expected: all tests pass workspace-wide. Exact total depends on the baseline at Task 1 plus the N value (the number of new tests added in Tasks 3-6). For reference: baseline 514 for scrat-core, plus workspace-level tests from scrat CLI and xtask.

- [ ] **Step 5: Verify per-ecosystem test distribution**

Run (with `dangerouslyDisableSandbox: true`):
```bash
for e in rust node go php python ruby swift generic; do
  count=$(cargo nextest list -p scrat-core ecosystem::${e}:: 2>&1 | grep -c "^scrat-core " || echo 0)
  echo "ecosystem::${e}::tests: ${count}"
done
cargo nextest list -p scrat-core ecosystem::types:: 2>&1 | grep -c "^scrat-core " && echo "ecosystem::types::tests"
cargo nextest list -p scrat-core ecosystem::tests:: 2>&1 | grep -c "^scrat-core " && echo "ecosystem::tests (shared helpers)"
```

Expected counts:
- `ecosystem::rust::tests`: 11
- `ecosystem::node::tests`: 16
- `ecosystem::go::tests`: 10
- `ecosystem::php::tests`: 7
- `ecosystem::python::tests`: 4
- `ecosystem::ruby::tests`: 26
- `ecosystem::swift::tests`: 7
- `ecosystem::generic::tests`: 5
- `ecosystem::types::tests`: 10
- `ecosystem::tests` (shared helpers): 4

Total ecosystem: 100.

- [ ] **Step 6: Verify total test count invariant**

Run (with `dangerouslyDisableSandbox: true`):
```bash
cargo nextest list -p scrat-core 2>&1 | grep -c "^scrat-core "
```

Expected: baseline + N where N is the actual count of new tests added. For the minimum (only the 4 required GenericDriver no-op contract tests from Tasks 3-6), post = baseline + 4 = **518**. For the maximum (all optional tests added), post = baseline + 7 = **521**.

If the count falls outside [baseline+4, baseline+7], STOP and investigate which tests are missing or extra. This is a hard gate.

- [ ] **Step 7: Verify that `LockfileDiffParser` and `check_registry_auth` references are fully gone**

Run:
```bash
rg 'LockfileDiffParser' crates/
rg 'fn check_registry_auth' crates/scrat-core/src/preflight.rs
rg 'check_registry_auth\(' crates/scrat-core/
```

Expected:
- `LockfileDiffParser`: no hits anywhere in `crates/`.
- `fn check_registry_auth` in preflight.rs: no hits (the function is deleted).
- `check_registry_auth(` (function call): only hits are the driver method calls `.check_registry_auth()` via `ecosystem.driver()`.

- [ ] **Step 8: Verify no per-ecosystem bump/deps/detect functions remain**

Run:
```bash
rg 'fn bump_rust_version|fn bump_node_version|fn bump_composer_version|fn bump_pyproject_version|fn bump_ruby_version' crates/
rg 'fn parse_cargo_lock_diff|fn parse_package_lock_diff|fn parse_go_mod_diff|fn parse_composer_lock_diff|fn parse_uv_lock_diff|fn parse_gemfile_lock_diff|fn parse_package_resolved_diff' crates/
rg 'fn detect_rust|fn detect_node|fn detect_go|fn detect_php|fn detect_python|fn detect_ruby|fn detect_swift' crates/
```

Expected: no hits. All per-ecosystem helper functions have been migrated into driver method bodies.

- [ ] **Step 9: Append Task 7 section to `commit.txt`**

Use the Edit tool to APPEND:

```

* docs(scrat-core): refresh lib.rs rustdoc for ecosystem module

Updates the module listing in the crate-level rustdoc to describe
ecosystem as the home of the EcosystemDriver trait and per-ecosystem
drivers in addition to types and smart defaults. No structural
lib.rs changes — pub mod declarations for bump, deps, and detect
continue to resolve to the new top-level files automatically.
```

- [ ] **Step 10: Verify `commit.txt` is complete and well-formed**

Run:
```bash
cat commit.txt
```

Expected: a complete bundled commit message with:
- One top-level subject line (`refactor(ecosystem): unify bump, deps, detect...`)
- Task 1's real intro paragraph (not a placeholder)
- Six `* refactor/docs(...)` sub-bullet sections for Tasks 2, 3, 4, 5, 6, 7
- No `TBD`, `TODO`, `[...]`, or other placeholder text

If any placeholder text is present, it's a plan failure — fix before proceeding.

- [ ] **Step 11: Stop — code quality review required, then Clay runs gtxt and git pm**

Task 7 is complete. Dispatch a final code-quality review on the full accumulated diff (all 7 tasks). Reviewer checklist:

1. No `LockfileDiffParser` trait or `check_registry_auth` free function remain.
2. `bump.rs`, `deps.rs`, `detect.rs` exist at crate root; `bump/`, `deps/`, `detect/` directories do NOT.
3. `ecosystem/` directory exists with 10 files: `mod.rs`, `types.rs`, 7 per-ecosystem driver files, `generic.rs`.
4. `EcosystemDriver` trait has exactly 4 methods: `detect`, `bump_version_files`, `parse_lockfile_diff`, `check_registry_auth`.
5. `PythonDriver::parse_lockfile_diff` is a one-line delegation to `RustDriver.parse_lockfile_diff(diff)`. Module-level doc comment explains why.
6. `ReadyBump::execute` in `bump.rs` contains the Ruby post-dispatch "no files modified" check.
7. Test count invariant holds (baseline + [4, 7]).
8. Per-ecosystem test distribution matches the Test allocation table exactly.
9. `commit.txt` is well-formed with no placeholders.
10. `cargo clippy --workspace --all-targets -- -D warnings` passes cleanly.

After the reviewer approves, the task is done. Clay runs `gtxt` to commit the bundled message, then `git pm` to push the branch, create the PR, and auto-merge. The phase is then complete.

---

## Post-merge updates

After `git pm` merges Phase 4 to main:

1. Update `project_ecosystem_modules_refactor.md` in scrat memory to mark Phase 4 complete with the squash commit SHA and PR number. Change the 4-phase table's Phase 4 row status to `COMPLETE — merged YYYY-MM-DD as PR #N`.
2. Update the scrat auto-memory `MEMORY.md` index entry for the refactor to reflect completion of the arc.
3. Write a handoff at `.handoffs/YYYY-MM-DD-HHMM-ecosystem-modules-phase-4-complete.md` summarizing the refactor's outcome and any follow-ups discovered during execution.
