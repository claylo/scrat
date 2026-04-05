# Configurable `gh release` + Systematic `--no-*` Flags

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make the GitHub release phase configurable (draft, title, edit-vs-create) and make `--no-*` CLI flags consistent and complete.

**Architecture:** All logic in `scrat-core`. Config gets 3 new `Option<T>` fields on `ReleaseConfig`. Ship gets a `ReleaseOptions` struct and rewritten release phase with edit-vs-create detection. CLI gets renamed/new flags. No new modules or deps.

**Tech Stack:** Rust, clap (derive), serde, `gh` CLI

---

### Task 1: Add new ReleaseConfig fields

**Files:**
- Modify: `crates/scrat-core/src/config.rs` (ReleaseConfig struct + tests)

**Step 1: Add fields to ReleaseConfig**

Add three new fields after `notes_template`:

```rust
/// Release workflow configuration.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct ReleaseConfig {
    /// Override the changelog tool (`"git-cliff"` or `"cog"`).
    pub changelog_tool: Option<ChangelogTool>,
    /// Whether to create a GitHub release (default: `true`).
    pub github_release: Option<bool>,
    /// File paths to attach to the GitHub release as assets.
    ///
    /// Hook commands produce these files; scrat attaches them.
    /// Paths are relative to the project root.
    pub assets: Option<Vec<String>>,
    /// Path to a custom git-cliff template for release notes.
    ///
    /// If unset, uses the built-in template. The template is rendered by
    /// git-cliff (Tera syntax) with scrat's extra data injected into context.
    pub notes_template: Option<String>,
    /// Create the GitHub release as a draft (default: `true`).
    ///
    /// When `true`, `gh release create --draft` is used. Review and publish
    /// with `gh release edit <tag> --draft=false`.
    pub draft: Option<bool>,
    /// Title format for the GitHub release.
    ///
    /// Supports `{var}` interpolation: `{version}`, `{prev_version}`,
    /// `{tag}`, `{owner}`, `{repo}`, `{changelog_path}`.
    /// Default (when `None`): uses the tag as title (e.g., `v1.2.3`).
    pub title: Option<String>,
    /// GitHub Discussions category to associate with the release.
    ///
    /// When set, passes `--discussion-category <value>` to `gh release create`.
    /// Only applies to newly created releases (not edits).
    pub discussion_category: Option<String>,
}
```

**Step 2: Add config deserialization test**

Add to the existing `mod tests` in config.rs:

```rust
#[test]
fn test_config_with_release_draft_and_title() {
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("config.toml");
    fs::write(
        &config_path,
        r#"
[release]
draft = true
title = "{repo} {tag}"
discussion_category = "releases"
"#,
    )
    .unwrap();

    let config_path = Utf8PathBuf::try_from(config_path).unwrap();
    let config = ConfigLoader::new()
        .with_user_config(false)
        .with_file(&config_path)
        .load()
        .unwrap();

    let release = config.release.unwrap();
    assert_eq!(release.draft, Some(true));
    assert_eq!(release.title.as_deref(), Some("{repo} {tag}"));
    assert_eq!(
        release.discussion_category.as_deref(),
        Some("releases")
    );
}
```

**Step 3: Run tests**

Run: `just test`
Expected: All pass (including new test)

**Step 4: Commit**

```
feat(config): add draft, title, and discussion_category to ReleaseConfig
```

---

### Task 2: Rename `skip_tests` → `no_test` and add `no_tag`/`no_git` to ShipOptions

**Files:**
- Modify: `crates/scrat-core/src/ship.rs` (ShipOptions struct + tests)

**Step 1: Update ShipOptions**

```rust
#[derive(Debug, Clone, Default)]
pub struct ShipOptions {
    /// Set the version explicitly (e.g., `"1.2.3"`).
    pub explicit_version: Option<String>,
    /// Skip changelog generation during the bump phase.
    pub no_changelog: bool,
    /// Skip the publish phase entirely.
    pub no_publish: bool,
    /// Skip git push (still commits and tags locally).
    pub no_push: bool,
    /// Skip GitHub release creation.
    pub no_release: bool,
    /// Skip dependency diff computation.
    pub no_deps: bool,
    /// Skip release statistics collection.
    pub no_stats: bool,
    /// Skip release notes rendering (falls back to --generate-notes).
    pub no_notes: bool,
    /// Preview what would happen without making changes.
    pub dry_run: bool,
    /// Skip running tests.
    pub no_test: bool,
    /// Skip git tag creation (still commits and pushes).
    pub no_tag: bool,
    /// Skip entire git phase (commit, tag, push).
    pub no_git: bool,
    /// Override draft mode from CLI (`Some(true)` = `--draft`, `Some(false)` = `--no-draft`).
    pub draft_override: Option<bool>,
}
```

