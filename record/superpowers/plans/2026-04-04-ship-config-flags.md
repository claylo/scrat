# Ship Config Flags Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `no_*` config fields to `[ship]` so every CLI `--no-*` flag has a config-file equivalent, with CLI flags overriding config values.

**Architecture:** Add 10 `Option<bool>` fields to `ShipConfig` in scrat-core. Merge config values into `ShipOptions` construction in the CLI crate's `cmd_ship`. CLI flag (if true) wins over config value, config wins over default (false).

**Tech Stack:** Rust, serde, clap, figment (config loading), cargo-nextest (testing)

---

### Task 1: Add fields to `ShipConfig`

**Files:**
- Modify: `crates/scrat-core/src/config.rs:200-209`

- [ ] **Step 1: Add the 10 `Option<bool>` fields to `ShipConfig`**

In `crates/scrat-core/src/config.rs`, replace the `ShipConfig` struct:

```rust
/// Ship command behavior.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct ShipConfig {
    /// Prompt for confirmation before executing (default: true).
    ///
    /// When `None` or `Some(true)`, `scrat ship` shows the plan and asks
    /// for confirmation before executing. Set to `false` for CI/scripted use.
    /// The `--yes`/`-y` CLI flag overrides this at runtime.
    pub confirm: Option<bool>,
    /// Skip changelog generation during bump (equivalent to `--no-changelog`).
    pub no_changelog: Option<bool>,
    /// Skip publishing to registry (equivalent to `--no-publish`).
    pub no_publish: Option<bool>,
    /// Skip git push (equivalent to `--no-push`).
    pub no_push: Option<bool>,
    /// Skip GitHub release creation (equivalent to `--no-release`).
    pub no_release: Option<bool>,
    /// Skip dependency diff (equivalent to `--no-deps`).
    pub no_deps: Option<bool>,
    /// Skip release statistics collection (equivalent to `--no-stats`).
    pub no_stats: Option<bool>,
    /// Skip release notes rendering (equivalent to `--no-notes`).
    pub no_notes: Option<bool>,
    /// Skip running tests (equivalent to `--no-test`).
    pub no_test: Option<bool>,
    /// Skip git tag creation (equivalent to `--no-tag`).
    pub no_tag: Option<bool>,
    /// Skip entire git phase — commit, tag, push (equivalent to `--no-git`).
    pub no_git: Option<bool>,
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check -p scrat-core 2>&1 | tail -5`
Expected: compiles clean. All new fields are `Option<bool>` with `Default` deriving `None`, so no existing code breaks.

- [ ] **Step 3: Run existing tests to confirm no regressions**

Run: `cargo nextest run -p scrat-core -E 'test(config::tests)' 2>&1 | tail -10`
Expected: all config tests pass. The existing `test_config_with_ship_section` only sets `confirm`, and missing fields default to `None`.

---

### Task 2: Add config parsing tests

**Files:**
- Modify: `crates/scrat-core/src/config.rs` (test module, after `test_config_ship_defaults_to_none` ~line 1014)

- [ ] **Step 1: Write test for parsing `[ship]` with `no_*` fields**

Add after the `test_config_ship_defaults_to_none` test:

```rust
#[test]
fn test_config_ship_no_flags() {
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("config.toml");
    fs::write(
        &config_path,
        r#"
[ship]
confirm = false
no_publish = true
no_release = true
no_test = true
"#,
    )
    .unwrap();

    let config_path = Utf8PathBuf::try_from(config_path).unwrap();
    let (config, _sources) = ConfigLoader::new()
        .with_user_config(false)
        .with_file(&config_path)
        .load()
        .unwrap();

    let ship = config.ship.unwrap();
    assert_eq!(ship.confirm, Some(false));
    assert_eq!(ship.no_publish, Some(true));
    assert_eq!(ship.no_release, Some(true));
    assert_eq!(ship.no_test, Some(true));
    // Unset fields remain None
    assert!(ship.no_changelog.is_none());
    assert!(ship.no_push.is_none());
    assert!(ship.no_deps.is_none());
    assert!(ship.no_stats.is_none());
    assert!(ship.no_notes.is_none());
    assert!(ship.no_tag.is_none());
    assert!(ship.no_git.is_none());
}

#[test]
fn test_config_ship_all_no_flags() {
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("config.toml");
    fs::write(
        &config_path,
        r#"
[ship]
no_changelog = true
no_publish = true
no_push = true
no_release = true
no_deps = true
no_stats = true
no_notes = true
no_test = true
no_tag = true
no_git = true
"#,
    )
    .unwrap();

    let config_path = Utf8PathBuf::try_from(config_path).unwrap();
    let (config, _sources) = ConfigLoader::new()
        .with_user_config(false)
        .with_file(&config_path)
        .load()
        .unwrap();

    let ship = config.ship.unwrap();
    assert_eq!(ship.no_changelog, Some(true));
    assert_eq!(ship.no_publish, Some(true));
    assert_eq!(ship.no_push, Some(true));
    assert_eq!(ship.no_release, Some(true));
    assert_eq!(ship.no_deps, Some(true));
    assert_eq!(ship.no_stats, Some(true));
    assert_eq!(ship.no_notes, Some(true));
    assert_eq!(ship.no_test, Some(true));
    assert_eq!(ship.no_tag, Some(true));
    assert_eq!(ship.no_git, Some(true));
}
```

