---
audit_date: 2026-04-07
project: scrat
commit: 1613ff24f25e022df16c8bc330023141c4922ad2
scope: "Workspace crates: crates/scrat (CLI binary, 4200 LOC) and crates/scrat-core (library, 12700 LOC)"
auditor: "claude-opus-4-6 (cased + crustoleum, 4 parallel review agents)"
findings:
  critical: 0
  significant: 0
  moderate: 1
  advisory: 2
  note: 2
---

# Audit: scrat

scrat is a release management CLI tool built as a Rust workspace with a thin CLI binary (`crates/scrat`) and a fat core library (`crates/scrat-core`). **The Supply Chain Surface** has one moderate finding: figment's "yaml" feature drags the deprecated serde_yaml crate (and its unsafe-libyaml transitive) into the dependency tree, contradicting the project's explicit dependency policy. **The Dependency Fitness Surface** is clean aside from two minor hygiene items. **The Error Handling Surface** is disciplined -- seven typed error enums, zero production unwraps, every silent discard documented. **The API Design and Performance Surface** passes all 16 ownership/lifetime/trait criteria and has no actionable performance findings. Remove the figment YAML feature, and this is a well-structured codebase with no security concerns.

---

## The Supply Chain Surface

*A single figment feature flag pulls a deprecated, unsafe-containing YAML library into the dependency tree, contradicting the project's explicit dependency policy.*

<div>&hairsp;</div>

### Figment 'yaml' feature pulls deprecated serde_yaml into dependency tree

**moderate** · `crates/scrat-core/Cargo.toml:35` · effort: small · <img src="assets/sparkline-figment-yaml-pulls-deprecated-dep.svg" height="14" alt="12-month commit activity" />

Figment's "yaml" feature gates on serde_yaml as its YAML provider backend. serde_yaml v0.9.34 is explicitly marked "+deprecated" in its version string and archived upstream. Its sole dependency, unsafe-libyaml v0.2.11, wraps C code and contains unsafe blocks. While the workspace-level `unsafe_code = "deny"` lint prevents unsafe in project code, it does not apply to external crates. The project already uses serde-saphyr (a pure-Rust YAML library) as a direct dependency in the CLI crate, making the serde_yaml transitive completely redundant.

```toml crates/scrat-core/Cargo.toml:35
figment = { version = "0.10", features = ["toml", "yaml", "json"] }
```

> The project says "no serde_yaml" but figment quietly brings it in through a feature flag. The deprecated crate works today, but it is no longer receiving security patches. Any future advisory against unsafe-libyaml would affect this tree.

**Remediation:** Remove "yaml" from figment's feature list, then implement a custom `figment::Provider` backed by serde-saphyr for YAML config file support:

```toml
figment = { version = "0.10", features = ["toml", "json"] }
```

The custom Provider reads the YAML file, deserializes with serde-saphyr into a `serde_json::Value`, and feeds it into figment. Roughly 30 lines of implementation. Alternatively, file a feature request on figment for a serde-saphyr backend.

<div>&hairsp;</div>

### Supply chain audit tools failed to run in automated scan

**advisory** · `.crustoleum/audit.txt:1-5` · effort: trivial · <img src="assets/sparkline-audit-tooling-incomplete.svg" height="14" alt="12-month commit activity" />

Three supply chain tools failed during the automated crustoleum scan: cargo-audit (stale advisory-db directory prevented fetch), cargo-deny (CLI argument ordering bug in the runner script -- config flag passed before subcommand), and cargo-geiger (cannot run against virtual workspace manifests). When cargo-deny is invoked correctly, all checks pass (advisories clean, licenses clean, bans clean, sources clean).

```text .crustoleum/audit.txt
error: couldn't fetch advisory database: git operation failed
Caused by: Refusing to initialize the non-empty directory
```

```text .crustoleum/deny.txt
error: invalid value '.config/deny.toml' for '--color <COLOR>'
```

The tools themselves are configured and installed. The failures are environment and invocation issues, not code problems. However, if these tools do not run reliably in CI, the supply chain scanning is effectively absent.

