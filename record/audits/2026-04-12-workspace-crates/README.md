---
audit_date: 2026-04-12
project: scrat
commit: 7b2f302a165bac8bd114ddee283d5d396da42e89
scope: Full workspace audit — crates/scrat (CLI) and crates/scrat-core (library)
auditor: Claude Opus 4.6 (1M context) via cased + crustoleum
findings:
  critical: 0
  significant: 1
  moderate: 3
  advisory: 9
  note: 6
---

# Audit: scrat workspace crates

scrat is a two-month-old Rust release-automation CLI, just shy of 18.4k lines
across two workspace crates, and the Phase 4 ecosystem-modules refactor landed
yesterday. The foundation is unusually tight for an early-stage tool: both
crates declare `#![deny(unsafe_code)]`, clippy is clean with `-D warnings`,
cargo-audit is clean across 189 dependencies, and the canonical
ownership/lifetime/trait-design surface holds against crustoleum's rubric
without a single finding. **The Error Architecture Surface** has one
load-bearing gap — two thiserror enums flatten sub-error context into
`String` and drop the `source()` chain at the user seam. **The Supply Chain
Surface** is safe today but needs hardening: a `0.0` caret on serde-saphyr
allows breaking-per-patch resolution, CI doesn't fail on yanked or
unmaintained crates, and scrat-core drags clap into every library consumer
for two `ValueEnum` derives that the CLI should own. **The Pipeline
Efficiency Surface** is mostly cleaned up since the April 9 audit — what
remains is a redundant `git rev-parse`, uncached PATH probes, and a release
profile missing trivial LTO/strip wins. **The Feature Completeness Surface**
is the one that draws blood: `scrat notes --from` doesn't actually affect the
rendered notes, the same command silently skips every hook the ship path
runs, and three documented config knobs have no implementation behind them.
The honest single takeaway: the release pipeline is production-ready for
Rust, but the scrat notes command and three orphan config fields each
contradict their own documentation in ways that would blindside a user the
first time they tried the feature.

## Findings by surface

### The Error Architecture Surface

