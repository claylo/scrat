# Agent Briefing — Full workspace audit — crates/scrat (CLI) and crates/scrat Core (library)

You are in a `cased` audit output directory. This file exists to help you pick
up remediation work without thrashing. Read it once, then act.

**Audit:** `2026-04-12-Full workspace audit — crates/scrat (CLI) and crates/scrat-core (library)`
**Date:** 2026-04-12
**Findings:** 19 total

## Files in this directory

- `README.md`        — authored narrative report (markdown, GitHub-rendered companion to report.html). Read-only for remediation work.
- `report.html`      — interactive rendered report (primary deliverable). Read-only.
- `findings.yaml`    — structured findings (source for the build). Read-only.
- `recon.yaml`       — structural model. Read-only.
- `assets/`          — generated sparkline SVGs. Don't edit.
- `actions-taken.md` — append-only remediation ledger. May not exist yet;
  create it the first time you log an action.
- `AGENTS.md`        — this file.

## The loop

For each finding you address:

1. Find it in `README.md` or `report.html` by its slug. Anchors match the slug
   exactly; every finding is pre-listed in the index below so you don't need
   to grep.
2. Read the concern, location, and remediation text.
3. Make the code change in the target repository.
4. Append one entry to `actions-taken.md`. **One entry per action**, even
   when a single action resolves multiple findings — put every slug it
   addresses in the `Addresses` field.

## `actions-taken.md` format

YAML front matter plus chronological markdown entries. Front matter is
mandatory; update `last_updated` and the `status` counts every time you
add an entry. The `open` count is `19 - (fixed + mitigated +
accepted + disputed + deferred)`.

```markdown
---
audit: 2026-04-12-Full workspace audit — crates/scrat (CLI) and crates/scrat-core (library)
last_updated: YYYY-MM-DD
status:
  fixed: 0
  mitigated: 0
  accepted: 0
  disputed: 0
  deferred: 0
  open: 19
---

# Actions Taken: Full workspace audit — crates/scrat (CLI) and crates/scrat Core (library)

Summary of remediation status for the [2026-04-12 Full workspace audit — crates/scrat (CLI) and crates/scrat-core (library) audit](README.md).

---

## YYYY-MM-DD — brief description of the action

**Disposition:** fixed
**Addresses:** [finding-slug](README.md#finding-slug)
**Commit:** {SHA or PR link}
**Author:** {who did the work}

One to three paragraphs describing what changed, in which files, and why
this approach. If the disposition is `accepted` or `disputed`, the rationale
must be here. If `deferred`, include the target date or milestone.
```

## Dispositions

- `fixed` — code change deployed; commit SHA or PR link required
- `mitigated` — compensating control in place; root cause remains; explain
  the residual risk
- `accepted` — risk acknowledged; rationale mandatory (who decided, why).
  This is not a euphemism for "ignored"
- `disputed` — finding contested with evidence; not a dismissal. The
  original finding stays in `README.md`; this entry records the counterargument
- `deferred` — scheduled for later; target date or milestone reference
  required. A deferred finding without a target is an accepted finding in
  disguise

## What you must not do

- Do not edit `README.md`, `report.html`, `findings.yaml`, `recon.yaml`, or
  anything in `assets/`. They are the audit artifact and must stay immutable.
- Do not edit past `actions-taken.md` entries. The file is append-only. If
  a previous action is superseded, add a new entry referencing the old one.
- Do not invent finding slugs. Use the ones in the index below, verbatim.
- Do not create an empty `actions-taken.md` until you have at least one
  action to log.

## Finding index

Every finding in this audit. Use these exact slugs in the `Addresses` field
of your `actions-taken.md` entries.

### The Error Architecture Surface

- `notes-error-flattens-source-chain` (moderate) — `crates/scrat-core/src/notes.rs:22-41`
- `observability-returns-anyhow-in-library` (moderate) — `crates/scrat-core/src/observability.rs:67-70`
- `observability-writer-silent-discard` (advisory) — `crates/scrat-core/src/observability.rs:196-202`
- `expect-messages-describe-value-not-invariant` (advisory) — `crates/scrat/src/commands/doctor.rs:112-116`

### The Supply Chain Surface

- `scrat-core-depends-on-clap-for-value-enum-derives` (moderate) — `crates/scrat-core/Cargo.toml:33`
- `serde-saphyr-caret-on-zero-zero-x` (advisory) — `crates/scrat-core/Cargo.toml:36`
- `ci-lacks-yanked-and-unmaintained-hardening` (advisory) — `.config/deny.toml:48-55`
- `owo-colors-pulls-duplicate-supports-color` (note) — `crates/scrat/Cargo.toml:49`
- `transitive-getrandom-triplicate` (note) — `Cargo.lock:477-513`

### The Pipeline Efficiency Surface

- `redundant-git-current-branch-per-ship` (advisory) — `crates/scrat-core/src/preflight.rs:188-198`
- `release-profile-missing-lto-and-strip` (advisory) — `Cargo.toml:57-59`
- `has-binary-path-probe-not-cached` (note) — `crates/scrat-core/src/detect.rs:131-134`

### The Feature Completeness Surface

- `notes-from-flag-ignored-by-cliff-context` (significant) — `crates/scrat-core/src/notes.rs:347-371`
- `notes-command-skips-all-hooks` (advisory) — `crates/scrat-core/src/notes.rs:158-175`
- `orphan-commands-build-and-clean-config` (advisory) — `crates/scrat-core/src/config.rs:105-115`
- `orphan-release-changelog-tool-config` (advisory) — `crates/scrat-core/src/config.rs:117-121`
- `example-config-advertises-unimplemented-otel-and-env` (note) — `config/scrat.toml.example:26-36`
- `bump-error-unsupported-ecosystem-dead-variant` (note) — `crates/scrat-core/src/bump.rs:43-49`
- `example-config-missing-filter-prefix-docs` (note) — `config/scrat.toml.example:82-92`

## If you have the `cased` skill loaded

Invoke it. The skill's Phase 5 covers remediation tracking with the full
schema reference and worked examples. This briefing exists for the case
where you land in the directory without the skill available.