- [ ] **Step 2: Run the new tests**

Run: `cargo nextest run -p scrat-core -E 'test(config::tests::test_config_ship)' 2>&1 | tail -10`
Expected: all 4 ship config tests pass (2 existing + 2 new).

---

### Task 3: Wire config merge into CLI `cmd_ship`

**Files:**
- Modify: `crates/scrat/src/commands/ship.rs:101-115`

- [ ] **Step 1: Update `ShipOptions` construction to merge config values**

In `crates/scrat/src/commands/ship.rs`, replace lines 101-115 (the `let options = ShipOptions { ... }` block) with:

```rust
    let ship_cfg = config.ship.as_ref();

    let options = ShipOptions {
        explicit_version: args.version,
        no_changelog: args.no_changelog
            || ship_cfg.and_then(|s| s.no_changelog).unwrap_or(false),
        no_publish: args.no_publish
            || ship_cfg.and_then(|s| s.no_publish).unwrap_or(false),
        no_push: args.no_push
            || ship_cfg.and_then(|s| s.no_push).unwrap_or(false),
        no_release: args.no_release
            || ship_cfg.and_then(|s| s.no_release).unwrap_or(false),
        no_deps: args.no_deps
            || ship_cfg.and_then(|s| s.no_deps).unwrap_or(false),
        no_stats: args.no_stats
            || ship_cfg.and_then(|s| s.no_stats).unwrap_or(false),
        no_notes: args.no_notes
            || ship_cfg.and_then(|s| s.no_notes).unwrap_or(false),
        dry_run: args.dry_run,
        no_test: args.no_test
            || ship_cfg.and_then(|s| s.no_test).unwrap_or(false),
        no_tag: args.no_tag
            || ship_cfg.and_then(|s| s.no_tag).unwrap_or(false),
        no_git: args.no_git
            || ship_cfg.and_then(|s| s.no_git).unwrap_or(false),
        draft_override,
    };
```

Note: `dry_run` and `draft_override` are intentionally NOT merged from config (see spec exclusions).

- [ ] **Step 2: Verify it compiles**

Run: `cargo check -p scrat 2>&1 | tail -5`
Expected: compiles clean.

- [ ] **Step 3: Run full test suite to confirm no regressions**

Run: `cargo nextest run -p scrat -p scrat-core 2>&1 | tail -10`
Expected: all tests pass.

---

### Task 4: Update README config reference

**Files:**
- Modify: `README.md:403-407`

- [ ] **Step 1: Expand the `[ship]` section in the config reference**

In `README.md`, replace the `[ship]` block (lines 403-407):

```toml
[ship]
# Prompt for confirmation before executing (default: true)
# Set to false for CI/scripted use. --yes/-y flag also skips.
# confirm = true

# Skip pipeline phases permanently (equivalent to --no-* CLI flags).
# CLI flags override these — passing --no-publish on a run where
# no_publish is already true in config is harmless.
# no_changelog = false
# no_publish = false
# no_push = false
# no_release = false
# no_deps = false
# no_stats = false
# no_notes = false
# no_test = false
# no_tag = false
# no_git = false
```

- [ ] **Step 2: Verify markdown renders correctly**

Skim the full `[ship]` section in context to make sure it reads well within the larger config reference block.

---

### Task 5: Update getting-started config example

**Files:**
- Modify: `docs/getting-started.md:229-256`

- [ ] **Step 1: Add `[ship]` example with `no_*` fields**

In `docs/getting-started.md`, in the Configuration section's TOML example (around line 253), add a `no_publish` example to the existing `[ship]` block:

```toml
# Ship command behavior
[ship]
confirm = true  # default — set to false for CI/scripted use
# no_publish = true   # skip registry publish (e.g., private tools)
# no_release = true   # skip GitHub release creation
```

---

### Task 6: Final verification

- [ ] **Step 1: Run full workspace check**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -20`
Expected: clean formatting and no clippy warnings.

- [ ] **Step 2: Run full test suite**

Run: `cargo nextest run --workspace 2>&1 | tail -15`
Expected: all tests pass.

- [ ] **Step 3: Manual smoke test**

Verify Clay's original issue is fixed. With `.config/scrat.toml` containing:

```toml
[ship]
no_publish = true
```

Run: `scrat ship --dry-run`
Expected: no "CARGO_REGISTRY_TOKEN not set" error. The preflight check for registry credentials should be skipped.
