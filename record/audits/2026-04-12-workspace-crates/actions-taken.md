---
audit: 2026-04-12-workspace-crates
last_updated: 2026-04-12
status:
  fixed: 2
  mitigated: 0
  accepted: 0
  disputed: 0
  deferred: 0
  open: 17
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
