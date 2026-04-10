---
audit: 2026-04-09-full-workspace
last_updated: 2026-04-10
status:
  fixed: 17
  mitigated: 0
  accepted: 1
  disputed: 0
  deferred: 0
  open: 0
---

# Actions Taken: scrat 2026-04-09 Full Workspace Audit

Summary of remediation status for the [2026-04-09 full workspace audit](README.md).

---

## 2026-04-10 — Bundled audit remediation (tiers 1–5)

**Disposition:** fixed
**Addresses:**
[hooks-filter-truncate-panic](README.md#byte-index-truncation-of-filter-output-panics-on-multi-byte-utf-8),
[hooks-stdin-write-silently-discarded](README.md#stdin-write-error-silently-discarded-in-filter-hook),
[git-fetch-silently-discarded](README.md#git-fetch-failure-silently-discarded-in-remote-sync-check),
[node-bump-hard-error](README.md#node-ecosystem-bump-returns-hard-error-blocking-scrat-ship),
[node-deps-parser-stub](README.md#node-dependency-diff-parser-is-a-no-op-stub),
[ruby-bump-silently-skipped](README.md#ruby-version-bump-silently-skipped-with-debug-only-message),
[detect-stubs-no-tool-probing](README.md#five-ecosystem-detectors-are-stubs-without-tool-probing),
[pipeline-context-serialized-unconditionally](README.md#pipelinecontext-serialized-to-json-up-to-12-times-per-ship-run),
[duplicate-detection-calls](README.md#ecosystem-detection-runs-2-3-times-for-the-same-project-root),
[redundant-git-current-branch](README.md#git-current_branch-spawns-3-processes-for-an-invariant-value),
[preflight-git-fetch-blocks-startup](README.md#preflight-runs-git-fetch-on-every-ship-invocation),
[version-file-config-mutual-exclusion](README.md#versionfileconfig-fieldfields-mutual-exclusion-not-type-enforced),
[pipeline-context-init-missing-debug](README.md#pipelinecontextinit-is-a-public-type-missing-debug-derive),
[scrat-core-lib-incomplete-reexports](README.md#scrat-core-librs-does-not-re-export-key-types-for-library-consumers),
[pipeline-types-missing-eq-hash](README.md#pipeline-types-lack-partialeq-for-testing-by-library-consumers),
[conventional-version-expect-temppath](README.md#expect-on-temp-file-path-utf-8-conversion-in-library-code),
[cli-spinner-template-unwrap](README.md#progressstylewith_templateunwrap-inconsistent-with-project-pattern)
**Commit:** _(see PR linked from front matter once merged)_
**Author:** @claylo

Single bundled commit on `fix/audit-2026-04-09` addressing every actionable finding from the audit. Verified end-to-end against real fixtures for all 7 ecosystems via `scripts/smoke-ecosystems.sh` (33/33) plus the full unit suite (580/580). Workspace clippy is clean.

### Tier 1 — correctness (3 findings)

The filter-hook truncation panic was the most urgent: any filter command that produced invalid JSON containing a non-ASCII character in the first 200 bytes would panic on `&trimmed[..200]`. Replaced the byte-index slice with a `floor_char_boundary` snap (the audit claimed this method was 1.82.0-stable, but it's actually 1.91.0 — see the MSRV note below). The stdin-write discard now logs a warning, and the git-fetch failure in `is_remote_in_sync` now logs at debug so degraded checks leave a diagnostic trail.

```rust crates/scrat-core/src/hooks.rs
fn truncate_at_char_boundary(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let boundary = s.floor_char_boundary(max_bytes);
    format!("{}...", &s[..boundary])
}
```

### Tier 2 — ecosystem completeness (4 findings)

This is the tier that mattered most. The previous `bump.rs` returned `BumpError::UnsupportedEcosystem` for Node — the second-most-common ecosystem the README claims to support — and silently skipped Ruby with a debug-only log. The Node `package-lock.json` deps parser was a `const fn` returning an empty `Vec`. The five non-Rust detectors hardcoded tool commands without checking PATH, and never wired the detected `git-cliff` into the `changelog_tool` field.

All four are now real implementations with comprehensive unit tests:

- **Node bump**: direct `serde_json` edit of `package.json` only. Hard-errors if no `version` field exists. Does **not** touch `package-lock.json` (lockfile sync is the user's package manager's responsibility — `npm install --package-lock-only` in a hook if needed).
- **Ruby bump**: byte-level parser walks `lib/**/version.rb` and `*.gemspec`. Preserves indent, quote style, and `.freeze` suffixes. Refuses to overwrite gemspec constant references like `spec.version = MyGem::VERSION` so `version.rb` remains the source of truth. Hard-errors if it finds nothing to bump and no `[[version_files]]` are configured — no more silent passes.
- **`package-lock.json` parser**: real JSON state machine on lockfile v2/v3. Reports top-level deps only (nested dedup entries under `node_modules/foo/node_modules/bar` are intentionally skipped). Handles scoped packages (`@scope/name`) and diff headers correctly. 16 unit tests covering update/added/removed/scoped/nested/mixed/header cases.
- **Detect tool probing**: every non-Rust detector now probes the relevant binaries on PATH (`pnpm`/`yarn`/`npm` for Node; `uv`/`pytest`/`python`/`twine` for Python; `bundle`/`rake`/`gem` for Ruby; `composer`, `go`, `swift` for the others) and degrades gracefully when they're missing. A new `VersionStrategy::changelog_tool()` const helper threads the detected git-cliff state into every ecosystem's `DetectedTools.changelog_tool` field — no more silent changelog skips when CC strategy is detected.

### Tier 3 — pipeline efficiency (4 findings)

Detection used to run 2–3 times per `scrat ship` invocation; `current_branch()` spawned 3 separate `git rev-parse` processes; the `PipelineContext` was JSON-serialized at every hook point even when no `filter:` hooks existed; and the preflight `git fetch` blocked startup unconditionally on a network round-trip.

- `plan_ship` now computes detection once and threads it through `run_preflight_with_detection` and `plan_bump_with_detection`.
- `run_git_phase` takes a `branch_hint: Option<&str>` parameter; the caller reads from `ctx.branch` (already populated by the pipeline init).
- `run_phase_hooks` checks `cmds.iter().any(|c| c.trim_start().starts_with("filter:"))` before serializing the pipeline context — common case is now zero-cost.
- New `--no-fetch` flag (CLI flag, `ShipOptions::no_fetch`, `[ship] no_fetch` config) and `git::is_remote_in_sync(fetch_remote: bool)` so users can opt into stale-but-fast preflight when they just pushed.

### Tier 4 — type polish (5 findings)

`PipelineContextInit` gained `Debug, Clone`. `ReleaseStats`, `Contributor`, and `DepChange` gained `PartialEq, Eq`. `PipelineContext` gained `PartialEq` (an `#[expect(clippy::derive_partial_eq_without_eq)]` with reason explains why `Eq` is unsound — `metadata: HashMap<String, serde_json::Value>` holds `f64` via `Value::Number`).

`VersionFileConfig` was refactored to use a `VersionFields { Single(String), Multiple(Vec<String>) }` enum, eliminating the field/fields representable-but-invalid state at the type level. Custom `Deserialize` keeps the on-disk YAML/TOML config syntax (`field: "..."` or `fields: [...]`) backward compatible and rejects configs that supply both with a clear error at parse time. Custom `Serialize` writes the legacy shape on round-trip.

Library re-exports added in `crates/scrat-core/src/lib.rs`: `Ecosystem`, `ProjectDetection`, `VersionStrategy`, `PipelineContext`, `PipelineContextInit`, `ShipOptions`, `ShipPlan`, `VersionFields`, `VersionFileConfig`, `VersionFileFormat`. Library consumers no longer need to navigate the module hierarchy for the common types.

The `expect()` on the temp-file UTF-8 conversion in `version/conventional.rs` was replaced with `ok_or_else(|| VersionError::ToolFailed { ... })?`.

### Tier 5 — cosmetic (1 finding)

`ProgressStyle::with_template().unwrap()` in `crates/scrat/src/commands/ship.rs` is now `.expect("valid spinner template")`, matching `doctor.rs`.

### MSRV bump (related cleanup)

The audit asserted `floor_char_boundary` was 1.82.0-stable; in reality it stabilized in 1.91.0 — past the prior MSRV of 1.89.0. Rather than carry a manual `is_char_boundary` walk indefinitely, the workspace MSRV was bumped from 1.89.0 to **1.94.1** to match `rust-toolchain.toml`. This eliminates the gap between dev toolchain and minimum supported version, making "is feature X stable yet?" guesswork unnecessary going forward. README badge and `scripts/add-crate` template updated; CI workflow reads MSRV dynamically from `Cargo.toml` so no hardcoded version to update.

### README parity sweep

While verifying each finding's fix end-to-end, several README claims turned out to be stale relative to the new behavior:

- The bump section said "Ruby skips version-file rewrite" — now describes the `lib/**/version.rb` and gemspec handling.
- The deps diff table said `Node | (stub — returns empty, full parser planned)` — now describes the JSON state machine.
- The test command table listed `npm test` for Node and `pytest` for Python — now shows the actual probing order (`pnpm > yarn > npm`, `uv run pytest > pytest`, etc.).
- The publish table similarly updated to show the probing order for Node and Python.
- The `--no-fetch` flag is now documented in the CLI flag table and the `[ship]` config example.

These docs gaps were the load-bearing concern — the previous-agent had told the user that all 7 ecosystems were production-ready when Node bump was a hard error and the package-lock parser was a no-op. The README now matches what the code actually does, every claim is backed by a real test, and `scripts/smoke-ecosystems.sh` produces durable evidence.

### Verification

- `cargo build -p scrat`: clean
- `cargo clippy --workspace --all-targets`: 0 warnings
- `cargo nextest run --workspace`: 580/580 passed
- `scripts/smoke-ecosystems.sh`: 33/33 passed across all 7 ecosystems, with each lockfile parser producing real `Dependencies` entries in the rendered release notes (not just exiting cleanly)

---

## 2026-04-10 — clap-in-core-crate accepted as-is

**Disposition:** accepted
**Addresses:** [clap-in-core-crate](README.md#scrat-core-depends-on-clap-despite-being-a-library-crate)
**Commit:** n/a
**Author:** @claylo

The audit flagged that `scrat-core` pulls in `clap` (with the `derive` feature) solely so two enums in `init.rs` (`ConfigFormat`, `ConfigStyle`) can derive `ValueEnum`. Moving them to the CLI crate or splitting the derive would be the architecturally clean fix.

We're accepting this as-is for now, on the basis that:

1. The CLI binary already depends on `clap`, so the compile-time and binary-size cost is paid exactly once. There is no measurable runtime or build-time penalty.
2. There are no current downstream consumers of `scrat-core` as a library — embedding it elsewhere is hypothetical, and the `clap` coupling only matters in that hypothetical scenario.
3. The fix would touch the `init` workflow which is otherwise stable and well-tested, in exchange for removing a dependency that everyone using `scrat` already has.

If `scrat-core` ever picks up an external library consumer, this becomes a real problem and should be revisited. Until then, the dependency stays.