**Step 2: Find-and-replace `skip_tests` → `no_test` in ship.rs**

Three occurrences in `execute()`:
- `self.options.skip_tests` → `self.options.no_test`
- The `PhaseOutcome::Skipped` reason: `"--skip-tests flag"` → `"--no-test flag"`

And the default test:
- `assert!(!opts.skip_tests)` → `assert!(!opts.no_test)`

**Step 3: Add `no_tag` and `no_git` assertions to the default test**

```rust
#[test]
fn ship_options_default() {
    let opts = ShipOptions::default();
    assert!(!opts.dry_run);
    assert!(!opts.no_publish);
    assert!(!opts.no_push);
    assert!(!opts.no_release);
    assert!(!opts.no_deps);
    assert!(!opts.no_stats);
    assert!(!opts.no_notes);
    assert!(!opts.no_test);
    assert!(!opts.no_tag);
    assert!(!opts.no_git);
    assert!(!opts.no_changelog);
    assert!(opts.explicit_version.is_none());
    assert!(opts.draft_override.is_none());
}
```

**Step 4: Run tests**

Run: `just test`
Expected: All pass

**Step 5: Commit**

```
refactor(ship): rename skip_tests to no_test, add no_tag/no_git/draft_override
```

---

### Task 3: Wire `no_tag` and `no_git` into the git phase

**Files:**
- Modify: `crates/scrat-core/src/ship.rs` (execute method + run_git_phase)

**Step 1: Update `run_git_phase` signature to accept `no_tag`**

```rust
fn run_git_phase(
    _project_root: &Utf8Path,
    tag: &str,
    version: &Version,
    no_push: bool,
    no_tag: bool,
) -> ShipResult<GitPhaseResult> {
    // Stage and commit all modified files
    let commit_msg = format!("chore: release {version}");
    let hash = git::commit(&["."], &commit_msg)?;

    // Create annotated tag (unless skipped)
    if !no_tag {
        let tag_msg = format!("Release {version}");
        git::create_tag(tag, &tag_msg)?;
    }

    // Push if requested
    if !no_push {
        let branch = git::current_branch()?.unwrap_or_else(|| "HEAD".into());
        // Only push tags if we created one
        git::push("origin", &branch, !no_tag)?;
        Ok(GitPhaseResult {
            hash,
            branch: Some(branch),
            pushed: true,
        })
    } else {
        Ok(GitPhaseResult {
            hash,
            branch: None,
            pushed: false,
        })
    }
}
```

**Step 2: Update the git phase section in `execute()` to handle `no_git`**

Replace the git phase block (around line 561-585) with:

```rust
on_event(ShipEvent::PhaseStarted(ShipPhase::Git));
let git_outcome = if self.options.no_git {
    PhaseOutcome::Skipped {
        reason: "--no-git flag".into(),
    }
} else if is_dry {
    let tag_msg = if self.options.no_tag {
        " (no tag)"
    } else {
        &format!(", tag {tag}")
    };
    let push_msg = if self.options.no_push {
        " (no push)"
    } else {
        " + push"
    };
    PhaseOutcome::Success {
        message: format!("Would commit{tag_msg}{push_msg}"),
    }
} else {
    let git_result = run_git_phase(
        project_root,
        &tag,
        version,
        self.options.no_push,
        self.options.no_tag,
    )?;
    ctx.record_git(Some(git_result.hash.clone()), git_result.branch.clone());
    let tag_part = if self.options.no_tag {
        String::new()
    } else {
        format!(", tagged {tag}")
    };
    let push_part = if git_result.pushed {
        ", pushed"
    } else {
        " (push skipped)"
    };
    PhaseOutcome::Success {
        message: format!("Committed {}{tag_part}{push_part}", git_result.hash),
    }
};
```

Also: skip `pre_tag`/`post_tag` hooks when `no_git` is set. Wrap the two `run_phase_hooks` calls for `pre_tag` and `post_tag` in `if !self.options.no_git { ... }`.

**Step 3: Run tests**

Run: `just test`
Expected: All pass

**Step 4: Commit**

```
feat(ship): wire no_tag and no_git into git phase
```

---

### Task 4: Add `ReleaseOptions` and rewrite release phase

**Files:**
- Modify: `crates/scrat-core/src/ship.rs` (new struct, rewrite `run_release_phase`, add arg builder tests)

**Step 1: Add `ReleaseOptions` struct and arg-building functions**

Add above `run_release_phase`:

