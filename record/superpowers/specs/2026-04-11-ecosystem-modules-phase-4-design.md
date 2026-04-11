# Ecosystem modules refactor — Phase 4

**Date:** 2026-04-11
**Status:** Draft
**Scope:** Unify `bump/`, `detect/`, `deps/` directories and `preflight::check_registry_auth` into a single `ecosystem/<name>.rs` tree implementing a unified `EcosystemDriver` trait. Fourth and final phase of the four-phase ecosystem-modules refactor arc.

## Problem

After Phases 1–3, per-ecosystem logic is scattered across three sibling directories (`bump/{rust,node,php,python,ruby}.rs`, `detect/{rust,node,go,php,python,ruby,swift}.rs`, `deps/{rust,node,go,php,python,ruby,swift}.rs`) plus a registry-auth match table inside `preflight.rs`. Adding a new ecosystem means touching four places. Phase 3 introduced the first trait (`LockfileDiffParser`) with a load-bearing `&self` receiver slot reserved for Phase 4's unification.

Phase 4 collapses the scatter into one file per ecosystem implementing a single trait, and eliminates four runtime dispatch match tables in favor of one factory method on the `Ecosystem` enum.

## Where this fits in the arc

| Phase | Goal | Output | Status |
|-------|------|--------|--------|
| **1** | Finish `detect/` split | `detect/{rust,node,go,php,python,ruby,swift}.rs` + normalized dispatch | **Complete** (PR #37, `0765242`) |
| **2** | Extract `bump/` with harmonized `BumpResult<Vec<String>>` return | `bump/{rust,node,php,python,ruby}.rs`; no trait (deliberate) | **Complete** (PR #38, `bbdd2ab`) |
| **3** | Extract `deps/` with `LockfileDiffParser` trait | `deps/{rust,node,go,php,python,ruby,swift}.rs`; first trait introduction | **Complete** (PR #40, `8c2cee3`) |
| **4** | Unify into `ecosystem/<name>.rs` with `EcosystemDriver` trait + absorb `check_registry_auth` | Single file per ecosystem; `bump/`, `deps/`, `detect/` directories collapsed; four match tables eliminated | **This spec** |

## Scope

**In scope:**
- `bump/`, `deps/`, `detect/` directory collapse into `ecosystem/<name>.rs`
- `EcosystemDriver` trait with four methods (`detect`, `bump_version_files`, `parse_lockfile_diff`, `check_registry_auth`)
- `Ecosystem::driver()` factory method returning `&'static dyn EcosystemDriver`
- `Ecosystem::marker_file()` → `Ecosystem::marker_files()` (singular → slice) with code comment explaining why
- Migration of `preflight::check_registry_auth` match table into per-driver implementations
- Collapse `bump/mod.rs`, `deps/mod.rs`, `detect/mod.rs` directories down to flat files `bump.rs`, `deps.rs`, `detect.rs` at the crate root

**Out of scope:**
- The rest of `preflight.rs` (git status, release branch, remote sync, tool presence, gh auth, tag availability) — not ecosystem-specific
- Pluggable / out-of-tree driver registration via runtime-named drivers (closed enum stays closed)
- A `Format::MarkdownFrontmatter` addition (not needed — `VersionFileFormat::Frontmatter` already exists in `config.rs:344`, covering the cheap path for AgentSkills-style projects today via `project.type = "generic"` + `[[version_files]]` config)

**Prerequisite (NOT part of Phase 4):**
- **Comment-polish PR** must land before Phase 4 Task 1. Touches `deps/node.rs:67-75` (context-line fallback comment), `deps/mod.rs:194` (orphan `// ── JSON string extractor ───` divider), Go/Ruby byte-slicing unification with Rust/PHP/Swift's `strip_prefix` pattern, and `deps/mod.rs` `mod`/`pub use` blank-line tightening. See `.handoffs/2026-04-11-1624-ecosystem-modules-phase-3-complete.md` for exact file:line pointers.

## Module layout

After Phase 4:

```
crates/scrat-core/src/
├── bump.rs          ← was bump/mod.rs — orchestration only, no siblings
├── deps.rs          ← was deps/mod.rs — compute_deps dispatch only
├── detect.rs        ← was detect/mod.rs — resolve/detect/scan dispatch only
├── ecosystem/
│   ├── mod.rs       ← EcosystemDriver trait + Ecosystem::driver() factory
│   │                   + shared lockfile diff helpers (pub(super)) + re-exports
│   ├── types.rs     ← Ecosystem, VersionStrategy, ProjectDetection,
│   │                   DetectedTools, ChangelogTool
│   ├── rust.rs      ← RustDriver: detect + bump + parse_lockfile_diff + check_registry_auth
│   ├── node.rs      ← NodeDriver
│   ├── go.rs        ← GoDriver
│   ├── php.rs       ← PhpDriver
│   ├── python.rs    ← PythonDriver (delegates parser to RustDriver — incidental format match)
│   ├── ruby.rs      ← RubyDriver
│   ├── swift.rs     ← SwiftDriver
│   └── generic.rs   ← GenericDriver (no-op implementations)
├── preflight.rs     ← check_registry_auth becomes a one-liner dispatching to driver
└── ... (unchanged)
```

### File migration — per ecosystem

Each `ecosystem/<name>.rs` absorbs three current sibling files plus one match arm from `preflight.rs`:

| Source | Destination | Becomes |
|---|---|---|
| `detect/<name>.rs::detect_<name>` | `ecosystem/<name>.rs` | `impl EcosystemDriver::detect` |
| `bump/<name>.rs::bump_<name>_version` | `ecosystem/<name>.rs` | `impl EcosystemDriver::bump_version_files` |
| `deps/<name>.rs::<Name>LockfileParser::parse_diff` | `ecosystem/<name>.rs` | `impl EcosystemDriver::parse_lockfile_diff` (method rename) |
| `preflight::check_registry_auth` match arm for this ecosystem | `ecosystem/<name>.rs` | `impl EcosystemDriver::check_registry_auth` |
| Per-ecosystem private helpers (Ruby's byte walkers, Node's JSON helpers) | `ecosystem/<name>.rs` | Private `fn` in same file |
| Per-ecosystem tests from detect/bump/deps/preflight | `ecosystem/<name>.rs::tests` | Merged test module |

### Orchestration files after collapse

- **`bump.rs`** holds: `plan_bump`, `plan_bump_with_detection`, `resolve_interactive`, `resolve_strategy`, `BumpPlan`, `ReadyBump`, `InteractiveBump`, `BumpError`, `BumpOutcome`, `generate_changelog`, 25 public bump tests. `ReadyBump::execute`'s 8-arm dispatch collapses to `self.detection.ecosystem.driver().bump_version_files(project_root, &self.next, &self.detection)?` plus Ruby's post-dispatch special-case check (see Per-driver quirks below).
- **`deps.rs`** holds: `compute_deps`. Dispatch collapses to `ecosystem.driver().parse_lockfile_diff(&diff)`.
- **`detect.rs`** holds: `resolve_detection`, `detect_project`, `detect_ecosystem`, `detect_version_strategy`, `build_detection`, `build_detection_for`, `has_binary`, `check_tool_version`, `parse_version_from_output`, `ToolVersionCheck`, `MIN_GIT_CLIFF_VERSION`, 12 dispatch + tool-helper tests. `build_detection_for`'s 8-arm dispatch collapses to `ecosystem.driver().detect(project_root, version_strategy)`.
- **`preflight.rs`** keeps everything except `check_registry_auth`. Its body collapses to `det.ecosystem.driver().check_registry_auth()`.

### `marker_file()` → `marker_files()`

The current signature `Ecosystem::marker_file(self) -> Option<&'static str>` returns a single file. Phase 4 changes it to a slice:

```rust
impl Ecosystem {
    /// Filenames that signal this ecosystem when any of them is found
    /// in a directory.
    ///
    /// Returns a slice to support ecosystems where multiple marker files
    /// can indicate the same project type — for example, a future
    /// `AgentSkill` variant might match `plugin.json`, `.claude-plugin/plugin.json`,
    /// and `.bito.yaml`. Every current ecosystem returns a single-element
    /// slice; [`Generic`](Self::Generic) returns an empty slice.
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

The inline code comment is load-bearing: it documents *why* the slice exists when every current caller returns one element, so future readers don't "simplify" it back.

`detect::detect_ecosystem` loops the inner slice:

```rust
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

`lockfile_path()` stays singular — every current ecosystem has exactly one canonical lockfile, and multi-lockfile scenarios within a single ecosystem (pnpm vs yarn vs npm for Node) warrant separate driver variants, not a slice on one ecosystem.

**Caller audit (exhaustive `rg 'marker_file\b' crates/` at design time):**

| File | Line | Usage |
|---|---|---|
| `ecosystem.rs` | 56 | Method definition (becomes `marker_files`) |
| `ecosystem.rs` | 243 | Test: `Ecosystem::Rust.marker_file() == Some("Cargo.toml")` |
| `ecosystem.rs` | 244 | Test: `Ecosystem::Node.marker_file() == Some("package.json")` |
| `ecosystem.rs` | 245 | Test: `Ecosystem::Generic.marker_file() == None` |
| `detect/mod.rs` | 88 | `detect_ecosystem` loop: `if let Some(marker) = ecosystem.marker_file()` |

No CLI crate callers, no display renderings, no `init.rs` usage. Task 2's edit surface is limited to these 5 lines (definition + 3 test assertions + 1 loop body). Plan writer re-verifies with the same `rg` command before Task 2 starts in case new callers landed post-design.

## The EcosystemDriver trait

```rust
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
    /// uniformly so future drivers (Node with npm, …) can opt in without
    /// signature churn.
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
    /// non-fatal" contract. Implementations must sort by `DepChange.name`
    /// for deterministic output.
    fn parse_lockfile_diff(&self, diff: &str) -> Vec<DepChange>;

    /// Check registry auth for the publish phase.
    ///
    /// Uses fast env-var checks (no network). Returns a pre-populated
    /// "no registry for this ecosystem" passing result for Go, PHP,
    /// Swift, and Generic.
    fn check_registry_auth(&self) -> CheckResult;
}
```

### Method rationale

- **`detect`** — Dispatch from `detect::build_detection_for` collapses to `ecosystem.driver().detect(...)`. Every existing `detect_<name>(project_root, version_strategy)` function already has this signature; mechanical rename.
- **`bump_version_files`** — Carries `&ProjectDetection` because Rust reads `detection.tools.bump_cmd` to find `cargo set-version`. Forcing every driver to accept it means Rust doesn't need a different signature, and future drivers that want tool-command config get it for free.
- **`parse_lockfile_diff`** — Mechanical rename from Phase 3's `LockfileDiffParser::parse_diff`. Phase 3 preserved `&self` specifically to enable this absorption. The old `LockfileDiffParser` trait gets deleted at Task 3.
- **`check_registry_auth`** — Absorbs `preflight::check_registry_auth`. Each driver owns its env-var list + registry name + login hint inline.

### What stays on `Ecosystem` (not on the trait) and why

- `marker_files() -> &'static [&'static str]` — pure static data, const lookup.
- `lockfile_path() -> Option<&'static str>` — pure static data.
- `bump_config() -> &'static str` — static git-cliff TOML.
- `AUTO_DETECTABLE`, `ALL` — static slices.

Putting these on the trait would force every driver to implement a const lookup. The trait is for *behavior*; the enum stays the home of *data*.

## Dispatch via `Ecosystem::driver()`

```rust
impl Ecosystem {
    /// Return the [`EcosystemDriver`] implementation for this ecosystem.
    ///
    /// Drivers are zero-sized unit structs; the returned reference is
    /// `'static` and incurs no allocation.
    pub fn driver(self) -> &'static dyn EcosystemDriver {
        match self {
            Self::Rust => &RustDriver,
            Self::Node => &NodeDriver,
            Self::Go => &GoDriver,
            Self::Php => &PhpDriver,
            Self::Python => &PythonDriver,
            Self::Ruby => &RubyDriver,
            Self::Swift => &SwiftDriver,
            Self::Generic => &GenericDriver,
        }
    }
}
```

Method-on-enum over free function because call sites read subject.verb: `detection.ecosystem.driver().bump_version_files(...)` flows left-to-right without reordering.

### Call site migrations

| File | Before | After |
|---|---|---|
| `bump.rs`, `ReadyBump::execute` | 8-arm match on `self.detection.ecosystem` | `self.detection.ecosystem.driver().bump_version_files(project_root, &self.next, &self.detection)?` |
| `deps.rs`, `compute_deps` | 8-arm match returning `Vec<DepChange>` | `ecosystem.driver().parse_lockfile_diff(&diff)` |
| `detect.rs`, `build_detection_for` | 8-arm match returning `ProjectDetection` | `ecosystem.driver().detect(project_root, version_strategy)` |
| `preflight.rs`, `check_registry_auth` (called from `run_preflight`) | 4-arm match table inside the function | `det.ecosystem.driver().check_registry_auth()` |

Four match tables collapse to four one-liners.

### Ruby's caller-side "no files modified" exception

The current `ReadyBump::execute` has this Ruby-specific post-call block:

```rust
Ecosystem::Ruby => {
    let files = ruby::bump_ruby_version(project_root, &self.next)?;
    if files.is_empty() && self.version_files.is_empty() {
        return Err(BumpError::ToolFailed { /* "no version.rb/gemspec" */ });
    }
    modified_files.extend(files);
}
```

The check depends on BOTH the driver's return value AND `ReadyBump::version_files` config — a caller-layer release-correctness rule (the release must not tag without updating any file), not a driver concern. **Phase 4 preserves this in `bump.rs` post-dispatch:**

```rust
let ecosystem_files = self.detection.ecosystem.driver()
    .bump_version_files(project_root, &self.next, &self.detection)?;
modified_files.extend(ecosystem_files.clone());

// Ruby-specific: if the driver found nothing AND there are no version_files,
// the release would tag without updating any file. Block it.
if self.detection.ecosystem == Ecosystem::Ruby
    && ecosystem_files.is_empty()
    && self.version_files.is_empty()
{
    return Err(BumpError::ToolFailed { /* same message */ });
}
```

This is an intentional asymmetry that stays visible in `bump.rs` — it documents a semantic caller-level rule rather than hiding it inside `RubyDriver`. Per Phase 3's guidance: *"don't let cosmetic consistency bulldoze semantic distinctions."*

## Per-driver quirks and landmines to preserve

These are the asymmetries Phase 4 must preserve. A well-meaning plan writer would be tempted to "fix" each of them; every one is load-bearing.

| # | Quirk | Why it exists | Phase 4 rule |
|---|---|---|---|
| 1 | **`PythonDriver::parse_lockfile_diff` calls `RustDriver.parse_lockfile_diff(diff)` directly** | `uv.lock` uses the same TOML `[[package]]` format as `Cargo.lock`. **Incidental format match**, not a shared abstraction. | Preserve the delegation. Add the same module-level doc comment Phase 3 added to `deps/python.rs` (see `deps/python.rs:1-11`). Do **NOT** extract a shared `parse_toml_package_diff` helper. |
| 2 | **`RustDriver::bump_version_files` reads `detection.tools.bump_cmd`** | Rust shells out to whatever tool `detect_rust` found on `PATH` (typically `cargo set-version`). Other ecosystems do in-process file rewriting. | Keep `detection: &ProjectDetection` on the trait method. Only `RustDriver` reads it today; the uniform signature reserves the slot. |
| 3 | **Ruby's "no files modified" check stays in `bump.rs::ReadyBump::execute`, post-dispatch** | The check depends on BOTH the driver's result AND `ReadyBump::version_files` config. Caller-layer release-correctness rule. | Leave the `if` block in `bump.rs`. `RubyDriver::bump_version_files` returns `Vec<String>` — empty is a valid state it does not error on. |
| 4 | **`NodeDriver::parse_lockfile_diff` reports top-level dependencies only, not transitive** | Intentional. npm lockfile v2/v3 has thousands of transitive entries; release notes want direct deps. | Preserve. Driver doc comment must say "top-level only by design." A future reader seeing "Node only reports some deps" might think it's a bug. |
| 5 | **`GenericDriver` implements every trait method with empty/no-op returns** | Generic is a first-class ecosystem (not an `Option<Driver>`) so dispatch has no special cases at call sites. | Write `GenericDriver` as a full driver impl. ~25 LOC. Call sites must NOT match on `Ecosystem::Generic` to skip the driver — they trust the no-op bodies. |
| 6 | **`RustDriver::bump_version_files` returns `Err(BumpError::NoBumpTool)` when `detection.tools.bump_cmd.is_none()`** | Rust can't bump without `cargo set-version`. Other ecosystems (Node: in-process JSON edit) don't shell out. | Preserve. Keep `BumpError::NoBumpTool` as a Rust-exclusive failure mode. Don't generalize it. |
| 7 | **Cosmetic inconsistency between Go/Ruby `content[1..]` and Rust/PHP/Swift `strip_prefix` in lockfile parsers** | Both styles are bytewise-safe single-char skips. | Resolved by the pre-Phase-4 **comment-polish PR**. Phase 4 inherits the post-polish shape. |

**Anti-refactor instructions** for implementer subagents must repeat each of these verbatim in the relevant task specs.

## Tests

### Principle

Tests colocate with the code they exercise. After Phase 4, all Rust-specific tests live in `ecosystem/rust.rs::tests`; all Ruby tests live in `ecosystem/ruby.rs::tests`; etc. Orchestration tests stay in the orchestration file they test.

### Per-ecosystem test file targets

| Driver file | Sources | Expected count |
|---|---|---|
| `ecosystem::rust::tests` | `detect::rust::tests` (3) + `deps::rust::tests` (7) + `preflight::check_registry_auth_rust` (1) | **11** |
| `ecosystem::node::tests` | `deps::node::tests` (15) + `preflight::check_registry_auth_node` (1) | **16** |
| `ecosystem::go::tests` | `deps::go::tests` (9) + `preflight::check_registry_auth_go_skips` (1) | **10** |
| `ecosystem::php::tests` | `deps::php::tests` (7) | **7** (no preflight registry arm today) |
| `ecosystem::python::tests` | `deps::python::tests` (3) + `preflight::check_registry_auth_python` (1) | **4** |
| `ecosystem::ruby::tests` | `deps::ruby::tests` (7) + `bump::ruby::tests` (19) + `preflight::check_registry_auth_ruby` (if exists) | **26 or 27** |
| `ecosystem::swift::tests` | `deps::swift::tests` (7) | **7** |
| `ecosystem::generic::tests` | `preflight::check_registry_auth_generic_skips` (1) + 3–4 no-op contract tests | **4–5** |

Plan writer verifies exact preflight test names and counts against the current `preflight.rs::tests` module during Task 6 plan step. The 5 confirmed registry_auth test names from design-time grep: `check_registry_auth_rust`, `check_registry_auth_node`, `check_registry_auth_python`, `check_registry_auth_go_skips`, `check_registry_auth_generic_skips`. Ruby/PHP/Swift may or may not have individual tests — if absent, nothing migrates.

### Orchestration test file targets

Tests colocate with the code they subject. Types tests go with types; shared-helper tests go with helpers.

| File | Sources | Expected count |
|---|---|---|
| `ecosystem::types::tests` | current `ecosystem.rs::tests` (10: display, serde, marker_files, bump_config checks) | **10** |
| `ecosystem::tests` (in `mod.rs`) | `deps::tests` shared-helper tests (4: TOML/JSON extractors `_basic` and `_no_match`) | **4** |
| `bump.rs::tests` | current `bump::tests` (25 public) | **25** (unchanged) |
| `deps.rs::tests` | possibly 0–2 thin `compute_deps` dispatch smoke tests | **0–2** |
| `detect.rs::tests` | current `detect::tests` (12 dispatch + tool helpers) | **12** |
| `preflight.rs::tests` | current count minus registry_auth tests that migrate to per-ecosystem modules (5 confirmed at design time, possibly more if Ruby/PHP/Swift have individual tests) plus one integration test verifying `run_preflight` still routes registry check through driver | exact number resolved at Task 6 plan step |

### Load-bearing invariants

1. **Workspace test count** = baseline + N, where N is the sum of Phase-4 *new* tests tracked explicitly:
    - `ecosystem::generic::tests`: **+4** no-op contract tests (`bump_version_files`, `parse_lockfile_diff`, `check_registry_auth`, `detect`)
    - `deps.rs::tests`: **+0 to +2** optional `compute_deps` dispatch smoke tests (plan writer's call at Task 3)
    - `preflight.rs::tests`: **+1** optional integration test verifying `run_preflight` still calls registry check through driver (plan writer's call at Task 6)
    - **Expected N ∈ [4, 7].** Baseline captured at Task 1; Task 7's gate asserts `post = baseline + N` where N is computed against whatever optional tests the plan writer elected to add. Any test delta outside the named list is a regression and blocks the PR.
2. **Per-ecosystem distribution checks** at each task: `cargo nextest list -p scrat-core ecosystem::<name>::` asserts the count matches the per-ecosystem table above.
3. **No test-name collisions after Task 5 merge** — `bump::ruby::tests` and `deps::ruby::tests` both use tempdir-based fixtures. Plan writer runs `cargo nextest list -p scrat-core ecosystem::ruby::` and checks for duplicate leaf names before completing Task 5. If any exist, rename the bump-side test to preserve history intent.
4. **Phase 3's 59-deps-test invariant becomes distributed** — 4 shared-helper tests move to `ecosystem::tests` (inside `mod.rs`), 55 distribute per driver. Sum check: `7+15+9+7+3+7+7 = 55` + `4 shared = 59` ✓.

### Verification command for Task 7

```bash
cargo nextest list -p scrat-core 2>&1 | \
  grep -E "^(ecosystem|bump|deps|detect|preflight)::" | \
  wc -l
```

Compare against the baseline number captured at Task 1. Gate Task 7 on `post = baseline + N` where N is the total count of new tests added from the invariant list above (expected N ∈ [4, 7]).

### `GenericDriver` no-op contract tests

Add **4** tests in `ecosystem/generic.rs::tests` asserting:
- `bump_version_files` returns `Ok(Vec::new())`
- `parse_lockfile_diff(anything)` returns `Vec::new()`
- `check_registry_auth()` returns a passing `CheckResult` with message "No registry publish for this ecosystem"
- `detect(root, strategy)` returns `ProjectDetection::generic(strategy)`

These surface the no-op contract in-file and cost nothing. Count of 4 is load-bearing for the invariant in the previous section.

## Execution plan

### Task sequence — 7 tasks total

| # | Task | Atomic? | LOC size | Review |
|---|---|---|---|---|
| 1 | Branch setup (`feat/ecosystem-modules-phase-4`), capture test count baseline, write real complete-sentence intro paragraph into `commit.txt` (not a placeholder — Phase 2's Task 2 skeleton leaked `[body to be appended…]` into merged commit `bbdd2ab`) | N/A | small | spec review inline |
| 2 | Module split: `ecosystem.rs` → `ecosystem/{mod.rs, types.rs}`; `marker_file()` → `marker_files()` with load-bearing code comment; update `detect_ecosystem` loop and `ecosystem_marker_files` test assertion. No trait yet, no drivers yet. | ✅ | small | spec review inline |
| 3 | **[BIGGEST]** Introduce `EcosystemDriver` trait with only `parse_lockfile_diff`. Create 8 driver files. Migrate all Phase 3 deps parsers into drivers. Preserve Python's delegation to `RustDriver`. Move shared helpers (`emit_change`, `extract_toml_string_value`, `extract_json_string_value`) to `ecosystem/mod.rs` at `pub(super)`. Delete `LockfileDiffParser` trait. Collapse `deps/` → `deps.rs`. Migrate 59 tests per the distribution table. Add `Ecosystem::driver()` factory. | ✅ | large | **code quality review** |
| 4 | Grow trait with `detect` method. Migrate 7 per-ecosystem `detect_<name>` functions into driver impls. Generic returns `ProjectDetection::generic(strategy)`. Delete `detect/<name>.rs` siblings. Collapse `detect/` → `detect.rs`. Migrate 3 `detect/rust.rs::tests` to `ecosystem::rust::tests`. | ✅ | medium | spec review inline (mechanical replication of Task 3 template) |
| 5 | **[SECOND-BIGGEST]** Grow trait with `bump_version_files` method. Migrate 5 per-ecosystem bump helpers (rust, node, php, python, ruby). Preserve Rust's `&ProjectDetection` dependency + Ruby's caller-side "no files modified" check in `ReadyBump::execute`. Go/Swift/Generic drivers return `Ok(Vec::new())` with debug log. Delete `bump/<name>.rs` siblings. Collapse `bump/` → `bump.rs`. Migrate 19 `bump::ruby` private tests to `ecosystem::ruby::tests`. | ✅ | large | **code quality review** — semantic asymmetries |
| 6 | Grow trait with `check_registry_auth` method. Migrate match arms from `preflight::check_registry_auth` into per-driver impls (rust, node, python, ruby get real impls; go, php, swift, generic return "no registry" CheckResult). Update `run_preflight` to dispatch through driver. Migrate registry_auth tests from `preflight.rs::tests` to per-ecosystem test modules. Delete `preflight::check_registry_auth` function. | ✅ | small | spec review inline (mechanical replication) |
| 7 | Final: `lib.rs` rustdoc update (mention `ecosystem::{EcosystemDriver, driver}`), workspace test invariant verification, `cargo clippy -p scrat-core --all-targets -- -D warnings`, `commit.txt` finalization, bundled squash commit prep. | N/A | small | **code quality review** — final PR |

### Review pattern (three-phase-validated)

- **Code quality review (dispatched subagent):** Tasks 3, 5, 7
- **Spec review inline (controller-held):** Tasks 2, 4, 6 (mechanical replications, template validated by Task 3)
- **Dispatch harness:** `superpowers:subagent-driven-development` with `superpowers:requesting-code-review` on the 3 heavy tasks
- **Atomic task flag:** Tasks 2-6 are intermediate-state-doesn't-compile atomic units. Plan must mark them explicitly so substeps land together.

### Known risks the plan doc must name

1. **Test name collisions at Task 5** — `bump::ruby::tests` and `deps::ruby::tests` both use tempdir-based fixtures. Plan writer runs `cargo nextest list -p scrat-core ecosystem::ruby::` before completing the task and resolves any duplicate leaf names by renaming the bump-side test.
2. **Python delegation landmine at Task 3** — explicit "do NOT extract a shared helper" instruction in the task spec. Implementer prompt repeats it. Spec reviewer verifies `PythonDriver::parse_lockfile_diff` body is a one-line call to `RustDriver.parse_lockfile_diff(diff)`.
3. **Ruby caller-side check landmine at Task 5** — explicit "do NOT push this check into `RubyDriver`" instruction. Spec reviewer checks that `ReadyBump::execute` still contains the post-dispatch Ruby special-case block.
4. **`marker_file()` callers not yet enumerated** — Task 2 grep discipline: `rg 'marker_file\b' crates/` exhaustively before editing. Any caller in the CLI crate that renders marker filenames for display needs `.first()` (with fallback to `"(none)"` for Generic) or `.join(", ")`.
5. **Preflight registry tests may not cover all 7 ecosystems** — Task 6 plan writer verifies which ecosystems have individual `check_registry_auth_*` tests today. If a migration target doesn't exist, the task spec says "no tests to migrate for this ecosystem."
6. **Test count baseline gating requires a captured number** — Task 1 plan-landing step records `cargo nextest list -p scrat-core | wc -l` as the baseline; Task 7's verification compares against it with `±3` slack.

### Sandbox flag reminder

Every `cargo`, `just`, or `nextest` invocation requires `dangerouslyDisableSandbox: true` or the whole toolchain returns cryptic compile errors (sccache permissions). Plan must repeat this across all task specs.

### Commit.txt discipline

- Task 1 writes a real complete sentence as the intro paragraph (Phase 2's Task 2 skeleton leaked `[body to be appended as each task completes]` into merged commit `bbdd2ab`; Phase 3 Task 2 used a real sentence and merged cleanly as `8c2cee3`).
- Subsequent tasks **append** sub-bullet sections via the Edit tool, never overwrite (bundled-commit format like `f706dc9`).

### Mixed-bash cascade reminder

Existence-check commands (`ls commit.txt 2>&1`) must be isolated into their own bash calls or forced to exit 0 via `|| true` — exit 1 from a parallel bash batch will cancel the other commands in that batch.

## Prerequisites

1. **Comment-polish PR** lands before Phase 4 Task 1 (see Scope section above).
2. **Test count baseline** captured at Task 1 plan-landing: `cargo nextest list -p scrat-core 2>&1 | grep -E "^(ecosystem|bump|deps|detect|preflight)::" | wc -l`. Record the number in the plan doc.
3. **Feature branch** created before Task 2 starts: `git checkout -b feat/ecosystem-modules-phase-4`.

## References

- Phase 3 completion handoff: `.handoffs/2026-04-11-1624-ecosystem-modules-phase-3-complete.md`
- Phase 2 completion handoff: `.handoffs/2026-04-10-2349-ecosystem-modules-phase-2-complete.md`
- Phase 1 completion handoff: `.handoffs/2026-04-10-2214-ecosystem-modules-phase-1-complete.md`
- Phase 3 plan: `record/superpowers/plans/2026-04-11-ecosystem-modules-phase-3-deps.md`
- Phase 2 plan: `record/superpowers/plans/2026-04-10-ecosystem-modules-phase-2-bump.md`
- Phase 1 plan: `record/superpowers/plans/2026-04-10-ecosystem-modules-phase-1-detect.md`
- Phase 3 squash commit: `8c2cee3` (PR #40)
- Phase 2 squash commit: `bbdd2ab` (PR #38)
- Phase 1 squash commit: `0765242` (PR #37)
- 4-phase arc auto-memory: `project_ecosystem_modules_refactor.md`
