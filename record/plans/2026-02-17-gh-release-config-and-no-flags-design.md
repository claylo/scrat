# Design: Configurable `gh release` + Systematic `--no-*` Flags

**Date:** 2026-02-17
**Status:** Approved
**Branch:** `feat/m4-release-config`

## Summary

Two changes that complete the M4 remaining work needed before scrat's first release:

1. Make `gh release create/edit` configurable (draft mode, title format, auto-detect edit-vs-create)
2. Make `--no-*` flags consistent and complete

## Config Model

New fields on `ReleaseConfig`:

```toml
[release]
github_release = true              # existing
assets = [...]                     # existing
notes_template = "..."             # existing
draft = true                       # NEW — default: true (create as draft)
title = "{tag}"                    # NEW — default: "{tag}", hook-style interpolation
discussion_category = ""           # NEW — optional, gh discussions integration
```

All `Option<T>` fields. Defaults applied at usage site, not in the struct.

## Release Phase: Edit-vs-Create

`run_release_phase()` gains auto-detection:

1. `gh release view {tag}` — check if release exists (exit code only)
2. **Exists:** `gh release edit {tag} --draft --notes-file ... --title ...`, then delete-and-reupload assets
3. **New:** `gh release create {tag} --draft --title ... --notes-file ... <assets>`
4. Both paths respect: `draft`, `title`, `notes_file`, `discussion_category`, `assets`

Extract a `ReleaseOptions` struct to pass into the function cleanly instead of bare args.

## `--no-*` Flag Changes

**Rename:**
- `--skip-tests` / `skip_tests` → `--no-test` / `no_test`

**Add:**
- `--no-tag` — skip tag creation (still commits + pushes)
- `--no-git` — skip entire git phase (commit + tag + push)
- `--draft` / `--no-draft` — CLI override for `release.draft` config

**Existing (unchanged):**
- `--no-changelog`, `--no-publish`, `--no-push`, `--no-release`
- `--no-deps`, `--no-stats`, `--no-notes`

**Precedence for draft:** CLI `--draft`/`--no-draft` > config `release.draft` > default (true).

## Dry-Run

Release phase dry-run shows:
- Create vs edit (would check `gh release view`)
- Title with interpolation applied
- Draft status
- Asset list

## Files Touched

| File | Changes |
|------|---------|
| `scrat-core/src/config.rs` | `draft`, `title`, `discussion_category` on `ReleaseConfig` |
| `scrat-core/src/ship.rs` | `ReleaseOptions` struct, rewrite `run_release_phase()`, rename `skip_tests` → `no_test`, add `no_tag`/`no_git` to `ShipOptions` |
| `scrat/src/commands/ship.rs` | Rename `--skip-tests` → `--no-test`, add `--no-tag`, `--no-git`, `--draft`/`--no-draft` |
| Tests in both crates | Update existing, add new for edit-vs-create logic |

No new modules. No new dependencies.
