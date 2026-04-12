# Multi-registry `check_registry_auth` — deferred design capture

**Status:** Deferred (Phase 4 Task 6 ships single-registry; this is the follow-up anchor)
**Date:** 2026-04-11
**Related plan:** [Phase 4 execution plan](../plans/2026-04-11-ecosystem-modules-phase-4.md) Task 6

## Context

Phase 4 Task 6 migrates `preflight::check_registry_auth` into the `EcosystemDriver`
trait as a `fn check_registry_auth(&self) -> CheckResult` method. Each real driver
encodes a single registry: crates.io for Rust, npmjs for Node, PyPI for Python,
RubyGems for Ruby, Packagist for PHP. Go/Swift/Generic return a passing no-op
because they have no central publish target.

This matches the existing pre-refactor behavior in `preflight::check_registry_auth`.
No regression, no new lock-in — the single-registry-per-ecosystem assumption was
already baked into scrat.

## The concern (captured during Task 6 design review)

Real ecosystems can have multiple publish targets, and the situation is particularly
unsettled in emerging tool ecosystems. Motivating examples:

- **AgentSkills** — multiple competing catalogs, community has not converged on one.
  scrat will want to publish skills eventually. No single canonical registry exists
  today.
- **MCP servers** — same story. The MCP catalog landscape is fragmented and
  changing quarterly.
- **Historical multi-registry splits**
  - Composer vs PEAR (pre-2015 PHP) — PEAR is effectively dead now, but the split
    persisted for almost a decade.
  - Private Packagist / Satis for corporate PHP environments.
  - Private gem servers (RubyGems is the public default but corp-internal gems
    exist on private servers).
  - Private npm / Verdaccio / GitHub Packages for Node.
  - Cargo alt-registries declared in `.cargo/config.toml [registries]`, with
    `CARGO_REGISTRIES_<NAME>_TOKEN` env var lookup.

Today's single-registry-per-driver assumption handles the dominant case for all
7 current scrat ecosystems but leaves no clean slot for publishing to non-default
registries. A user whose project publishes to a private registry must override
the env var at the CI layer; scrat's preflight check still reports against the
default registry name.

## Current state (Phase 4 Task 6 shipping shape)

```rust
pub trait EcosystemDriver {
    /// Check registry auth for the publish phase.
    ///
    /// Currently checks credentials for this ecosystem's default public
    /// registry (crates.io, npmjs, PyPI, RubyGems, Packagist). Multi-registry
    /// and private-registry support is tracked as a follow-up — users with
    /// private registries can override by setting the relevant env var
    /// directly.
    fn check_registry_auth(&self) -> CheckResult;
}
```

Each real driver returns ONE `CheckResult` against ONE hardcoded env var set.

## Options for future expansion

### Option A (rejected): Driver-enumerated `Vec<CheckResult>` return

```rust
fn check_registry_auth(&self) -> Vec<CheckResult>;
```

Each driver enumerates its "known" registries and returns one check per.

- **Why rejected:** "Known registries" is fundamentally fuzzy. Cargo alt-registries
  live in the user's `.cargo/config.toml`, not in scrat's source. Private Packagist
  URLs live in the user's `composer.json` repositories field. Drivers cannot
  enumerate what they don't know. Also breaks the "one CheckResult per named
  thing" convention that the rest of preflight follows.

### Option B (the real path forward): Config-driven registry descriptor

```rust
pub struct RegistryConfig {
    pub name: String,        // "crates.io", "my-private-registry", "agentskills-beta"
    pub env_vars: Vec<String>, // ["CARGO_REGISTRY_TOKEN"] or ["AGENTSKILLS_API_KEY"]
    pub login_hint: String,    // "set CARGO_REGISTRY_TOKEN or run cargo login"
}

fn check_registry_auth(&self, registry: Option<&RegistryConfig>) -> CheckResult;
```

New `[release.registry]` or `[release.registries]` config section. When absent,
driver uses its hardcoded default. When present, driver uses the config's env
var list and registry name. Multi-registry projects declare multiple entries;
`run_preflight` iterates and calls `check_registry_auth` per entry.

- **Why deferred:** This is a full feature — schema changes, figment provider
  update, CLI arg wiring, docs, tests. It belongs in its own design session
  with its own plan. Not a refactor step.
- **Prerequisites for revisiting:** (1) Concrete use case with a chosen non-default
  registry (likely AgentSkills or MCP catalog publishing). (2) Decision on whether
  scrat's config carries raw env var names or abstract "registry identity" that
  maps to env vars per ecosystem.

### Option C (considered, not recommended): `registries()` enumeration method

```rust
fn registries(&self) -> Vec<RegistryDescriptor>;
fn check_registry_auth(&self, registry: &RegistryDescriptor) -> CheckResult;
```

Driver declares what it can publish to; caller picks.

- **Why not recommended:** Same failure mode as Option A — drivers can't enumerate
  user-configured registries. Only works in concert with Option B, and at that
  point Option B alone suffices.

## Decision

**Defer to post-Phase 4.** Ship Task 6 with single-registry semantics and a doc
comment on the trait method explicitly noting the limitation and the env-var
override workaround.

Re-open this capture when:
- A scrat user hits the limitation with a concrete non-default registry, or
- AgentSkills / MCP server publishing becomes a real scrat use case, or
- Cargo alt-registries become common enough in scrat's user base to warrant
  first-class support.

When re-opened, start from Option B. Write a full design doc + plan. Don't retrofit.

## References

- Phase 4 plan: `record/superpowers/plans/2026-04-11-ecosystem-modules-phase-4.md` Task 6
- Phase 4 design: `record/superpowers/specs/2026-04-11-ecosystem-modules-phase-4-design.md`
- Current `check_registry_auth` implementation: `crates/scrat-core/src/preflight.rs:407-458` (pre-Task 6) → `crates/scrat-core/src/ecosystem/<driver>.rs` (post-Task 6)