```rust
/// Options for the GitHub release phase.
struct ReleaseOptions<'a> {
    tag: &'a str,
    title: Option<String>,
    draft: bool,
    notes_file: Option<&'a std::path::Path>,
    assets: &'a [String],
    discussion_category: Option<&'a str>,
    project_root: &'a Utf8Path,
}

/// Build args for `gh release create`.
fn build_create_args(opts: &ReleaseOptions<'_>) -> Vec<String> {
    let mut args = vec!["release".into(), "create".into(), opts.tag.into()];

    if let Some(ref title) = opts.title {
        args.push("--title".into());
        args.push(title.clone());
    }

    if opts.draft {
        args.push("--draft".into());
    }

    if let Some(path) = opts.notes_file {
        args.push("--notes-file".into());
        args.push(path.to_string_lossy().to_string());
    } else {
        args.push("--generate-notes".into());
    }

    if let Some(cat) = opts.discussion_category {
        args.push("--discussion-category".into());
        args.push(cat.into());
    }

    for asset in opts.assets {
        args.push(asset.clone());
    }

    args
}

/// Build args for `gh release edit`.
fn build_edit_args(opts: &ReleaseOptions<'_>) -> Vec<String> {
    let mut args = vec!["release".into(), "edit".into(), opts.tag.into()];

    if let Some(ref title) = opts.title {
        args.push("--title".into());
        args.push(title.clone());
    }

    if opts.draft {
        args.push("--draft".into());
    } else {
        args.push("--draft=false".into());
    }

    if let Some(path) = opts.notes_file {
        args.push("--notes-file".into());
        args.push(path.to_string_lossy().to_string());
    }

    args
}
```

**Step 2: Add tests for arg builders**

```rust
#[test]
fn build_create_args_with_all_options() {
    let notes = tempfile::NamedTempFile::new().unwrap();
    let opts = ReleaseOptions {
        tag: "v1.2.3",
        title: Some("myrepo v1.2.3".into()),
        draft: true,
        notes_file: Some(notes.path()),
        assets: &["dist/app.tar.gz".into(), "dist/checksums.txt".into()],
        discussion_category: Some("releases"),
        project_root: Utf8Path::new("/tmp"),
    };
    let args = build_create_args(&opts);
    assert_eq!(args[0], "release");
    assert_eq!(args[1], "create");
    assert_eq!(args[2], "v1.2.3");
    assert!(args.contains(&"--title".into()));
    assert!(args.contains(&"myrepo v1.2.3".into()));
    assert!(args.contains(&"--draft".into()));
    assert!(args.contains(&"--notes-file".into()));
    assert!(args.contains(&"--discussion-category".into()));
    assert!(args.contains(&"releases".into()));
    assert!(args.contains(&"dist/app.tar.gz".into()));
    assert!(args.contains(&"dist/checksums.txt".into()));
}

#[test]
fn build_create_args_minimal() {
    let opts = ReleaseOptions {
        tag: "v0.1.0",
        title: None,
        draft: false,
        notes_file: None,
        assets: &[],
        discussion_category: None,
        project_root: Utf8Path::new("/tmp"),
    };
    let args = build_create_args(&opts);
    assert_eq!(args, vec!["release", "create", "v0.1.0", "--generate-notes"]);
}

#[test]
fn build_edit_args_draft() {
    let opts = ReleaseOptions {
        tag: "v1.0.0",
        title: Some("Release v1.0.0".into()),
        draft: true,
        notes_file: None,
        assets: &[],
        discussion_category: None,
        project_root: Utf8Path::new("/tmp"),
    };
    let args = build_edit_args(&opts);
    assert_eq!(args[0], "release");
    assert_eq!(args[1], "edit");
    assert_eq!(args[2], "v1.0.0");
    assert!(args.contains(&"--draft".into()));
    assert!(args.contains(&"--title".into()));
}

#[test]
fn build_edit_args_publish() {
    let opts = ReleaseOptions {
        tag: "v1.0.0",
        title: None,
        draft: false,
        notes_file: None,
        assets: &[],
        discussion_category: None,
        project_root: Utf8Path::new("/tmp"),
    };
    let args = build_edit_args(&opts);
    assert!(args.contains(&"--draft=false".into()));
}
```

**Step 3: Rewrite `run_release_phase` with edit-vs-create**

