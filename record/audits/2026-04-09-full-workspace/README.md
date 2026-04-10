---
audit_date: 2026-04-09
project: scrat
commit: f706dc96b708f63fc3c47f9dd09ca30fd89438a8
scope: Full workspace audit — crates/scrat (CLI) and crates/scrat-core (library)
auditor: claude-opus-4-6 (crustoleum rubric, 5 parallel agents + reviewer)
findings:
  critical: 0
  significant: 2
  moderate: 3
  advisory: 8
  note: 5
---

# Audit: scrat

scrat is a young Rust release-management CLI (2 months, ~18k LOC) with a disciplined architecture: thin CLI over fat core, typed error hierarchy via thiserror, zero `unwrap()` in production library code, and a clean dependency tree (no CVEs, no unused deps, no async runtime for a synchronous tool). **The Shell Execution Boundary** — where scrat spawns git, git-cliff, gh, and user-defined hooks — is the primary risk surface, with one panic reachable from external input and two silent error discards. **The Ecosystem Completeness Surface** documents 7 ecosystems but delivers full capability only for Rust; Node bump fails hard. **The Pipeline Efficiency Surface** has redundant subprocess spawns and unconditional JSON serialization adding 1-3 seconds per release. **The Type Design Surface** is sound for the CLI consumer but has missing derives and a library-depends-on-clap coupling. **The Error Architecture Surface** is the strongest aspect — textbook Rust. **The Supply Chain Surface** is clean: no advisories, no unused deps, all dependencies justified. Fix the hook panic and wire up Node, and this is solid infrastructure.

---

## The Shell Execution Boundary

*The interface between scrat and external processes handles the happy path well but has gaps in error reporting and one panic reachable from untrusted output.*

### Byte-index truncation of filter output panics on multi-byte UTF-8

**significant** · `crates/scrat-core/src/hooks.rs:393-394` · effort: trivial · <img src="assets/sparkline-hooks-filter-truncate-panic.svg" height="14" alt="commit activity" />

When a `filter:` hook command returns invalid JSON, scrat builds an error message that includes the first 200 characters of the output. The truncation uses byte indexing (`&trimmed[..200]`) on a string whose content is entirely external — it is stdout from a user-defined shell command. If a multi-byte UTF-8 character straddles byte offset 200, `str` indexing panics with "byte index 200 is not a char boundary". Any non-ASCII character in the first 200 bytes of invalid filter output triggers this.

```rust crates/scrat-core/src/hooks.rs:393-394
detail: if trimmed.len() > 200 {
    format!("{}...", &trimmed[..200])
```

**Remediation:** Replace `&trimmed[..200]` with a char-boundary-safe truncation. `floor_char_boundary` is stable since Rust 1.82.0 (MSRV 1.89.0 qualifies):

```rust
let boundary = trimmed.floor_char_boundary(200);
format!("{}...", &trimmed[..boundary])
```

<div>&hairsp;</div>

### stdin write error silently discarded in filter hook

**moderate** · `crates/scrat-core/src/hooks.rs:368-371` · effort: small · <img src="assets/sparkline-hooks-stdin-write-silently-discarded.svg" height="14" alt="commit activity" />

A `filter:` hook receives the full PipelineContext as JSON on stdin. If the write fails partway through (EPIPE on a large context, child exiting mid-read), the child receives truncated JSON. The child may then produce garbage output that happens to be valid JSON, which scrat would merge back into the pipeline context. The comment documents the "child exited early" case but the discard also swallows partial-write errors where the child is still running.

```rust crates/scrat-core/src/hooks.rs:368-371
if let Some(mut stdin) = child.stdin.take() {
    // Ignore write errors — the child may have exited early
    let _ = stdin.write_all(json_stdin.as_bytes());
}
```

