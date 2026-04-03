# Cliff Config Ownership Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** scrat owns git-cliff configuration by default instead of relying on `cliff.toml` in the project. Version strategy detection is based on binary presence, not config file presence.

**Architecture:** `detect_version_strategy()` checks for the `git-cliff` binary on PATH instead of `cliff.toml`. `compute_via_cliff()` writes a per-ecosystem `[bump]` config to a temp file and passes it via `--config`. A new `version.cliff_config` field in `VersionConfig` lets users opt in to their own cliff.toml.

**Tech Stack:** Rust, tempfile (already a dev dep, needs to become a regular dep in scrat-core), git-cliff CLI

---

## File Map

| File | Action | Responsibility |
|------|--------|---------------|
| `crates/scrat-core/src/detect/mod.rs` | Modify | Change `detect_version_strategy()` to check binary, not file |
| `crates/scrat-core/src/detect/rust.rs` | Modify | Change `detect_changelog_tool()` to check binary, not file |
| `crates/scrat-core/src/version/conventional.rs` | Modify | Pass `--config <temp>` with ecosystem-specific bump config |
| `crates/scrat-core/src/config.rs` | Modify | Add `cliff_config` field to `VersionConfig` |
| `crates/scrat-core/src/ecosystem.rs` | Modify | Add `bump_config()` method to `Ecosystem` |
| `crates/scrat-core/src/bump.rs` | Modify | Thread ecosystem through to `compute_next_version()` |
| `crates/scrat-core/Cargo.toml` | Modify | Move `tempfile` from dev-dependencies to dependencies |

---

### Task 1: Add `cliff_config` to `VersionConfig`

**Files:**
- Modify: `crates/scrat-core/src/config.rs:86-92`

- [ ] **Step 1: Add the field**

In `config.rs`, update the `VersionConfig` struct and its doc comment:

```rust
/// Version strategy configuration.
///
/// Normally auto-detected from the `git-cliff` binary on PATH.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct VersionConfig {
    /// Override the version strategy.
    ///
    /// Possible values: `"conventional-commits"`, `"interactive"`, `"explicit"`.
    pub strategy: Option<String>,
    /// Path to a cliff.toml to use for version computation instead of scrat's
    /// built-in per-ecosystem config. When set, scrat passes this to
    /// `git-cliff --bumped-version --config <path>`.
    pub cliff_config: Option<String>,
}
```

- [ ] **Step 2: Run tests**

Run: `just test`
Expected: All 494 tests pass. The field is `Option` with serde defaults, so no existing config or test breaks.

- [ ] **Step 3: Commit**

Message: `feat(config): add version.cliff_config opt-in field`

---

### Task 2: Add `bump_config()` to `Ecosystem`

**Files:**
- Modify: `crates/scrat-core/src/ecosystem.rs`

- [ ] **Step 1: Write the test**

Add to the `tests` module in `ecosystem.rs`:

```rust
#[test]
fn rust_bump_config_disables_breaking_always_major() {
    let cfg = Ecosystem::Rust.bump_config();
    assert!(cfg.contains("breaking_always_bump_major = false"));
}

#[test]
fn node_bump_config_enables_breaking_always_major() {
    let cfg = Ecosystem::Node.bump_config();
    assert!(cfg.contains("breaking_always_bump_major = true"));
}

#[test]
fn generic_bump_config_enables_breaking_always_major() {
    let cfg = Ecosystem::Generic.bump_config();
    assert!(cfg.contains("breaking_always_bump_major = true"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `just test`
Expected: Compile error — `bump_config` method doesn't exist yet.

- [ ] **Step 3: Implement `bump_config()`**

Add to the `impl Ecosystem` block in `ecosystem.rs`:

```rust
/// Return the built-in git-cliff `[bump]` configuration for this ecosystem.
///
/// Rust treats `0.x` as stable (breaking changes bump minor, not major).
/// All other ecosystems follow standard semver.
pub fn bump_config(&self) -> &'static str {
    match self {
        Self::Rust => concat!(
            "[bump]\n",
            "breaking_always_bump_minor = true\n",
            "breaking_always_bump_major = false\n",
        ),
        _ => concat!(
            "[bump]\n",
            "breaking_always_bump_minor = false\n",
            "breaking_always_bump_major = true\n",
        ),
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `just test`
Expected: All tests pass including the 3 new ones.

- [ ] **Step 5: Commit**

Message: `feat(ecosystem): add per-ecosystem cliff bump config`

---

### Task 3: Change version strategy detection to check binary

**Files:**
- Modify: `crates/scrat-core/src/detect/mod.rs:93-115`
- Modify: `crates/scrat-core/src/detect/mod.rs` tests

- [ ] **Step 1: Update the tests**

Replace the existing detection tests. The key change: `detect_cc_strategy_cliff` no longer creates `cliff.toml` — it tests that ConventionalCommits is detected when git-cliff binary is available (which it is in this dev environment). `interactive_when_no_cc_config` needs to reflect the new behavior.

Replace these tests:

```rust
#[test]
fn detect_cc_strategy_when_git_cliff_available() {
    // git-cliff is installed in the dev environment.
    // If this test runs where git-cliff is NOT installed, it should
    // be skipped or the assertion inverted.
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join("Cargo.toml"), "[package]").unwrap();

    let det = detect_project(utf8_tmp(&tmp)).unwrap();
    if super::has_binary("git-cliff") {
        assert_eq!(
            det.version_strategy,
            VersionStrategy::ConventionalCommits {
                tool: ChangelogTool::GitCliff
            }
        );
    } else {
        assert_eq!(det.version_strategy, VersionStrategy::Interactive);
    }
}

#[test]
fn detect_cc_strategy_cog() {
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join("Cargo.toml"), "[package]").unwrap();
    fs::write(tmp.path().join("cog.toml"), "").unwrap();

    let det = detect_project(utf8_tmp(&tmp)).unwrap();
    // cog.toml still triggers Cog strategy (cog is its own tool)
    assert_eq!(
        det.version_strategy,
        VersionStrategy::ConventionalCommits {
            tool: ChangelogTool::Cog
        }
    );
}

#[test]
fn detect_version_strategy_unit() {
    let tmp = TempDir::new().unwrap();
    let root = utf8_tmp(&tmp);

    let strategy = detect_version_strategy(root);
    if super::has_binary("git-cliff") {
        assert!(matches!(
            strategy,
            VersionStrategy::ConventionalCommits {
                tool: ChangelogTool::GitCliff
            }
        ));
    } else {
        assert_eq!(strategy, VersionStrategy::Interactive);
    }
}
```

Remove the `cliff_takes_priority_over_cog` test (cliff.toml no longer participates in strategy detection). Remove `interactive_when_no_cc_config` (it asserts Interactive when no cliff.toml — that behavior no longer applies).

- [ ] **Step 2: Update `detect_version_strategy()`**

```rust
/// Determine the version strategy from available tooling.
///
/// Priority:
/// 1. `git-cliff` binary on PATH → `ConventionalCommits(GitCliff)`
/// 2. `cog.toml` present        → `ConventionalCommits(Cog)`
/// 3. Neither                    → `Interactive`
pub fn detect_version_strategy(project_root: &Utf8Path) -> VersionStrategy {
    if has_binary("git-cliff") {
        debug!("git-cliff binary found on PATH");
        return VersionStrategy::ConventionalCommits {
            tool: ChangelogTool::GitCliff,
        };
    }

    if project_root.join("cog.toml").is_file() {
        debug!("found cog.toml");
        return VersionStrategy::ConventionalCommits {
            tool: ChangelogTool::Cog,
        };
    }

    VersionStrategy::Interactive
}
```

- [ ] **Step 3: Run tests**

Run: `just test`
Expected: All tests pass.

- [ ] **Step 4: Commit**

Message: `fix(detect): base version strategy on git-cliff binary, not cliff.toml`

---

### Task 4: Change changelog tool detection to check binary

**Files:**
- Modify: `crates/scrat-core/src/detect/rust.rs:50-59`
- Modify: `crates/scrat-core/src/detect/rust.rs` tests

- [ ] **Step 1: Update `detect_changelog_tool()`**

```rust
/// Check which changelog tool is available for this project.
fn detect_changelog_tool(project_root: &Utf8Path) -> Option<ChangelogTool> {
    if super::has_binary("git-cliff") {
        Some(ChangelogTool::GitCliff)
    } else if project_root.join("cog.toml").is_file() {
        Some(ChangelogTool::Cog)
    } else {
        None
    }
}
```

- [ ] **Step 2: Update tests**

Replace the `rust_changelog_tool_cliff` test:

```rust
#[test]
fn rust_changelog_tool_when_git_cliff_available() {
    let tmp = TempDir::new().unwrap();
    let tool = detect_changelog_tool(utf8_tmp(&tmp));
    if super::has_binary("git-cliff") {
        assert_eq!(tool, Some(ChangelogTool::GitCliff));
    } else {
        assert_eq!(tool, None);
    }
}
```

The `rust_changelog_tool_cog` test stays as-is (still checks `cog.toml`). Update `rust_no_changelog_tool` to account for git-cliff being installed:

```rust
#[test]
fn rust_no_changelog_tool_when_nothing_available() {
    let tmp = TempDir::new().unwrap();
    let tool = detect_changelog_tool(utf8_tmp(&tmp));
    if super::has_binary("git-cliff") {
        assert_eq!(tool, Some(ChangelogTool::GitCliff));
    } else {
        assert_eq!(tool, None);
    }
}
```

- [ ] **Step 3: Run tests**

Run: `just test`
Expected: All tests pass.

- [ ] **Step 4: Commit**

Message: `fix(detect): base changelog tool on binary presence, not cliff.toml`

---

### Task 5: Thread ecosystem into `compute_next_version()` and pass `--config`

**Files:**
- Modify: `crates/scrat-core/Cargo.toml` — move tempfile to `[dependencies]`
- Modify: `crates/scrat-core/src/version/conventional.rs`
- Modify: `crates/scrat-core/src/bump.rs:147-155`

- [ ] **Step 1: Move `tempfile` to regular dependencies**

In `crates/scrat-core/Cargo.toml`, move `tempfile` from `[dev-dependencies]` to `[dependencies]`.

- [ ] **Step 2: Update `compute_next_version()` signature**

In `version/conventional.rs`, change the signature and `compute_via_cliff` to accept ecosystem and an optional config path override:

```rust
use crate::ecosystem::{ChangelogTool, Ecosystem};

/// Compute the next version using a conventional-commit tool.
///
/// - **git-cliff**: writes a temp config with ecosystem-specific `[bump]`
///   rules and runs `git cliff --bumped-version --config <temp>`.
/// - **cog**: runs `cog bump --dry-run --auto`
#[instrument(skip(cliff_config_override))]
pub fn compute_next_version(
    tool: ChangelogTool,
    ecosystem: Ecosystem,
    cliff_config_override: Option<&str>,
) -> VersionResult<Version> {
    match tool {
        ChangelogTool::GitCliff => compute_via_cliff(ecosystem, cliff_config_override),
        ChangelogTool::Cog => compute_via_cog(),
    }
}

fn compute_via_cliff(
    ecosystem: Ecosystem,
    cliff_config_override: Option<&str>,
) -> VersionResult<Version> {
    debug!("computing version via git-cliff");

    // Determine which config to use: user's explicit cliff.toml, or scrat's
    // built-in per-ecosystem bump config written to a temp file.
    let tmp_file;
    let config_path = if let Some(path) = cliff_config_override {
        path.to_string()
    } else {
        tmp_file = tempfile::Builder::new()
            .prefix("scrat-cliff-")
            .suffix(".toml")
            .tempfile()
            .map_err(|e| VersionError::ToolFailed {
                tool: "git-cliff".into(),
                message: format!("failed to create temp config: {e}"),
            })?;
        std::fs::write(tmp_file.path(), ecosystem.bump_config()).map_err(|e| {
            VersionError::ToolFailed {
                tool: "git-cliff".into(),
                message: format!("failed to write temp config: {e}"),
            }
        })?;
        tmp_file
            .path()
            .to_str()
            .expect("temp path is UTF-8")
            .to_string()
    };

    let output = Command::new("git-cliff")
        .args(["--bumped-version", "--config", &config_path])
        .output()
        .map_err(|e| VersionError::ToolFailed {
            tool: "git-cliff".into(),
            message: format!("failed to execute: {e}"),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(VersionError::ToolFailed {
            tool: "git-cliff".into(),
            message: stderr,
        });
    }

    let version_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
    debug!(%version_str, "git-cliff suggested version");
    parse_version(&version_str)
}
```

- [ ] **Step 3: Update the call site in `bump.rs`**

In `bump.rs`, the `plan_bump()` function at the `ConventionalCommits` match arm (~line 147), thread through the ecosystem and cliff_config:

```rust
VersionStrategy::ConventionalCommits { tool } => {
    let cliff_config_override = config
        .version
        .as_ref()
        .and_then(|v| v.cliff_config.as_deref());
    let next = conventional::compute_next_version(
        tool,
        detection.ecosystem,
        cliff_config_override,
    )?;
    let previous = current_or_zero()?;
    Ok(BumpPlan::Ready(ReadyBump {
        previous,
        next,
        strategy: VersionStrategy::ConventionalCommits { tool },
        detection,
    }))
}
```

- [ ] **Step 4: Update tests in `conventional.rs`**

The existing tests call `compute_next_version(ChangelogTool::GitCliff)` — update them to pass the new args:

```rust
#[test]
fn compute_dispatches_to_cliff() {
    let result = compute_next_version(ChangelogTool::GitCliff, Ecosystem::Rust, None);
    match result {
        Ok(v) => {
            assert!(v.major > 0 || v.minor > 0 || v.patch > 0 || v == Version::new(0, 0, 0));
        }
        Err(VersionError::ToolFailed { tool, .. }) => {
            assert_eq!(tool, "git-cliff");
        }
        Err(e) => {
            let _ = e.to_string();
        }
    }
}

#[test]
fn compute_dispatches_to_cog() {
    let result = compute_next_version(ChangelogTool::Cog, Ecosystem::Rust, None);
    match result {
        Ok(v) => {
            assert!(v.major > 0 || v.minor > 0 || v.patch > 0 || v == Version::new(0, 0, 0));
        }
        Err(VersionError::ToolFailed { tool, .. }) => {
            assert_eq!(tool, "cog");
        }
        Err(e) => {
            let _ = e.to_string();
        }
    }
}
```

- [ ] **Step 5: Run tests**

Run: `just test`
Expected: All tests pass.

- [ ] **Step 6: Commit**

Message: `feat(version): use scrat-owned cliff config for version computation`

---

### Task 6: Update documentation

**Files:**
- Modify: `docs/getting-started.md`
- Modify: `README.md`

- [ ] **Step 1: Update getting-started.md**

In the "What scrat Detects Automatically" section (~line 45), change `Version strategy: cliff.toml → conventional commits via git-cliff` to:

```
Version strategy: git-cliff on PATH → conventional commits, otherwise → interactive picker
```

- [ ] **Step 2: Update README.md**

In "### 2. Version Resolution" (~line 108), update the table row for Conventional Commits. Change `cliff.toml present` to `git-cliff installed`. Update the paragraph below the table referencing cliff.toml if needed.

In the "Configuration > Full Reference" section, add `cliff_config` to the `[version]` block:

```toml
[version]
# Override version strategy: conventional-commits, interactive, explicit
# strategy = "conventional-commits"
# Use your own cliff.toml for version computation instead of scrat's built-in
# cliff_config = "cliff.toml"
```

- [ ] **Step 3: Run tests**

Run: `just test`
Expected: All tests still pass.

- [ ] **Step 4: Commit**

Message: `docs: update version strategy detection to reflect binary-based detection`
