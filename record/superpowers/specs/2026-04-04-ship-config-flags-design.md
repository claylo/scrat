# Ship Config Flags

**Date:** 2026-04-04
**Status:** Approved
**Scope:** Add `no_*` config fields to `[ship]` so CLI flags have config-file equivalents

## Problem

Every `--no-*` flag on `scrat ship` controls whether a pipeline phase runs. These flags have no config-file equivalent. If you always skip publish (e.g., private tool), you must pass `--no-publish` on every invocation. Worse, preflight checks credentials for phases you never intend to run, causing false failures like "CARGO_REGISTRY_TOKEN not set" when `no_publish` is only set via CLI.

`ShipConfig` currently has only `confirm`. All `no_*` values are built purely from CLI args.

## Design

### Config model

Add 10 `Option<bool>` fields to `ShipConfig` in `crates/scrat-core/src/config.rs`:

```rust
pub struct ShipConfig {
    pub confirm: Option<bool>,
    pub no_changelog: Option<bool>,
    pub no_publish: Option<bool>,
    pub no_push: Option<bool>,
    pub no_release: Option<bool>,
    pub no_deps: Option<bool>,
    pub no_stats: Option<bool>,
    pub no_notes: Option<bool>,
    pub no_test: Option<bool>,
    pub no_tag: Option<bool>,
    pub no_git: Option<bool>,
}
```

TOML example:

```toml
[ship]
no_publish = true
no_release = true
```

All fields are `Option<bool>` with `None` meaning "not set" (same as the existing `confirm` field). Serde's default handles missing fields as `None`.

### Merge logic

In `crates/scrat/src/commands/ship.rs`, when building `ShipOptions`, merge CLI args with config. CLI flag wins (if true), otherwise config value, otherwise false:

```rust
let ship_cfg = config.ship.as_ref();

let options = ShipOptions {
    explicit_version: args.version,
    no_changelog: args.no_changelog
        || ship_cfg.and_then(|s| s.no_changelog).unwrap_or(false),
    no_publish: args.no_publish
        || ship_cfg.and_then(|s| s.no_publish).unwrap_or(false),
    // ... same pattern for all 10 flags
    dry_run: args.dry_run,  // CLI-only, no config equivalent
    draft_override,         // already handled via release.draft
};
```

### Exclusions

- `dry_run` stays CLI-only. A config `dry_run = true` would silently prevent all releases.
- `draft` / `no_draft` already has config support via `release.draft`. No duplication needed.
- `explicit_version` stays CLI-only. A pinned version in config makes no sense across releases.

### Files changed

| File | Change |
|------|--------|
| `crates/scrat-core/src/config.rs` | Add 10 fields to `ShipConfig` |
| `crates/scrat/src/commands/ship.rs` | Merge config into `ShipOptions` construction |
| `README.md` | Update `[ship]` config reference |
| `docs/getting-started.md` | Update config example if relevant |

### Tests

1. **Config parsing** (`config.rs`): Deserialize `[ship]` with various `no_*` fields, verify round-trip.
2. **Merge behavior** (`ship.rs` or CLI integration): Verify CLI flag overrides config, config overrides default, missing config defaults to false.
3. **Preflight integration**: Verify `no_publish = true` in config skips registry credential check (the original bug).

### Backwards compatibility

All new fields are `Option<bool>` defaulting to `None`. Existing config files without `[ship]` or with only `confirm` continue to work unchanged.