- [`notes-error-flattens-source-chain`](#notes-error-flattens-source-chain) (moderate) — `crates/scrat-core/src/notes.rs:22-41`
- [`observability-returns-anyhow-in-library`](#observability-returns-anyhow-in-library) (moderate) — `crates/scrat-core/src/observability.rs:67-70`
- [`observability-writer-silent-discard`](#observability-writer-silent-discard) (advisory) — `crates/scrat-core/src/observability.rs:196-202`
- [`expect-messages-describe-value-not-invariant`](#expect-messages-describe-value-not-invariant) (advisory) — `crates/scrat/src/commands/doctor.rs:112-116`

### The Supply Chain Surface

- [`scrat-core-depends-on-clap-for-value-enum-derives`](#scrat-core-depends-on-clap-for-value-enum-derives) (moderate) — `crates/scrat-core/Cargo.toml:33`
- [`serde-saphyr-caret-on-zero-zero-x`](#serde-saphyr-caret-on-zero-zero-x) (advisory) — `crates/scrat-core/Cargo.toml:36`
- [`ci-lacks-yanked-and-unmaintained-hardening`](#ci-lacks-yanked-and-unmaintained-hardening) (advisory) — `.config/deny.toml:48-55`
- [`owo-colors-pulls-duplicate-supports-color`](#owo-colors-pulls-duplicate-supports-color) (note) — `crates/scrat/Cargo.toml:49`
- [`transitive-getrandom-triplicate`](#transitive-getrandom-triplicate) (note) — `Cargo.lock:477-513`

### The Pipeline Efficiency Surface

- [`redundant-git-current-branch-per-ship`](#redundant-git-current-branch-per-ship) (advisory) — `crates/scrat-core/src/preflight.rs:188-198`
- [`release-profile-missing-lto-and-strip`](#release-profile-missing-lto-and-strip) (advisory) — `Cargo.toml:57-59`
- [`has-binary-path-probe-not-cached`](#has-binary-path-probe-not-cached) (note) — `crates/scrat-core/src/detect.rs:131-134`

### The Feature Completeness Surface

- [`notes-from-flag-ignored-by-cliff-context`](#notes-from-flag-ignored-by-cliff-context) (significant) — `crates/scrat-core/src/notes.rs:347-371`
- [`notes-command-skips-all-hooks`](#notes-command-skips-all-hooks) (advisory) — `crates/scrat-core/src/notes.rs:158-175`
- [`orphan-commands-build-and-clean-config`](#orphan-commands-build-and-clean-config) (advisory) — `crates/scrat-core/src/config.rs:105-115`
- [`orphan-release-changelog-tool-config`](#orphan-release-changelog-tool-config) (advisory) — `crates/scrat-core/src/config.rs:117-121`
- [`example-config-advertises-unimplemented-otel-and-env`](#example-config-advertises-unimplemented-otel-and-env) (note) — `config/scrat.toml.example:26-36`
- [`bump-error-unsupported-ecosystem-dead-variant`](#bump-error-unsupported-ecosystem-dead-variant) (note) — `crates/scrat-core/src/bump.rs:43-49`
- [`example-config-missing-filter-prefix-docs`](#example-config-missing-filter-prefix-docs) (note) — `config/scrat.toml.example:82-92`

---

## The Error Architecture Surface

*Typed errors with thiserror, zero `unwrap()` in library code, consistent
`?` propagation — the spine is sound. The gaps are at the edges: two enums
flatten sub-error context into Strings, one library module leaks
`anyhow::Error`, and one logging discard is missing its intent comment.*

### notes-error-flattens-source-chain

**moderate** · `crates/scrat-core/src/notes.rs:22-41` · effort: medium

The CLI's rendering path does the work to show error source chains — `main.rs`
has a careful `err.chain().skip(1).for_each(|cause| eprintln!(...))` loop
designed to surface why a failure happened, not just what. That machinery
runs dry when the library hands it a `NotesError::CliffContext(String)` or
`NotesError::CliffRender(String)`, because the `String` variant breaks the
`Error::source()` chain. Both variants take a `String` at construction and
six call sites in the same file shove heterogeneous sub-errors —
`std::io::Error`, `serde_json::Error`, `semver::Error`, `GitError` — through
`format!("... {e}")` before they get there. The same anti-pattern appears in
`BumpError::ToolFailed` with ~20 call sites across `version_files.rs` and
`bump.rs`. Net effect: users see one line where they should see a chain, and
have to re-run with `-vv` to figure out what actually broke.

```rust crates/scrat-core/src/notes.rs:22-41
/// Errors from the release notes rendering pipeline.
#[derive(Error, Debug)]
pub enum NotesError {
    /// Failed to run `git-cliff --context` or parse its output.
    #[error("git-cliff context extraction failed: {0}")]
    CliffContext(String),

    /// Failed to run `git-cliff --from-context` to render notes.
    #[error("git-cliff rendering failed: {0}")]
    CliffRender(String),

    /// Failed to read a custom template file.
    #[error("failed to read template at {path}: {source}")]
    ReadTemplate {
        /// Path to the template file.
        path: String,
        /// The underlying I/O error.
        source: std::io::Error,
    },
}
```

`ReadTemplate` in the same enum shows the pattern that works: a real
`#[source]` field preserves the chain. The two `String` variants can follow
suit.

**Remediation:** Introduce sub-variants that carry the real source via
`#[from]`:

```rust
#[error("git-cliff exec failed: {0}")]
CliffExec(#[from] std::io::Error),

#[error("git-cliff JSON parse failed: {0}")]
CliffJson(#[from] serde_json::Error),
```

Keep a `String` variant only for the legitimate "tool returned non-zero with
stderr" case where no underlying Rust error exists. Apply the same treatment
to `BumpError::ToolFailed` — introduce `ToolIo`, `ToolParse`, `ToolSerialize`
alongside the existing `ToolFailed(String)`. Most call sites drop their
`format!` wrapper and rely on `#[from]`. Phase `NotesError` first (smaller
blast radius); `BumpError` second.

<div>&hairsp;</div>

### observability-returns-anyhow-in-library

**moderate** · `crates/scrat-core/src/observability.rs:67-70` · effort: small

scrat-core is the library half of a "thin CLI, fat core" architecture, and
every other module exposes typed errors via thiserror enums — `BumpError`,
`ShipError`, `GitError`, `HookError`, `VersionError`, `ConfigError`,
`NotesError`. `observability` is the sole exception. `use anyhow::Result;`
at the top of the file feeds through to the public `init_observability`
signature, which returns `Result<ObservabilityGuard>` — effectively
`Result<_, anyhow::Error>`. Downstream consumers of scrat-core can't pattern
match on the failure cause because `anyhow::Error` is a `Box<dyn Error>` in
disguise. The function already swallows its own failure internally (the
stderr fallback at lines 71-80 returns `Ok` regardless), so the `Result`
return type is both unnecessary and type-erasing.

```rust crates/scrat-core/src/observability.rs:67-70
pub fn init_observability(
    cfg: &ObservabilityConfig,
    env_filter: EnvFilter,
) -> Result<ObservabilityGuard> {
```

The typed errors that `observability` needs already exist in the module —
look at `resolve_log_target`, `log_target_from_dir`, `log_target_from_path`,
`ensure_writable`. They return `Result<_, String>` internally. Stringifying
them was a half step; publishing them as an enum is the rest of the trip.

**Remediation:** Define `ObservabilityError` with thiserror in the same file.
Change the signature to
`pub fn init_observability(...) -> Result<ObservabilityGuard, ObservabilityError>`.
The CLI's `main.rs` already converts library errors via `?` + `.context(...)`,
so the binary surface doesn't change.

<div>&hairsp;</div>

### observability-writer-silent-discard

**advisory** · `crates/scrat-core/src/observability.rs:196-202` · effort: trivial

The project has deliberately converted similar discards elsewhere. `git.rs`
used to have a bare `let _ = git(&["fetch", "--quiet"])`; it now reads
`if let Err(e) = git(...) { debug!(error = %e, ...) }` with a comment
explaining that fetch failure is intentionally non-fatal. The
`JsonLogLayer` in observability is the one surviving case where a discard is
unexplained, and it's in a place where the intent is actually *stronger*
than git-fetch — a logging layer must not panic or error back through the
program. The code is correct; it just doesn't say so.

```rust crates/scrat-core/src/observability.rs:196-202
        // Buffer the entire line so it's written in a single write() syscall,
        // which is atomic with O_APPEND for lines under PIPE_BUF (typically 4096).
        if let Ok(mut buf) = serde_json::to_vec(&Value::Object(map)) {
            buf.push(b'\n');
            let mut writer = self.writer.make_writer();
            let _ = writer.write_all(&buf);
        }
```

**Remediation:** Add a comment above the discard explaining the intent. If
diagnostic value matters, promote the discard to an `AtomicUsize` "dropped
log lines" counter that `scrat doctor` surfaces. Both are trivial.

<div>&hairsp;</div>

### expect-messages-describe-value-not-invariant

**advisory** · `crates/scrat/src/commands/doctor.rs:112-116` · effort: trivial

The project's rule against panics in library code holds — this finding is
about message quality, not about whether the panic should exist. Two
indicatif `ProgressStyle::template` calls unwrap with `expect("valid
template")` (doctor.rs) and `expect("valid spinner template")` (ship.rs).
Both messages paraphrase the value when they could state the invariant and
carry the literal:

```rust crates/scrat/src/commands/doctor.rs:112-116
    spinner.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.cyan} {msg}")
            .expect("valid template"),
    );
```

A future maintainer reading a panic message in a user's crash report learns
nothing from "valid template." They'd learn everything from the literal.

**Remediation:** Replace both with a message that carries the literal and the
invariant:
`.expect("indicatif must accept literal template '{spinner:.cyan} {msg}'")`.

*Verdict: the spine of the error architecture is the codebase's strongest
surface. The four findings are all localized and mechanical; together they
would close the last daylight at the user seam.*

<div>&nbsp;</div>

## The Supply Chain Surface

*cargo-audit is clean on 189 deps, cargo-deny is clean on
advisories/licenses/sources, cargo-machete finds zero unused deps. The
remaining risk lives in two places: a `0.0` caret on serde-saphyr that
allows breaking-per-patch resolution, and a library crate pulling clap just
for two `ValueEnum` derives the CLI could own. CI gates don't flag yanked
or unmaintained crates.*

### scrat-core-depends-on-clap-for-value-enum-derives

**moderate** · `crates/scrat-core/Cargo.toml:33` · effort: small

scrat-core is published as a library (`description = "Core library for
scrat"`) and re-exported via the CLI crate. Its only use of clap is two
`#[derive(..., clap::ValueEnum)]` attributes in `init.rs` — `ConfigFormat`
and `ConfigStyle`. The cost of those two derives is clap_builder + clap_derive
(a proc-macro) + anstyle + strsim + ~15 more transitive crates leaking into
every downstream consumer of scrat-core, and a clap 4.x major-version lock
that downstreams can't override.

```toml crates/scrat-core/Cargo.toml:33
clap = { version = "4.6", features = ["derive"] }
```

The workspace's own rule — "thin CLI, fat core" with CLI parsing in the
binary — gets violated by the library crate itself. This is the one dep in
scrat-core that doesn't pay for its weight.

**Remediation:** Move the derives to the CLI. Define mirror enums in
`crates/scrat/src/commands/init.rs` that derive `clap::ValueEnum` and
provide `From<CliConfigFormat> for scrat_core::ConfigFormat`. Drop clap from
`crates/scrat-core/Cargo.toml`. Alternatively, feature-gate the library's
clap derives behind a `cli` feature — opt-in coupling, opt-out cost.

<div>&hairsp;</div>

### serde-saphyr-caret-on-zero-zero-x

**advisory** · `crates/scrat-core/Cargo.toml:36` · effort: trivial

scrat-core depends on `serde-saphyr = "0.0"` — a caret requirement that
resolves to any 0.0.x version. Semver convention treats every 0.0.x patch
as potentially breaking, so a future `serde-saphyr 0.0.24` with breaking
API changes still satisfies the requirement. scrat's own Cargo.lock holds
the current version at 0.0.23, but `cargo install scrat` bypasses the
lockfile and a downstream library consumer that takes `scrat-core` will get
whatever 0.0.x resolves at their `cargo update`.

```toml crates/scrat-core/Cargo.toml:36
serde-saphyr = { version = "0.0", features = ["figment"] }
```

> A supply-chain adversary doesn't need to poison serde-saphyr — they just
> need to wait until the maintainer publishes a 0.0.x with a breaking
> change, and observe which consumers' builds fall over. The caret-on-0.0
> pattern advertises "I don't care what you publish here."

**Remediation:** Pin to a tilde or exact version while serde-saphyr is in
the 0.0.x series:

```toml
serde-saphyr = { version = "~0.0.23", features = ["figment"] }
```

Update both `crates/scrat-core/Cargo.toml` and `crates/scrat/Cargo.toml`.
Bump explicitly when a new 0.0.x lands, ideally through a workspace version
constant so the two entries stay aligned.

<div>&hairsp;</div>

### ci-lacks-yanked-and-unmaintained-hardening

**advisory** · `.config/deny.toml:48-55` · effort: trivial

CI runs `cargo deny check` and has no dedicated `cargo audit` step.
cargo-deny's defaults for yanked and unmaintained advisories are `warn`,
not `deny`, so a post-merge yank of a pinned dep — or a RustSec
`unmaintained` flag on a direct dep like serde-saphyr or directories —
appears as a warning and does not fail CI.

```toml .config/deny.toml:48-55
[advisories]
# Always check the RustSec advisory database
db-path = "~/.cargo/advisory-db"
db-urls = ["https://github.com/RustSec/advisory-db"]

# Ignore specific advisories (use with caution in enterprise)
ignore = [
]
```

> The interesting yank isn't the malicious one — it's the legitimate one
> that the maintainer does at 2 AM on a Saturday because they spotted an
> integer overflow. If CI doesn't fail on yanked, scrat's main branch keeps
> shipping the yanked artifact until someone happens to read a CI log.

**Remediation:** Add three gates to `[advisories]`:

```toml
yanked = "deny"
unmaintained = "deny"
vulnerability = "deny"
```

Or add a dedicated `cargo audit --deny warnings` step to `ci.yml` so
RustSec signaling runs through two independent checks.

<div>&hairsp;</div>

### owo-colors-pulls-duplicate-supports-color

**note** · `crates/scrat/Cargo.toml:49` · effort: trivial

cargo-deny reports `supports-color` as a 2-version duplicate. The root
cause is upstream: owo-colors 4.3.0's `supports-colors` feature pulls both
`supports-color 3.0.0` and (via package rename) `supports-color-2 = "2.0"`
intentionally. scrat can't dedupe it without losing terminal-capability
auto-detect. The duplicate is harmless; it just drowns real signal in
every cargo-deny and cargo-tree report.

```toml crates/scrat/Cargo.toml:49
owo-colors = { version = "4.3", features = ["supports-colors"] }
```

**Remediation:** Add a documented skip entry to `.config/deny.toml`:

```toml
[bans]
skip = [
  { name = "supports-color" },  # upstream owo-colors 4.3 dual-dep
]
```

That silences the warning without hiding anything consequential.

<div>&hairsp;</div>

### transitive-getrandom-triplicate

**note** · `Cargo.lock:477-513` · effort: small

Three `getrandom` majors coexist in the lockfile via disjoint transitive
chains: 0.2.17 rides in through `directories → dirs-sys → redox_users`
(Redox-cfg gated, not loaded on macOS/Linux/Windows at runtime), 0.3.4
through `serde-saphyr → ahash`, and 0.4.2 through `tempfile`. The
r-efi 5.3.0 and 6.0.0 duplicates ride along. Nothing in scrat is the
cause — it's three independent upstream version requirements that can't be
satisfied by a single minor.

```toml Cargo.lock:477-513
[[package]]
name = "getrandom"
version = "0.2.17"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "ff2abc00be7fca6ebc474524697ae276ad847ad0a6b3faa4bcb027e9a4614ad0"
dependencies = [
 "cfg-if",
 "libc",
 "wasi",
]

[[package]]
name = "getrandom"
version = "0.3.4"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "899def5c37c4fd7b2664648c28120ecec138e4d395b459e5ca34f9cce2dd77fd"
dependencies = [
 "cfg-if",
 "js-sys",
 "libc",
 "r-efi 5.3.0",
 "wasip2",
 "wasm-bindgen",
]

[[package]]
name = "getrandom"
version = "0.4.2"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "0de51e6874e94e7bf76d726fc5d13ba782deca734ff60d5bb2fb2607c7406555"
dependencies = [
 "cfg-if",
 "libc",
 "r-efi 6.0.0",
 "wasip2",
 "wasip3",
]
```

Related: [`owo-colors-pulls-duplicate-supports-color`](#owo-colors-pulls-duplicate-supports-color).

**Remediation:** Two paths, both advisory. Wait for upstream alignment —
ahash 0.9+ and tempfile 3.x-next may converge on getrandom 0.4 once it
stabilizes. Or replace the three `directories::ProjectDirs::from` call sites
in `config.rs` and `observability.rs` with a hand-rolled XDG lookup reading
`XDG_CONFIG_HOME` / `XDG_DATA_HOME` / `HOME` directly, dropping the
`directories → dirs-sys → redox_users → getrandom 0.2` chain entirely
(~6 transitive crates saved on Unix). Low priority for a macOS-first tool.

*Verdict: the current footprint is safe today. Hardening is trivial — three
lines in deny.toml, two Cargo.toml edits, and roughly 30 lines to evict
clap from scrat-core. The transitive-duplicate warnings are upstream-owned
and should be skipped in deny.toml so they stop drowning the real signal.*

<div>&nbsp;</div>

## The Pipeline Efficiency Surface

*The headline wins from the April 9 audit are already merged — detection
runs once in `plan_ship`, `PipelineContext` serializes only when a `filter:`
hook exists. What remains: one redundant `git rev-parse` per ship, an
uncached `which::which` loop across tool probing, and a release profile
that doesn't pull the trivial LTO/strip levers.*

### redundant-git-current-branch-per-ship

**advisory** · `crates/scrat-core/src/preflight.rs:188-198` · effort: small

A ship run resolves the current branch in preflight's
`check_release_branch` and resolves it again when building the
`PipelineContext` in `ReadyShip::execute()` (ship.rs:448). Each call is a
`git rev-parse --abbrev-ref HEAD` fork+exec, ~5-15ms on macOS. The branch
cannot change between plan and execute within the same process, so the
second call is pure waste. The Phase 4 refactor deduplicated detection but
didn't thread branch through.

```rust crates/scrat-core/src/preflight.rs:188-198
fn check_release_branch(override_branch: Option<&str>) -> CheckResult {
    let current = match git::current_branch() {
        Ok(Some(b)) => b,
        Ok(None) => {
            return CheckResult {
                name: "Release branch".into(),
                passed: false,
                message: "Detached HEAD — not on any branch".into(),
                skip_flag: None,
            };
        }
```

Related: [`has-binary-path-probe-not-cached`](#has-binary-path-probe-not-cached).

**Remediation:** Stash the branch result on `ReadyShip` — populate once in
`plan_ship` by reading `report.checks` or calling `current_branch` directly
there, then read `self.branch` in `execute()`. Alternatively, add
`ProjectDetection.branch: Option<String>` if other callers want it.

<div>&hairsp;</div>

### release-profile-missing-lto-and-strip

**advisory** · `Cargo.toml:57-59` · effort: trivial

The release profile only sets `debug = "line-tables-only"`. No LTO, no
strip, default codegen-units. For a user-facing CLI invoked interactively
(startup latency matters) and distributed via crates.io (binary size
matters), this leaves easy wins on the table. Thin LTO, codegen-units = 1,
and strip = "symbols" typically give 10-20% smaller output and measurable
startup wins — especially meaningful for scrat given the thin-CLI
architecture, where every command does `scrat → scrat_core::*` and
cross-crate inlining is the main win.

```toml Cargo.toml:57-59
# Release profile with reduced debug info
[profile.release]
debug = "line-tables-only"
```

**Remediation:** Three lines:

```toml
[profile.release]
debug = "line-tables-only"
lto = "thin"
codegen-units = 1
strip = "symbols"
```

Benchmark with `hyperfine --warmup 3 'scrat --version'` before and after.
If release build time becomes noticeable, drop `codegen-units = 1` — thin
LTO alone captures most of the benefit.

<div>&hairsp;</div>

### has-binary-path-probe-not-cached

**note** · `crates/scrat-core/src/detect.rs:131-134` · effort: small

A single `plan_ship` invokes `has_binary` for `git-cliff`, `cargo`,
`cargo-nextest`, `cargo-set-version` during detection, then re-probes the
same set in `check_required_tools` and adds `gh` for `check_gh_auth`.
Each probe is ~1-2ms of PATH walking on a normal macOS box; they aggregate
to ~10-15ms per ship. Binaries can't move during a single process, so the
probing is idempotent.

```rust crates/scrat-core/src/detect.rs:131-134
/// Check whether a binary is available on `PATH`.
pub fn has_binary(name: &str) -> bool {
    which::which(name).is_ok()
}
```

Related: [`redundant-git-current-branch-per-ship`](#redundant-git-current-branch-per-ship).

**Remediation:** Wrap `has_binary` with a process-lifetime cache:

```rust
use std::sync::{Mutex, OnceLock};
use std::collections::HashMap;

static PATH_CACHE: OnceLock<Mutex<HashMap<String, bool>>> = OnceLock::new();

pub fn has_binary(name: &str) -> bool {
    let cache = PATH_CACHE.get_or_init(Default::default);
    if let Some(&v) = cache.lock().unwrap().get(name) {
        return v;
    }
    let v = which::which(name).is_ok();
    cache.lock().unwrap().insert(name.to_owned(), v);
    v
}
```

Add a `#[cfg(test)] fn clear_path_cache()` if any test needs to pick up a
binary installed mid-run. Single-digit-ms win on ship, meaningful on
`scrat info` which runs detection twice through the same paths for display.

*Verdict: nothing here affects correctness. Together these findings add up
to tens of milliseconds on a ship run plus one-time binary-size savings.
The release-profile tweak is the most user-visible — every installed binary
ships smaller and starts faster, with no code changes.*

<div>&nbsp;</div>

## The Feature Completeness Surface

*Seven ecosystems now ship real bump, deps, and detect behavior — the Node
and Ruby gaps from the April 9 audit are closed. The remaining completeness
debt has moved into `scrat notes` and the config surface: one documented
flag does nothing, the standalone notes command diverges from the ship
artifact, and three documented config knobs have no implementation.*

### notes-from-flag-ignored-by-cliff-context

**significant** · `crates/scrat-core/src/notes.rs:347-371` · effort: small

README documents `scrat notes --from v1.0.0` as "diff against specific
tag" and the CLI help declares `--from` as the "Previous version tag to
diff against." The implementation stashes `options.from` in `previous_tag`
for stats and dep-diff computation — but the git-cliff invocation that
produces the actual notes body always passes `--unreleased --context` with
no range selector. For any already-tagged release the call returns an empty
context and `preview_notes` fails with "git-cliff produced empty context
output." Historical release notes cannot be regenerated.

```rust crates/scrat-core/src/notes.rs:347-371
/// Run `git-cliff --unreleased --context` and capture JSON output.
fn run_cliff_context(project_root: &Utf8Path) -> Result<String, NotesError> {
    let output = Command::new("git-cliff")
        .args(["--unreleased", "--context"])
        .current_dir(project_root.as_std_path())
        .output()
        .map_err(|e| NotesError::CliffContext(format!("failed to execute git-cliff: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(NotesError::CliffContext(format!(
            "git-cliff exited with {}: {stderr}",
            output.status
        )));
    }

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    if stdout.trim().is_empty() {
        return Err(NotesError::CliffContext(
            "git-cliff produced empty context output".into(),
        ));
    }

    Ok(stdout)
}
```

The `--from` flag influences two side channels (stats, deps) but not the
primary artifact the command is named after. This is the gap already
recorded as "notes CLI can't regenerate past releases" in project memory;
it's been live since the command shipped.

Related: [`notes-command-skips-all-hooks`](#notes-command-skips-all-hooks).

**Remediation:** Forward the resolved `previous_tag` to git-cliff as a
tag or range selector — either `--tag <new-tag>` paired with
`<from>..HEAD`, or a per-release `--tag-pattern`. Fall back to
`--unreleased` only when no explicit `--from` is provided and no prior tag
exists.

<div>&hairsp;</div>

### notes-command-skips-all-hooks

**advisory** · `crates/scrat-core/src/notes.rs:158-175` · effort: small

README frames `scrat notes` as a low-friction preview of what ship will
produce: "Renders release notes without shipping. Useful for previewing
what the notes will look like." When `scrat ship` runs, `filter:` hooks
(and any post-bump hook that mutates metadata) rewrite the
`PipelineContext` that feeds git-cliff. `preview_notes` never calls
`hooks::run_hooks`, so users who rely on filter hooks to shape release
notes — the whole point of the `filter:` prefix — see a preview that
diverges from the artifact ship produces.

```rust crates/scrat-core/src/notes.rs:158-175
    // Compute stats
    if !options.no_stats && !previous_tag.is_empty() {
        ctx.stats = stats::compute_stats(&ctx.previous_tag);
        if ctx.stats.is_some() {
            debug!("stats computed");
        }
    }

    // Determine template: options > config > built-in
    let template = options.template.as_deref().or_else(|| {
        config
            .release
            .as_ref()
            .and_then(|r| r.notes_template.as_deref())
    });

    // Render
    let notes = render_notes(project_root, &ctx, template)?;
```

Enabled by [`notes-from-flag-ignored-by-cliff-context`](#notes-from-flag-ignored-by-cliff-context).

**Remediation:** Thread `config.hooks` through `preview_notes` and execute
the applicable `filter:` and post-bump-phase hooks against the preview
context before calling `render_notes`. Alternatively, document `scrat
notes` as a pre-hook preview and direct users to `scrat ship --dry-run`
for hook-aware previews — but the "matches what ship will produce"
framing in the README is the more valuable promise to keep.

<div>&hairsp;</div>

### orphan-commands-build-and-clean-config

**advisory** · `crates/scrat-core/src/config.rs:105-115` · effort: trivial

README's Configuration reference and `config/scrat.toml.example` (lines
58-62) advertise `commands.build` and `commands.clean` as override knobs.
A repo-wide grep for every reference to these fields in the core crate
finds them only in the struct definition and a single deserialization
round-trip test. No phase reads them, and scrat has no `build` or `clean`
subcommand.

```rust crates/scrat-core/src/config.rs:105-115
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct CommandsConfig {
    /// Override the test command (e.g., `"cargo nextest run"`).
    pub test: Option<String>,
    /// Override the build command (e.g., `"cargo build --release"`).
    pub build: Option<String>,
    /// Override the publish command (e.g., `"cargo publish"`).
    pub publish: Option<String>,
    /// Override the clean command.
    pub clean: Option<String>,
}
```

Related: [`orphan-release-changelog-tool-config`](#orphan-release-changelog-tool-config),
[`example-config-advertises-unimplemented-otel-and-env`](#example-config-advertises-unimplemented-otel-and-env).

**Remediation:** Delete both fields along with their example-config entries
and README mentions. Or wire them in (auto-invoke `commands.build` before
the publish phase if set, for instance). Given no `build`/`clean` phase is
on the stated roadmap, deletion is the honest fix.

<div>&hairsp;</div>

### orphan-release-changelog-tool-config

**advisory** · `crates/scrat-core/src/config.rs:117-121` · effort: trivial

`config/scrat.toml.example:69` advertises
`changelog_tool = "git-cliff"` as a release-workflow knob. In practice
scrat derives the changelog tool from `VersionStrategy` inside each
driver's `detect()` and production reads consult
`detection.tools.changelog_tool`. No code path feeds a user-provided
`release.changelog_tool` back into detection or pipeline flow.

```rust crates/scrat-core/src/config.rs:117-121
/// Release workflow configuration.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct ReleaseConfig {
    /// Override the changelog tool (currently only `"git-cliff"` is supported).
    pub changelog_tool: Option<ChangelogTool>,
```

Related: [`orphan-commands-build-and-clean-config`](#orphan-commands-build-and-clean-config).

**Remediation:** Honor the override (propagate it into
`DetectedTools.changelog_tool` when set), or remove the field and its
example-config entry.

<div>&hairsp;</div>

### example-config-advertises-unimplemented-otel-and-env

**note** · `config/scrat.toml.example:26-36` · effort: trivial

The canonical example config advertises two integration points that don't
exist in source: `SCRAT_ENV` (claimed env-var override) and
`otel_endpoint` (claimed config knob). A repo-wide grep for both returns
zero matches in `crates/`. A user following the example to wire an OTel
collector sees no spans and no error.

```toml config/scrat.toml.example:26-36
# Log files are written as JSONL with daily rotation.
# Default paths:
# - /var/log/scrat.jsonl (Unix, if writable)
# - ~/.local/share/scrat/logs/scrat.jsonl (fallback)
# Override with environment variables:
# - SCRAT_LOG_PATH=/path/to/scrat.jsonl  (rotated daily)
# - SCRAT_LOG_DIR=/path/to/log/dir
# - SCRAT_ENV=dev
# Config options:
# - log_dir = "/path/to/log/dir"
# - otel_endpoint = "http://127.0.0.1:4317"
```

Related: [`orphan-commands-build-and-clean-config`](#orphan-commands-build-and-clean-config).

**Remediation:** Either land the OTel endpoint plumbing against the existing
observability module or delete both references from the example config
and the init templates that mirror it.

<div>&hairsp;</div>

### bump-error-unsupported-ecosystem-dead-variant

**note** · `crates/scrat-core/src/bump.rs:43-49` · effort: trivial

After Phase 4 every driver implements `bump_version_files` with real
behavior or an explicit `Ok(Vec::new())` no-op (Go, Swift, Generic). The
only `BumpError` variants production code returns are `ToolFailed`,
`NoBumpTool`, `Detection`, and `Version`. `UnsupportedEcosystem` is
constructed only in a display-smoke test at `bump.rs:786`. Its error
message — "bump not yet supported for {0} ecosystem" — is misleading;
downstream consumers matching on the variant are writing unreachable code.

```rust crates/scrat-core/src/bump.rs:43-49
    /// No bump tool available for this ecosystem.
    #[error("no bump tool available (install cargo-edit for Rust)")]
    NoBumpTool,

    /// Ecosystem not supported for bump operations.
    #[error("bump not yet supported for {0} ecosystem")]
    UnsupportedEcosystem(Ecosystem),
```

**Remediation:** Remove the variant and the display-only test. Reintroduce
with a real call site if a future scenario actually needs to refuse bump
for an ecosystem.

<div>&hairsp;</div>

### example-config-missing-filter-prefix-docs

**note** · `config/scrat.toml.example:82-92` · effort: trivial

README documents `filter:` as a first-class hook prefix with JSON pipe
semantics and a full TOML example. The canonical
`config/scrat.toml.example` documents only `sync:`. Users starting from
the example — especially users following `scrat init` scaffolds that
mirror it — don't discover the `filter:` capability without reading the
README end-to-end. The implementation in `hooks.rs::split_batches`
handles both prefixes; this is documentation drift rather than a missing
feature.

```toml config/scrat.toml.example:82-92
# ------------------------------------------------------------------------------
# Hooks (pre/post commands per phase of `scrat ship`)
# ------------------------------------------------------------------------------
# Variables: {version}, {prev_version}, {tag}, {changelog_path}, {owner}, {repo}
#
# Commands run in parallel by default. Prefix with "sync:" to create a
# barrier — all prior commands finish, the sync command runs alone, then
# subsequent commands resume in parallel.
#
# Phase order:
#   pre_ship → test → bump → git → release → publish → post_ship
```

**Remediation:** Add a `filter:` paragraph and example to the example
config alongside the `sync:` paragraph. README prose can be lifted
verbatim.

*Verdict: the notes command is the most impactful fix — users are sold a
preview that can't regenerate past releases and won't reflect filter-hook
mutations. The orphan config fields are each trivial individually but
together erode trust across the config surface; the honest fix is
deletion unless the roadmap includes implementation soon. The
`UnsupportedEcosystem` variant is a leftover from the pre-Phase-4 era and
should go.*

<div>&nbsp;</div>

## Remediation Ledger

| Finding | Concern | Location | Effort | Chains |
|---------|---------|----------|--------|--------|
| **The Error Architecture Surface** | | | | |
| [notes-error-flattens-source-chain](#notes-error-flattens-source-chain) | moderate | `crates/scrat-core/src/notes.rs:22-41` | medium | — |
| [observability-returns-anyhow-in-library](#observability-returns-anyhow-in-library) | moderate | `crates/scrat-core/src/observability.rs:67-70` | small | — |
| [observability-writer-silent-discard](#observability-writer-silent-discard) | advisory | `crates/scrat-core/src/observability.rs:196-202` | trivial | — |
| [expect-messages-describe-value-not-invariant](#expect-messages-describe-value-not-invariant) | advisory | `crates/scrat/src/commands/doctor.rs:112-116` | trivial | — |
| **The Supply Chain Surface** | | | | |
| [scrat-core-depends-on-clap-for-value-enum-derives](#scrat-core-depends-on-clap-for-value-enum-derives) | moderate | `crates/scrat-core/Cargo.toml:33` | small | — |
| [serde-saphyr-caret-on-zero-zero-x](#serde-saphyr-caret-on-zero-zero-x) | advisory | `crates/scrat-core/Cargo.toml:36` | trivial | — |
| [ci-lacks-yanked-and-unmaintained-hardening](#ci-lacks-yanked-and-unmaintained-hardening) | advisory | `.config/deny.toml:48-55` | trivial | — |
| [owo-colors-pulls-duplicate-supports-color](#owo-colors-pulls-duplicate-supports-color) | note | `crates/scrat/Cargo.toml:49` | trivial | related: [transitive-getrandom-triplicate](#transitive-getrandom-triplicate) |
| [transitive-getrandom-triplicate](#transitive-getrandom-triplicate) | note | `Cargo.lock:477-513` | small | related: [owo-colors-pulls-duplicate-supports-color](#owo-colors-pulls-duplicate-supports-color) |
| **The Pipeline Efficiency Surface** | | | | |
| [redundant-git-current-branch-per-ship](#redundant-git-current-branch-per-ship) | advisory | `crates/scrat-core/src/preflight.rs:188-198` | small | related: [has-binary-path-probe-not-cached](#has-binary-path-probe-not-cached) |
| [release-profile-missing-lto-and-strip](#release-profile-missing-lto-and-strip) | advisory | `Cargo.toml:57-59` | trivial | — |
| [has-binary-path-probe-not-cached](#has-binary-path-probe-not-cached) | note | `crates/scrat-core/src/detect.rs:131-134` | small | related: [redundant-git-current-branch-per-ship](#redundant-git-current-branch-per-ship) |
| **The Feature Completeness Surface** | | | | |
| [notes-from-flag-ignored-by-cliff-context](#notes-from-flag-ignored-by-cliff-context) | significant | `crates/scrat-core/src/notes.rs:347-371` | small | related: [notes-command-skips-all-hooks](#notes-command-skips-all-hooks) |
| [notes-command-skips-all-hooks](#notes-command-skips-all-hooks) | advisory | `crates/scrat-core/src/notes.rs:158-175` | small | enabled by: [notes-from-flag-ignored-by-cliff-context](#notes-from-flag-ignored-by-cliff-context) |
| [orphan-commands-build-and-clean-config](#orphan-commands-build-and-clean-config) | advisory | `crates/scrat-core/src/config.rs:105-115` | trivial | — |
| [orphan-release-changelog-tool-config](#orphan-release-changelog-tool-config) | advisory | `crates/scrat-core/src/config.rs:117-121` | trivial | — |
| [example-config-advertises-unimplemented-otel-and-env](#example-config-advertises-unimplemented-otel-and-env) | note | `config/scrat.toml.example:26-36` | trivial | — |
| [bump-error-unsupported-ecosystem-dead-variant](#bump-error-unsupported-ecosystem-dead-variant) | note | `crates/scrat-core/src/bump.rs:43-49` | trivial | — |
| [example-config-missing-filter-prefix-docs](#example-config-missing-filter-prefix-docs) | note | `config/scrat.toml.example:82-92` | trivial | — |

<sub>
Generated 2026-04-12 at commit 7b2f302.
Intermediate artifacts: recon.yaml, findings.yaml, report.html.
</sub>
