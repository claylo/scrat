---
audit: 2026-04-12-workspace-crates
last_updated: 2026-04-12
status:
  fixed: 7
  mitigated: 0
  accepted: 0
  disputed: 0
  deferred: 0
  open: 12
---

# Actions Taken: scrat 2026-04-12 Workspace Crates Audit

Remediation status for the [2026-04-12 workspace audit](README.md).

---

## 2026-04-12 — `scrat notes` command remediation

**Disposition:** fixed
**Addresses:**
[notes-from-flag-ignored-by-cliff-context](README.md#notes-from-flag-ignored-by-cliff-context) (significant),
[notes-command-skips-all-hooks](README.md#notes-command-skips-all-hooks) (advisory)
**Commit:** _(see PR linked from front matter once merged)_
**Author:** @claylo

Bundled fix on `fix/cased-audit-notes-command`. The two findings are causally
linked — the notes command's preview path diverged from the ship artifact in two
ways that a user would hit on their first use of `scrat notes`. Both are now
covered by unit tests (5 for arg construction, 4 for hook wiring), and the full
suite runs 594/594 green with clippy clean.

### notes-from-flag-ignored-by-cliff-context (significant)

`run_cliff_context` hardcoded `--unreleased --context` regardless of what
`preview_notes` resolved for `previous_tag`. Historical releases could not be
regenerated — git-cliff returned an empty context because `--unreleased` means
"since the latest tag," and the latest tag for an already-shipped release is
the release itself.

Extracted a pure `cliff_context_args(previous_tag, target_tag, target_tag_exists) -> Vec<String>`
that picks the right arg set per scenario:

| previous_tag | target_tag  | target_tag_exists | args                                               |
|--------------|-------------|-------------------|----------------------------------------------------|
| empty        | empty       | —                 | `--unreleased --context`                           |
| empty        | `v1.0.0`    | false             | `--unreleased --tag v1.0.0 --context`              |
| `v1.0.0`     | `v1.1.0`    | false             | `v1.0.0..HEAD --tag v1.1.0 --context`              |
| `v1.0.0`     | `v1.1.0`    | true              | `v1.0.0..v1.1.0 --context`                         |

`render_notes` queries `git::tag_exists(ctx.tag)` to pick the upcoming vs.
historical branch. End-to-end verified against the scrat repo itself:
`scrat notes --from v0.1.0 --version 0.1.1` now renders the v0.1.1 release
notes correctly from the `v0.1.0..v0.1.1` range.

### notes-command-skips-all-hooks (advisory)

`preview_notes` never called `hooks::run_hooks`, so any `filter:` hook a user
configured to shape release notes (inject quote-of-the-day, postcard path,
custom metadata) was silently skipped. Preview diverged from ship.

Added a private `apply_preview_hooks(ctx, hooks_config, project_root)` that
runs **only `filter:` hooks** from `post_bump`. Parallel and `sync:` hooks are
skipped deliberately:

- `filter:` hooks are pure JSON transformations of pipeline state — safe to
  run during read-only preview.
- Parallel and `sync:` hooks typically generate artifacts, upload files, or
  push state — firing them during preview would be an ugly side-effect
  surprise.

The decision was forced by an end-to-end test: the scrat repo's own
`scrat.toml` has a `post_bump` hook that invokes `claylo-graphics` to render
release postcards. Running it during preview failed (expected — it needs a
real release tag) but also wasn't what the user asked for. The filter-only
scoping hits the audit's intent ("match what ship's notes will render")
without the side-effect footgun.

Two new error variants added to `NotesError`:

- `HookExecution(#[from] HookError)` — propagates the underlying chain, which
  also partially addresses [notes-error-flattens-source-chain](README.md#notes-error-flattens-source-chain)
  at the hook seam (full remediation of that finding is a separate PR).
- `ContextSerde { direction, source }` — wraps `serde_json::Error` around the
  serialize-before-filter / deserialize-after-filter round trip.

### Verification

- `just test`: 594/594 passed (new: 5 `cliff_context_args` + 4 `apply_preview_hooks`)
- `just clippy`: 0 warnings
- `cargo fmt --check`: clean
- End-to-end smoke against scrat's own repo:
  - `scrat notes` (no `--from`): uses `v0.1.2..HEAD --tag vunreleased`
  - `scrat notes --from v0.1.0 --version 0.1.1`: uses `v0.1.0..v0.1.1` (historical)
  - `scrat notes --version 0.2.0`: uses `v0.1.2..HEAD --tag v0.2.0` (upcoming)

---

## 2026-04-12 — audit cleanup bundle

**Disposition:** fixed
**Addresses:**
[orphan-commands-build-and-clean-config](README.md#orphan-commands-build-and-clean-config) (advisory),
[orphan-release-changelog-tool-config](README.md#orphan-release-changelog-tool-config) (advisory),
[release-profile-missing-lto-and-strip](README.md#release-profile-missing-lto-and-strip) (advisory),
[bump-error-unsupported-ecosystem-dead-variant](README.md#bump-error-unsupported-ecosystem-dead-variant) (note),
[example-config-missing-filter-prefix-docs](README.md#example-config-missing-filter-prefix-docs) (note)
**Commit:** _(see PR linked from front matter once merged)_
**Author:** @claylo

Bundled trivial cleanup on `fix/audit-cleanup-bundle-a`. Five findings, all
independent, all trivial in isolation — one PR keeps the change-review cost
proportional to the work. Test suite runs 593/593 green (one bump-error
display test was removed alongside its variant), clippy clean.

### Config surface — deletion wins

Three documented config fields had no implementation behind them and no
roadmap item waiting. Deletion is the honest fix. Serde's default behavior
(no `deny_unknown_fields`) means removing the fields is a silent,
backwards-compatible change: if a user already has `[commands] build = "..."`
or `[release] changelog_tool = "git-cliff"` in their config, the field is
ignored after the removal just as it was ignored before.

- `CommandsConfig.build` and `CommandsConfig.clean` — removed. `test` and
  `publish` remain (both wired through `run_test_phase` / `run_publish_phase`).
- `ReleaseConfig.changelog_tool` — removed. `ChangelogTool` as a concept
  stays; it lives on `DetectedTools.changelog_tool`, derived from
  `version_strategy.changelog_tool()` per ecosystem driver.
- Updated three tests in `config.rs` that asserted on the deleted fields.
- Removed `build`/`clean`/`changelog_tool` references from
  `config/scrat.toml.example`, `config/scrat.yaml.example`,
  `docs/agent-guide.md`, `docs/getting-started.md`, and `README.md`.

### Dead variant

`BumpError::UnsupportedEcosystem(Ecosystem)` was constructed only by a
display-smoke unit test. Every ecosystem driver's `bump_version_files` now
returns real behavior or `Ok(Vec::new())` (Go, Swift, Generic). Variant
removed along with its test.

### Filter hook docs

Added a `filter:` paragraph to `config/scrat.toml.example` alongside the
existing `sync:` documentation. Now the canonical example config matches
README's framing of `filter:` as a first-class hook prefix.

### Release profile

Added `lto = "thin"`, `codegen-units = 1`, `strip = "symbols"` to
`[profile.release]`. Thin LTO enables cross-crate inlining between `scrat`
and `scrat_core`, which matters under scrat's thin-CLI architecture. The
`codegen-units = 1` setting maximizes inlining at the cost of release
build time — comment in the TOML notes this is the first knob to drop if
build time becomes noticeable. No benchmark run yet; `hyperfine --warmup 3
'scrat --version'` before/after is a follow-up measurement.

### Verification

- `just test`: 593/593 passed (one test removed alongside the
  `UnsupportedEcosystem` variant)
- `just clippy`: 0 warnings
- `cargo fmt --all`: clean

---