```rust
/// Check if a GitHub release already exists for the given tag.
fn release_exists(tag: &str, project_root: &Utf8Path) -> bool {
    Command::new("gh")
        .args(["release", "view", tag])
        .current_dir(project_root.as_std_path())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Upload assets to an existing release, replacing any with the same name.
fn upload_release_assets(
    tag: &str,
    assets: &[String],
    project_root: &Utf8Path,
) -> ShipResult<()> {
    for asset in assets {
        // Try to delete existing asset (ignore failure — may not exist)
        let basename = std::path::Path::new(asset)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| asset.clone());

        let _ = Command::new("gh")
            .args(["release", "delete-asset", tag, &basename, "--yes"])
            .current_dir(project_root.as_std_path())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();

        // Upload
        let status = Command::new("gh")
            .args(["release", "upload", tag, asset])
            .current_dir(project_root.as_std_path())
            .output()
            .map_err(|e| ShipError::PhaseFailed {
                phase: ShipPhase::Release,
                message: format!("failed to upload asset {asset}: {e}"),
            })?;

        if !status.status.success() {
            let stderr = String::from_utf8_lossy(&status.stderr).trim().to_string();
            return Err(ShipError::PhaseFailed {
                phase: ShipPhase::Release,
                message: format!("failed to upload asset {asset}: {stderr}"),
            });
        }
    }
    Ok(())
}

/// Create or update a GitHub release using `gh`.
fn run_release_phase(opts: &ReleaseOptions<'_>) -> ShipResult<ReleasePhaseResult> {
    let exists = release_exists(opts.tag, opts.project_root);

    if exists {
        debug!(tag = opts.tag, "release exists, editing");
        let args = build_edit_args(opts);
        let output = Command::new("gh")
            .args(&args)
            .current_dir(opts.project_root.as_std_path())
            .output()
            .map_err(|e| ShipError::PhaseFailed {
                phase: ShipPhase::Release,
                message: format!("failed to execute gh release edit: {e}"),
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(ShipError::PhaseFailed {
                phase: ShipPhase::Release,
                message: format!("gh release edit failed: {stderr}"),
            });
        }

        // Upload assets separately for edits
        if !opts.assets.is_empty() {
            upload_release_assets(opts.tag, opts.assets, opts.project_root)?;
        }

        let raw_url = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let url = if raw_url.is_empty() { None } else { Some(raw_url) };
        Ok(ReleasePhaseResult { url, edited: true })
    } else {
        debug!(tag = opts.tag, "creating new release");
        let args = build_create_args(opts);
        let output = Command::new("gh")
            .args(&args)
            .current_dir(opts.project_root.as_std_path())
            .output()
            .map_err(|e| ShipError::PhaseFailed {
                phase: ShipPhase::Release,
                message: format!("failed to execute gh release create: {e}"),
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(ShipError::PhaseFailed {
                phase: ShipPhase::Release,
                message: format!("gh release create failed: {stderr}"),
            });
        }

        let raw_url = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let url = if raw_url.is_empty() { None } else { Some(raw_url) };
        Ok(ReleasePhaseResult { url, edited: false })
    }
}
```

Update `ReleasePhaseResult`:

```rust
struct ReleasePhaseResult {
    url: Option<String>,
    edited: bool,
}
```

**Step 4: Run tests**

Run: `just test`
Expected: All pass

**Step 5: Commit**

```
feat(ship): rewrite release phase with edit-vs-create and ReleaseOptions
```

---

### Task 5: Wire release config into ship orchestrator

**Files:**
- Modify: `crates/scrat-core/src/ship.rs` (execute method — release phase block)

**Step 1: Replace the release phase wiring in `execute()`**

In the release phase section of `execute()`, resolve draft and title from config + CLI override, then build `ReleaseOptions` and call the rewritten `run_release_phase`:

```rust
// Resolve release config
let release_cfg = self.config.release.as_ref();
let draft = self
    .options
    .draft_override
    .or_else(|| release_cfg.and_then(|r| r.draft))
    .unwrap_or(true); // default: draft

let title_template = release_cfg.and_then(|r| r.title.as_deref());
let title = title_template.map(|t| hooks::interpolate_command(t, &hook_ctx));

let discussion_category = release_cfg
    .and_then(|r| r.discussion_category.as_deref());
```

Then in the `else` (non-dry, github_release=true) branch, replace the old `run_release_phase` call:

```rust
let release_opts = ReleaseOptions {
    tag: &tag,
    title,
    draft,
    notes_file: notes_file.as_ref().map(|f| f.path()),
    assets,
    discussion_category,
    project_root,
};
let release_result = run_release_phase(&release_opts)?;
ctx.record_release(release_result.url.clone());
let action = if release_result.edited { "Updated" } else { "Created" };
let draft_label = if draft { " (draft)" } else { "" };
let msg = release_result.url.as_ref().map_or_else(
    || format!("{action} GitHub release {tag}{draft_label}"),
    |url| format!("{action} GitHub release{draft_label}: {url}"),
);
PhaseOutcome::Success { message: msg }
```