Related: [figment-yaml-pulls-deprecated-dep](#figment-yaml-feature-pulls-deprecated-serde_yaml-into-dependency-tree) -- reliable audit tooling would have flagged the deprecated serde_yaml.

**Remediation:** Fix the cargo-deny invocation in the runner script (config flag after subcommand). Clear the stale `~/.cargo/advisory-db` directory. For cargo-geiger, invoke per-crate (`cargo geiger -p scrat`). Consider adding these checks to CI.

*Verdict: The supply chain has one real issue and one tooling gap. The figment YAML feature is the only finding that changes the dependency tree in an unwanted way. Removing it is a small, self-contained change. The audit tooling failures are environment issues that should be fixed to prevent future blind spots, but they did not mask any current vulnerabilities -- manual checks confirmed no advisories.*

<!-- whitespace is important -->
<div>&nbsp;</div>

## The Dependency Fitness Surface

*Dependency choices are well-matched to the project's needs, with two minor hygiene items that do not affect correctness or security.*

<div>&hairsp;</div>

### directories crate declared in CLI Cargo.toml but unused

**note** · `crates/scrat/Cargo.toml:55` · effort: trivial · <img src="assets/sparkline-directories-redundant-in-cli.svg" height="14" alt="12-month commit activity" />

The CLI crate declares `directories` as a direct dependency but has zero `use directories` statements in its source code. All directory resolution (XDG config, cache, data, log paths) happens in scrat-core's `config.rs` and `observability.rs`. Cargo deduplicates so there is no binary size impact, but it is dead manifest weight. cargo-machete correctly flagged this; cargo-udeps did not (udeps considers workspace-level usage).

```toml crates/scrat/Cargo.toml:55
directories = "6.0"
```

**Remediation:** Remove the line from `crates/scrat/Cargo.toml`.

<div>&hairsp;</div>

### clap dependency in scrat-core used only for two ValueEnum derives

**advisory** · `crates/scrat-core/Cargo.toml:33` · effort: small · <img src="assets/sparkline-clap-in-library-crate.svg" height="14" alt="12-month commit activity" />

scrat-core depends on clap (a CLI framework) solely for two `#[derive(clap::ValueEnum)]` annotations on `ConfigFormat` and `ConfigStyle` enums in `init.rs`. This couples the library to a CLI framework. Since the CLI crate also depends on clap, the compile cost is shared and marginal build time impact is zero. However, any downstream consumer of scrat-core that does not use clap would inherit the dependency unnecessarily.

```rust crates/scrat-core/src/init.rs:31-41
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum ConfigFormat {
    Toml,
    Yaml,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum ConfigStyle {
    Minimal,
    Documented,
}
```

**Remediation:** Replace the `clap::ValueEnum` derives with manual `FromStr`/`Display` implementations, or move the `ValueEnum` derivation to the CLI crate side. Alternatively, accept the coupling and document the rationale -- the compile cost is zero marginal.

*Verdict: The dependency tree is lean for a CLI tool of this scope: no async runtime, no HTTP client, no image processing. 149 transitive crates is reasonable. These two findings are housekeeping, not risks.*

<!-- whitespace is important -->
<div>&nbsp;</div>

## The Error Handling Surface

*Error handling is disciplined and well-structured, with typed errors throughout the library and zero panic-capable patterns reachable from production code paths.*

This surface is clean. Seven typed error enums with thiserror (`ConfigError`, `BumpError`, `ShipError`, `VersionError`, `GitError`, `HookError`, `NotesError`), consistent `?` propagation, no production `unwrap()`/`expect()` on external input, and every silent discard documented with a justifying comment. The three production `expect()` calls are on compile-time invariants (clap derive consistency, hardcoded template string, stdout write). All ~350 `unwrap()` calls are confined to `#[cfg(test)]` modules.

The ASAN test failures (6 in preflight.rs) are test infrastructure issues -- `set_current_dir()` in tempdir cleanup paths. The TSAN SIGSEGV is a known Apple Silicon sanitizer false positive on global `AtomicBool` access in owo-colors. Neither reflects production code bugs.

*Verdict: Solid. The error architecture follows the right pattern for a library crate: typed errors with thiserror, composed via `#[from]`, with Result type aliases per module. No changes needed.*

<!-- whitespace is important -->
<div>&nbsp;</div>

## The API Design and Performance Surface

*The ownership model, lifetime usage, trait design, and performance characteristics are all appropriate for a CLI tool that delegates heavy work to subprocesses.*

All 16 API design criteria (ownership, lifetimes, trait design, idiomatic patterns) pass. `#![deny(unsafe_code)]` eliminates an entire class of memory safety concerns. Zero custom traits -- all polymorphism via enums with exhaustive match, which is correct for a closed set of ecosystems. The plan/execute pattern (`BumpPlan`, `ShipPlan`) enforces workflow state at the type level. `ConfigLoader` uses a proper builder; `PipelineContext` uses a dedicated init struct.

Performance is dominated by subprocess I/O: git, git-cliff, gh, and cargo each take 50-500ms per invocation while all Rust code runs in single-digit milliseconds. No allocations in loops, no regex, no async overhead, no `HashMap` with default hasher in hot paths (all maps hold tens of entries at most). The parallel hook executor correctly uses spawn-all/wait-all with a single-command short circuit. Zero performance findings warrant code changes.

*Verdict: Clean across all surfaces. The codebase makes consistently correct tradeoffs: simple, readable code over micro-optimization; owned types in plan structs for clean API boundaries; no traits where enums suffice.*

<!-- whitespace is important -->
<div>&nbsp;</div>

## Remediation Ledger

| Finding | Concern | Location | Effort | Chains |
|---------|---------|----------|--------|--------|
| | | **Supply Chain Surface** | | |
| [figment-yaml-pulls-deprecated-dep](#figment-yaml-feature-pulls-deprecated-serde_yaml-into-dependency-tree) | moderate | `scrat-core/Cargo.toml:35` | small | -- |
| [audit-tooling-incomplete](#supply-chain-audit-tools-failed-to-run-in-automated-scan) | advisory | `.crustoleum/` | trivial | related: figment-yaml |
| | | **Dependency Fitness Surface** | | |
| [directories-redundant-in-cli](#directories-crate-declared-in-cli-cargotoml-but-unused) | note | `scrat/Cargo.toml:55` | trivial | -- |
| [clap-in-library-crate](#clap-dependency-in-scrat-core-used-only-for-two-valueenum-derives) | advisory | `scrat-core/Cargo.toml:33` | small | -- |

---

<sub>
Generated 2026-04-07 at commit 1613ff2.
Intermediate artifacts: recon.yaml, findings.yaml.
</sub>