Related: [hooks-filter-truncate-panic](#byte-index-truncation-of-filter-output-panics-on-multi-byte-utf-8).

**Remediation:** Log a warning on write failure and check the child's exit status afterward. If the child is still running after a write error, treat it as a hook failure.

<div>&hairsp;</div>

### git fetch failure silently discarded in remote sync check

**moderate** · `crates/scrat-core/src/git.rs:94-95` · effort: trivial · <img src="assets/sparkline-git-fetch-silently-discarded.svg" height="14" alt="commit activity" />

`is_remote_in_sync()` calls `git fetch` to update remote refs before comparing local vs remote HEAD. If the fetch fails (network outage, SSH key issue), the function compares against potentially stale refs and may report "in sync" incorrectly. The comment documents the intent (non-fatal) but no diagnostic is emitted, making it invisible when the preflight safety net degrades.

```rust crates/scrat-core/src/git.rs:94-95
// Fetch to get latest remote state (non-fatal if it fails)
let _ = git(&["fetch", "--quiet"]);
```

**Remediation:** Log at debug level when fetch fails:

```rust
if let Err(e) = git(&["fetch", "--quiet"]) {
    debug!(%e, "fetch failed, comparing with cached remote state");
}
```

*Verdict: The hooks system is architecturally sound — parallel batches, sync barriers, filter pipes — but the error handling at the boundary treats all failures as non-critical. The truncation panic is the most urgent fix; the silent discards are defense-in-depth gaps that degrade diagnostic quality without causing incorrect behavior.*

<!-- whitespace is important -->
<div>&nbsp;</div>

## The Ecosystem Completeness Surface

*scrat documents 7 ecosystems but delivers full capability only for Rust; Node bump fails hard, and the remaining ecosystem detectors are stubs without tool probing.*

### Node ecosystem bump returns hard error, blocking scrat ship

**significant** · `crates/scrat-core/src/bump.rs:262-264` · effort: small · <img src="assets/sparkline-node-bump-hard-error.svg" height="14" alt="commit activity" />

When a user runs `scrat ship` on a Node project, the bump phase immediately returns `BumpError::UnsupportedEcosystem`, aborting the entire pipeline. The detection stub at `detect/mod.rs:122` populates `bump_cmd` with `"npm version --no-git-tag-version"`, but that command is never called. Node is the second most common ecosystem after Rust.

```rust crates/scrat-core/src/bump.rs:262-264
Ecosystem::Node => {
    return Err(BumpError::UnsupportedEcosystem(Ecosystem::Node));
}
```

Enables [node-deps-parser-stub](#node-dependency-diff-parser-is-a-no-op-stub).

**Remediation:** Implement Node version bumping similar to PHP/Python: read `package.json`, check for a `"version"` field, update it with `serde_json`, write back. Alternatively, shell out to the already-detected `npm version --no-git-tag-version`.

<div>&hairsp;</div>

### Node dependency diff parser is a no-op stub

**advisory** · `crates/scrat-core/src/deps.rs:165-170` · effort: medium · <img src="assets/sparkline-node-deps-parser-stub.svg" height="14" alt="commit activity" />

A Node user running `scrat ship` sees no dependency changes in their release notes even when `package-lock.json` has changed. The stub is documented in a doc comment but no user-visible warning is emitted.

```rust crates/scrat-core/src/deps.rs:165-170
/// Parse a unified diff of `package-lock.json` into dependency changes.
///
/// Stub — returns empty for now. Full implementation deferred.
const fn parse_package_lock_diff(_diff: &str) -> Vec<DepChange> {
    Vec::new()
}
```

Enabled by [node-bump-hard-error](#node-ecosystem-bump-returns-hard-error-blocking-scrat-ship).

**Remediation:** At minimum, add a `warn!()` log. Full implementation requires a JSON state machine parser.

<div>&hairsp;</div>

### Ruby version bump silently skipped with debug-only message

**advisory** · `crates/scrat-core/src/bump.rs:282-284` · effort: small · <img src="assets/sparkline-ruby-bump-silently-skipped.svg" height="14" alt="commit activity" />

Unlike Node (which fails hard), Ruby silently proceeds without bumping any version file. A Ruby project release could end up with the git tag saying v2.0.0 while the gemspec still says 1.9.0. The `version_files` config provides a workaround but this is not documented.

```rust crates/scrat-core/src/bump.rs:282-284
Ecosystem::Ruby => {
    debug!("ruby version bump not yet supported — version lives in gemspec/version.rb");
}
```

**Remediation:** Log at `info!` level. Document that Ruby users should use `version_files` config.

<div>&hairsp;</div>

### Five ecosystem detectors are stubs without tool probing

**advisory** · `crates/scrat-core/src/detect/mod.rs:111-126` · effort: medium · <img src="assets/sparkline-detect-stubs-no-tool-probing.svg" height="14" alt="commit activity" />

The Rust detector probes for `cargo-nextest`, `cargo-set-version`, and `git-cliff` on PATH. The Node/Go/PHP/Python/Ruby/Swift stubs hardcode tool commands without checking whether the binary exists. More critically, all stubs set `changelog_tool: None` even when `detect_version_strategy()` already found git-cliff — this causes the changelog phase to silently skip for non-Rust ecosystems.

```rust crates/scrat-core/src/detect/mod.rs:111-126
/// Stub detection for Node ecosystem (future implementation).
fn detect_node_stub(version_strategy: VersionStrategy) -> ProjectDetection {
    use crate::ecosystem::DetectedTools;

    ProjectDetection {
        ecosystem: Ecosystem::Node,
        version_strategy,
        tools: DetectedTools {
            test_cmd: "npm test".into(),
            build_cmd: "npm run build".into(),
            publish_cmd: Some("npm publish".into()),
            bump_cmd: Some("npm version --no-git-tag-version".into()),
            changelog_tool: None,
        },
    }
}
```

**Remediation:** Wire `git-cliff` detection into `changelog_tool` for all detection paths.

*Verdict: Rust is production-ready. PHP and Python bump works when the version field exists. Go and Swift correctly skip version files. Node is broken. Ruby silently skips. The changelog_tool not being wired through is a real bug that causes silent changelog skips for non-Rust ecosystems.*

<!-- whitespace is important -->
<div>&nbsp;</div>

## The Pipeline Efficiency Surface

*The ship pipeline spawns redundant subprocesses and serializes the pipeline context unconditionally, adding measurable latency to every release.*

### PipelineContext serialized to JSON up to 12 times per ship run

**advisory** · `crates/scrat-core/src/ship.rs:914-920` · effort: trivial · <img src="assets/sparkline-pipeline-context-serialized-unconditionally.svg" height="14" alt="commit activity" />

`run_phase_hooks()` serializes the entire PipelineContext to JSON before calling `hooks::run_hooks()`, even when no `filter:` hooks are present. With 12 hook points and PipelineContext growing as phases complete, this is 12 serialize-and-drop cycles. The JSON is only consumed if a `filter:` hook exists, which is a rare advanced feature.

```rust crates/scrat-core/src/ship.rs:914-920
if !dry_run {
    let pipeline_json =
        serde_json::to_string(pipeline_ctx).map_err(|e| ShipError::PhaseFailed {
            phase,
            message: format!("failed to serialize pipeline context: {e}"),
        })?;
    let output = hooks::run_hooks(cmds, context, project_root, Some(&pipeline_json))?;
```

**Remediation:** Check `cmds.iter().any(|c| c.starts_with("filter:"))` before serializing.

<div>&hairsp;</div>

### Ecosystem detection runs 2-3 times for the same project root

**advisory** · `crates/scrat-core/src/preflight.rs:84` + `crates/scrat-core/src/bump.rs:123` · effort: small · <img src="assets/sparkline-duplicate-detection-calls.svg" height="14" alt="commit activity" />

`resolve_detection()` scans for marker files and probes PATH for binaries. It is called from `run_preflight()`, then again from `plan_bump()` via `plan_ship()`. These are pure functions of `(project_root, config)` — the result cannot change between calls.

**Remediation:** Run detection once in `plan_ship`, pass the result to both `run_preflight` (requires adding a detection parameter) and `plan_bump`.

<div>&hairsp;</div>

### git current_branch() spawns 3 processes for an invariant value

**note** · `crates/scrat-core/src/preflight.rs:173` + `crates/scrat-core/src/ship.rs:420,1045` · effort: small · <img src="assets/sparkline-redundant-git-current-branch.svg" height="14" alt="commit activity" />

Three separate `git rev-parse --abbrev-ref HEAD` spawns for a value that cannot change during the pipeline. `PipelineContextInit` already has a `branch` field — `run_git_phase` could read from the context.

**Remediation:** Cache the branch name in the plan phase and thread it through.

<div>&hairsp;</div>

### Preflight runs git fetch on every ship invocation

**advisory** · `crates/scrat-core/src/git.rs:94-95` · effort: medium · <img src="assets/sparkline-preflight-git-fetch-blocks-startup.svg" height="14" alt="commit activity" />

`git fetch` is a synchronous network round-trip. On slow networks or when GitHub is under load, this dominates startup latency (1-5 seconds). The user experiences a pause before seeing any output.

**Remediation:** Skip if the remote tracking ref was updated recently, or add a `--no-fetch` flag.

*Verdict: None of these affect correctness. They add up to roughly 1-3 seconds of unnecessary work per release. For a tool that runs infrequently, these are polish items, but the git fetch is user-visible latency.*

<!-- whitespace is important -->
<div>&nbsp;</div>

## The Type Design Surface

*The public API is well-structured for its primary consumer (the CLI binary) but has missing derives and one architectural coupling that would hinder library consumers.*

### VersionFileConfig field/fields mutual exclusion not type-enforced

**advisory** · `crates/scrat-core/src/config.rs:238-250` · effort: small · <img src="assets/sparkline-version-file-config-mutual-exclusion.svg" height="14" alt="commit activity" />

Both `field` and `fields` can be `Some` simultaneously. Runtime validation in `version_files.rs:381-385` catches this with a hard `BumpError`, so the contradiction is enforced — but only at runtime, not at the type level.

```rust crates/scrat-core/src/config.rs:238-250
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct VersionFileConfig {
    pub path: String,
    pub format: VersionFileFormat,
    /// Dot-path to the version field (e.g., `"version"`, `"metadata.version"`).
    /// Mutually exclusive with `fields`.
    pub field: Option<String>,
    /// Multiple dot-paths to update in one file.
    /// Mutually exclusive with `field`.
    pub fields: Option<Vec<String>>,
}
```

**Remediation:** Replace with an enum: `VersionFields { Single(String), Multiple(Vec<String>) }`. Use `#[serde(untagged)]` for backward-compatible deserialization.

<div>&hairsp;</div>

### PipelineContextInit is a public type missing Debug derive

**advisory** · `crates/scrat-core/src/pipeline.rs:138-161` · effort: trivial · <img src="assets/sparkline-pipeline-context-init-missing-debug.svg" height="14" alt="commit activity" />

A public 11-field struct with zero derive macros. All fields are `String`, `Option<String>`, or `bool` — all derivable. Per Rust API guidelines, `Debug` is essential for any public type.

**Remediation:** Add `#[derive(Debug, Clone)]`.

<div>&hairsp;</div>

### scrat-core depends on clap despite being a library crate

**advisory** · `crates/scrat-core/Cargo.toml:33` · effort: small · <img src="assets/sparkline-clap-in-core-crate.svg" height="14" alt="commit activity" />

scrat-core pulls in `clap` (including `clap_derive` proc macro) solely to derive `ValueEnum` on two enums in `init.rs` (`ConfigFormat` and `ConfigStyle`). The binary already depends on clap, so compile-time cost is paid once — but the coupling makes scrat-core unusable without clap.

**Remediation:** Move the two enums to the CLI crate, or define them in core without `ValueEnum` and implement the clap mapping via `From`.

<div>&hairsp;</div>

### scrat-core lib.rs does not re-export key types for library consumers

**advisory** · `crates/scrat-core/src/lib.rs:72-78` · effort: small · <img src="assets/sparkline-scrat-core-lib-incomplete-reexports.svg" height="14" alt="commit activity" />

`lib.rs` re-exports only `Config`, `ConfigLoader`, `LogLevel`, `ConfigError`, `ConfigResult`, and `semver`. Key workflow types like `ShipOptions`, `Ecosystem`, `ProjectDetection`, and `PipelineContext` have no top-level re-exports.

**Remediation:** Add re-exports for commonly used types, or add a `pub mod prelude`.

<div>&hairsp;</div>

### Pipeline types lack PartialEq for testing by library consumers

**note** · `crates/scrat-core/src/pipeline.rs:28-29` · effort: trivial · <img src="assets/sparkline-pipeline-types-missing-eq-hash.svg" height="14" alt="commit activity" />

`PipelineContext`, `DepChange`, `ReleaseStats`, and `Contributor` derive `Debug, Clone, Serialize, Deserialize` but not `PartialEq` or `Eq`. Library consumers cannot use `assert_eq!` on these types.

**Remediation:** Add `PartialEq, Eq` to the derive list.

*Verdict: The type system does its job — enums with match dispatch, typed errors, plan/execute pattern. The gaps are all polish: missing derives, a library depending on clap for two derives, representable-but-invalid config states. None affect the CLI user; all affect anyone embedding scrat-core as a library.*

<!-- whitespace is important -->
<div>&nbsp;</div>

## The Error Architecture Surface

*The error handling architecture is textbook Rust — typed errors, thiserror, consistent `?` propagation — with one `expect()` in library code that should be a `Result`.*

### expect() on temp file path UTF-8 conversion in library code

**moderate** · `crates/scrat-core/src/version/conventional.rs:53-57` · effort: trivial · <img src="assets/sparkline-conventional-version-expect-temppath.svg" height="14" alt="commit activity" />

`compute_via_cliff` returns `Result<Version, VersionError>` but uses `expect()` for the temp path conversion. On systems with non-UTF-8 temp directories (rare but possible on some Linux configurations), this panics in library code instead of returning an error.

```rust crates/scrat-core/src/version/conventional.rs:53-57
tmp_file
    .path()
    .to_str()
    .expect("temp path is UTF-8")
    .to_string()
```

**Remediation:** Replace with `.to_str().ok_or_else(|| VersionError::ToolFailed { ... })?`.

<div>&hairsp;</div>

### ProgressStyle::with_template().unwrap() inconsistent with project pattern

**note** · `crates/scrat/src/commands/ship.rs:251-253` · effort: trivial · <img src="assets/sparkline-cli-spinner-template-unwrap.svg" height="14" alt="commit activity" />

Bare `.unwrap()` on a compile-time-constant template. Safe in practice, but `doctor.rs` uses `.expect("valid template")` for the same pattern. This is the only `unwrap()` in the binary crate.

**Remediation:** Replace `.unwrap()` with `.expect("valid spinner template")`.

*Verdict: The error architecture is the strongest aspect of this codebase. Seven thiserror enums compose cleanly via `#[from]`. Zero `unwrap()` in production library code. The single `expect()` on a temp path is the only blemish. The error hierarchy is ready for library consumers without modification.*

---

## Remediation Ledger

| Finding | Concern | Location | Effort | Chains |
|---------|---------|----------|--------|--------|
| | | **Shell Execution Boundary** | | |
| [hooks-filter-truncate-panic](#byte-index-truncation-of-filter-output-panics-on-multi-byte-utf-8) | significant | `hooks.rs:393-394` | trivial | — |
| [hooks-stdin-write-silently-discarded](#stdin-write-error-silently-discarded-in-filter-hook) | moderate | `hooks.rs:368-371` | small | related: hooks-filter-truncate-panic |
| [git-fetch-silently-discarded](#git-fetch-failure-silently-discarded-in-remote-sync-check) | moderate | `git.rs:94-95` | trivial | — |
| | | **Ecosystem Completeness** | | |
| [node-bump-hard-error](#node-ecosystem-bump-returns-hard-error-blocking-scrat-ship) | significant | `bump.rs:262-264` | small | enables: node-deps-parser-stub |
| [node-deps-parser-stub](#node-dependency-diff-parser-is-a-no-op-stub) | advisory | `deps.rs:165-170` | medium | enabled by: node-bump-hard-error |
| [ruby-bump-silently-skipped](#ruby-version-bump-silently-skipped-with-debug-only-message) | advisory | `bump.rs:282-284` | small | — |
| [detect-stubs-no-tool-probing](#five-ecosystem-detectors-are-stubs-without-tool-probing) | advisory | `detect/mod.rs:111-126` | medium | — |
| | | **Pipeline Efficiency** | | |
| [pipeline-context-serialized-unconditionally](#pipelinecontext-serialized-to-json-up-to-12-times-per-ship-run) | advisory | `ship.rs:914-920` | trivial | — |
| [duplicate-detection-calls](#ecosystem-detection-runs-2-3-times-for-the-same-project-root) | advisory | `preflight.rs:84` | small | related: redundant-git-current-branch |
| [redundant-git-current-branch](#git-current_branch-spawns-3-processes-for-an-invariant-value) | note | `preflight.rs:173` | small | related: duplicate-detection-calls |
| [preflight-git-fetch-blocks-startup](#preflight-runs-git-fetch-on-every-ship-invocation) | advisory | `git.rs:94-95` | medium | related: git-fetch-silently-discarded |
| | | **Type Design** | | |
| [version-file-config-mutual-exclusion](#versionfileconfig-fieldfields-mutual-exclusion-not-type-enforced) | advisory | `config.rs:238-250` | small | — |
| [pipeline-context-init-missing-debug](#pipelinecontextinit-is-a-public-type-missing-debug-derive) | advisory | `pipeline.rs:138-161` | trivial | related: pipeline-types-missing-eq-hash |
| [clap-in-core-crate](#scrat-core-depends-on-clap-despite-being-a-library-crate) | advisory | `Cargo.toml:33` | small | — |
| [scrat-core-lib-incomplete-reexports](#scrat-core-librs-does-not-re-export-key-types-for-library-consumers) | advisory | `lib.rs:72-78` | small | — |
| [pipeline-types-missing-eq-hash](#pipeline-types-lack-partialeq-for-testing-by-library-consumers) | note | `pipeline.rs:28-29` | trivial | related: pipeline-context-init-missing-debug |
| | | **Error Architecture** | | |
| [conventional-version-expect-temppath](#expect-on-temp-file-path-utf-8-conversion-in-library-code) | moderate | `conventional.rs:53-57` | trivial | — |
| [cli-spinner-template-unwrap](#progressstylewith_templateunwrap-inconsistent-with-project-pattern) | note | `ship.rs:251-253` | trivial | — |

---

<sub>
Generated 2026-04-09 at commit f706dc9.
Intermediate artifacts: recon.yaml, findings.yaml.
Tools: clippy (pass), cargo audit (pass), cargo deny (warnings: 3 duplicate crates), cargo machete (pass), cargo udeps (pass).
Domain rubric: crustoleum (14 surfaces, 89 criteria). Agents: api-type-design, error-robustness, supply-chain-deps, performance, completeness + reviewer.
</sub>