**Step 2: Update dry-run release output**

```rust
} else if is_dry {
    let draft_label = if draft { " as draft" } else { "" };
    let title_label = title.as_ref().map_or(String::new(), |t| format!(" titled \"{t}\""));
    let notes_msg = if self.options.no_notes {
        " (--generate-notes)"
    } else {
        " (with rendered notes)"
    };
    let asset_count = assets.len();
    let asset_msg = if asset_count > 0 {
        format!(", {asset_count} asset{}", if asset_count == 1 { "" } else { "s" })
    } else {
        String::new()
    };
    PhaseOutcome::Success {
        message: format!("Would create GitHub release for {tag}{draft_label}{title_label}{notes_msg}{asset_msg}"),
    }
}
```

Note: `hooks::interpolate_command` needs to be pub (it already is — confirmed in hooks.rs:414).

**Step 3: Run tests**

Run: `just test`
Expected: All pass

**Step 4: Commit**

```
feat(ship): wire release config (draft, title, discussion_category) into orchestrator
```

---

### Task 6: Update CLI ship command

**Files:**
- Modify: `crates/scrat/src/commands/ship.rs` (ShipArgs + mapping)

**Step 1: Update ShipArgs**

```rust
#[derive(Args, Debug, Default)]
pub struct ShipArgs {
    /// Set version explicitly (e.g., "1.2.3" or "v1.2.3")
    #[arg(long, value_name = "VERSION")]
    pub version: Option<String>,

    /// Skip changelog generation
    #[arg(long)]
    pub no_changelog: bool,

    /// Skip publishing to registry
    #[arg(long)]
    pub no_publish: bool,

    /// Skip git push (still commits and tags locally)
    #[arg(long)]
    pub no_push: bool,

    /// Skip GitHub release creation
    #[arg(long)]
    pub no_release: bool,

    /// Skip dependency diff
    #[arg(long)]
    pub no_deps: bool,

    /// Skip release statistics collection
    #[arg(long)]
    pub no_stats: bool,

    /// Skip release notes rendering (uses GitHub auto-generated notes)
    #[arg(long)]
    pub no_notes: bool,

    /// Skip running tests
    #[arg(long)]
    pub no_test: bool,

    /// Skip git tag creation (still commits and pushes)
    #[arg(long)]
    pub no_tag: bool,

    /// Skip entire git phase (commit, tag, push)
    #[arg(long)]
    pub no_git: bool,

    /// Create release as draft (overrides config)
    #[arg(long, conflicts_with = "no_draft")]
    pub draft: bool,

    /// Create release as published (overrides config)
    #[arg(long, conflicts_with = "draft")]
    pub no_draft: bool,

    /// Preview what would happen without making changes
    #[arg(long)]
    pub dry_run: bool,

    /// Skip confirmation prompt
    #[arg(long, short = 'y')]
    pub yes: bool,
}
```

**Step 2: Update the ShipOptions mapping in `cmd_ship`**

```rust
let draft_override = if args.draft {
    Some(true)
} else if args.no_draft {
    Some(false)
} else {
    None
};

let options = ShipOptions {
    explicit_version: args.version,
    no_changelog: args.no_changelog,
    no_publish: args.no_publish,
    no_push: args.no_push,
    no_release: args.no_release,
    no_deps: args.no_deps,
    no_stats: args.no_stats,
    no_notes: args.no_notes,
    dry_run: args.dry_run,
    no_test: args.no_test,
    no_tag: args.no_tag,
    no_git: args.no_git,
    draft_override,
};
```

**Step 3: Update `print_phase_summary` to include new phases**

```rust
fn print_phase_summary(options: &ShipOptions, config: &Config) {
    let phases: &[(&str, bool)] = &[
        ("test", !options.no_test),
        ("bump", true),
        ("publish", !options.no_publish),
        ("git", !options.no_git),
        ("release", !options.no_release),
    ];
    // ... rest unchanged
}
```

**Step 4: Run tests**

Run: `just test`
Expected: All pass

**Step 5: Commit**

```
feat(cli): rename --skip-tests to --no-test, add --no-tag/--no-git/--draft flags
```

---

### Task 7: Full check

**Step 1: Run full check suite**

Run: `just check`
Expected: fmt ok, clippy clean, deny clean, all tests pass, doc-tests pass

**Step 2: Fix any issues found by clippy or tests**

**Step 3: Commit any fixes**

---

### Task 8: Prepare for PR

**Step 1: Run `just check` one final time**

**Step 2: Create PR**

```
feat: configurable gh release and systematic --no-* flags (M4 #6)
```
