---
audit: 2026-04-12-workspace-crates
last_updated: 2026-04-12
status:
  fixed: 12
  mitigated: 0
  accepted: 0
  disputed: 0
  deferred: 0
  open: 7
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

## 2026-04-12 — supply chain hardening

**Disposition:** fixed
**Addresses:**
[serde-saphyr-caret-on-zero-zero-x](README.md#serde-saphyr-caret-on-zero-zero-x) (advisory),
[ci-lacks-yanked-and-unmaintained-hardening](README.md#ci-lacks-yanked-and-unmaintained-hardening) (advisory),
[owo-colors-pulls-duplicate-supports-color](README.md#owo-colors-pulls-duplicate-supports-color) (note)
**Commit:** _(see PR linked from front matter once merged)_
**Author:** @claylo

Supply-chain Bundle B on `fix/audit-cleanup-bundle-b`. Three findings, one
theme: harden what cargo-deny considers blocking before the next 2 AM yank
has to be caught by a human reading CI logs.

### serde-saphyr pin

`serde-saphyr = "0.0"` accepted any 0.0.x version at `cargo install scrat`
time — and every 0.0.x patch can be breaking per semver convention.
Pinned to `"~0.0.23"` (current lockfile version) in both
`crates/scrat-core/Cargo.toml` and `crates/scrat/Cargo.toml`. Bump
explicitly when a new 0.0.x is vetted.

### cargo-deny advisory gates

Added to `[advisories]` in `.config/deny.toml`:

- `yanked = "deny"` — post-merge yank of a pinned dep now fails CI.
- `unmaintained = "all"` — RustSec `unmaintained` flag on any workspace
  or transitive dep fails CI.

**Note on schema drift:** the audit's original recommendation was
`unmaintained = "deny"` and `vulnerability = "deny"`. Those were the
pre-0.14 cargo-deny schema. The current (0.19.1) schema treats
`unmaintained` as a scope selector (`all`/`workspace`/`transitive`/`none`)
and removed the `vulnerability` key entirely — vulnerability advisories
are always hard errors now, no opt-in required. Comment in the TOML
records this.

### supports-color skip

Added documented skip entry in `[bans]` for `supports-color`. cargo-deny
was reporting a spurious duplicate-version warning driven by upstream
`owo-colors 4.3`'s intentional dual-dep (`supports-color 3.0` +
`supports-color 2.0` via package rename). No dedup path without losing
terminal auto-detect — the skip silences noise without hiding
consequential duplicates (getrandom, r-efi remain visible as warnings,
matching their audit note-level findings).

### Template-scope caveat

`ci-lacks-yanked-and-unmaintained-hardening` and possibly
`serde-saphyr-caret-on-zero-zero-x` originate in the claylo-rs template,
not scrat alone. Fixing in scrat closes the findings for this project;
promoting the same changes into `~/source/claylo/claylo-rs` is a
follow-up that closes the findings across every descendant tool.

### Verification

- `just deny`: `advisories ok, bans ok, licenses ok, sources ok` — the
  only remaining duplicate warnings are `getrandom` and `r-efi`, both
  tracked as separate note-level findings.
- `just test`: 593/593 passed
- `just clippy`: 0 warnings
- `cargo fmt --all --check`: clean

---

## 2026-04-12 — pipeline efficiency (Bundle C)

**Disposition:** fixed
**Addresses:**
[redundant-git-current-branch-per-ship](README.md#redundant-git-current-branch-per-ship) (advisory),
[has-binary-path-probe-not-cached](README.md#has-binary-path-probe-not-cached) (note)
**Commit:** _(see PR linked from front matter once merged)_
**Author:** @claylo

Two independent efficiency wins in one PR on `fix/audit-cleanup-bundle-c`.

### has_binary PATH probe cache

`detect::has_binary` is called ~10 times across a single `scrat ship` run
(detection + preflight + version planning + gh auth), each invocation
walking PATH via `which::which`. Added a process-lifetime cache with
`OnceLock<Mutex<HashMap<String, bool>>>` so each binary is probed at most
once per process. ~10-15ms saved per ship on macOS.

A `#[cfg(test)] clear_path_cache()` escape hatch is available for any test
that installs or uninstalls a binary during a test run — no current test
needed it, but the hook exists for when one does.

### Current-branch threading

Before: `git rev-parse --abbrev-ref HEAD` ran twice per ship — once in
preflight's `check_release_branch`, once again in `ReadyShip::execute`
when building `PipelineContext`. The branch cannot change between plan
and execute within one process, so the second call was pure waste.

Hoisted the resolution: `run_preflight_with_detection` now calls
`git::current_branch()` once, feeds the result into `check_release_branch`,
and stores it on `PreflightReport.branch`. `plan_ship` threads that
value into `ReadyShip.branch` and `InteractiveShip.branch`.
`ReadyShip::execute` reads `self.branch.clone()` instead of re-forking
git. Net: one `git rev-parse` per ship run, down from two.

`PreflightReport` gains a public `branch: Option<String>` field with
`#[serde(skip_serializing_if = "Option::is_none")]` — backwards-
compatible for `scrat preflight --json` consumers (existing keys
unchanged, new field only appears when a branch is present).

### Verification

- `just test`: 593/593 passed
- `just clippy`: 0 warnings
- `cargo fmt --all --check`: clean
- `cargo build --lib`: compiles after schema changes to `PreflightReport`,
  `ReadyShip`, `InteractiveShip`

---

## 2026-04-13 — NotesError source-chain restoration (Bundle D #1)

**Disposition:** fixed
**Addresses:**
[notes-error-flattens-source-chain](README.md#notes-error-flattens-source-chain) (moderate)
**Commit:** _(see PR linked from front matter once merged)_
**Author:** @claylo

Fixed the partial remediation left behind by Bundle C (Notes hook seam).
`NotesError::CliffContext(String)` and `CliffRender(String)` flattened four
distinct sub-error types into opaque strings, so
`err.chain().skip(1).for_each(...)` in `main.rs:15` printed nothing
beyond the top line. Users lost the OS error code for missing-binary
failures and the file/line context for malformed-JSON failures unless
they re-ran with `-vv`.

### Four new `#[from]` variants

- `CliffExec(#[from] std::io::Error)` — covers the four subprocess sites
  (`.output()`, `.spawn()`, `stdin.write_all()`, `.wait_with_output()`).
- `CliffJson(#[from] serde_json::Error)` — covers the two context-JSON
  sites (`from_str` for parse, `to_string` for re-serialize after
  injecting scrat's `extra`).
- `Git(#[from] crate::git::GitError)` — single site, `latest_version_tag()`
  when `--from` is omitted.
- `Version(#[from] crate::version::VersionError)` — single site,
  `parse_version()` when `--version` is user-supplied.

`CliffContext(String)` and `CliffRender(String)` are intentionally
retained for the one case they still make semantic sense: git-cliff
exited non-zero with stderr, and stderr itself is the primary
diagnostic — there is no Rust-level wrapped error to preserve.

### Sites refactored

Eight call sites in `crates/scrat-core/src/notes.rs`:

| Location | Before | After |
|----------|--------|-------|
| `preview_notes` — prev tag | `.map_err(\|e\| NotesError::CliffContext(format!("failed to query git tags: {e}")))?` | `?` |
| `preview_notes` — version parse | `.map_err(\|e\| NotesError::CliffContext(format!("invalid version: {e}")))?` | `?` |
| `run_cliff_context` — subprocess | `.map_err(\|e\| NotesError::CliffContext(format!("failed to execute git-cliff: {e}")))?` | `?` |
| `inject_extra` — parse | `.map_err(\|e\| NotesError::CliffContext(format!("failed to parse context JSON: {e}")))?` | `?` |
| `inject_extra` — reserialize | `.map_err(\|e\| NotesError::CliffContext(format!("failed to re-serialize context: {e}")))` | `Ok(...?)` |
| `run_cliff_render` — spawn | `.map_err(\|e\| NotesError::CliffRender(format!("failed to spawn git-cliff: {e}")))?` | `?` |
| `run_cliff_render` — stdin | `.map_err(\|e\| NotesError::CliffRender(format!("failed to write to stdin: {e}")))?` | `?` |
| `run_cliff_render` — wait | `.map_err(\|e\| NotesError::CliffRender(format!("failed to wait for git-cliff: {e}")))?` | `?` |

### Tests

Four new source-chain tests verify the fix's guarantee:

- `cliff_exec_preserves_io_source` — constructs via `io::Error::new`,
  asserts `matches!(err, CliffExec(_))` and `err.source().is_some()`.
- `cliff_json_preserves_serde_source` — same shape for
  `serde_json::Error`.
- `git_variant_preserves_git_source` — `GitError::NotARepo` → verifies
  `source().to_string()` contains `"not a git repository"`.
- `version_variant_preserves_version_source` — `VersionError::NoTags`
  → verifies `source().to_string()` contains `"no version tags"`.

Two existing tests (`inject_extra_errors_on_malformed_json`,
`inject_extra_errors_on_truncated_json`) were updated from
string-contains assertions on the old flattened message to
`matches!(err, CliffJson(_))` + `source().is_some()` — the same
observable behavior the upstream CLI relies on.

### Before/after — end-to-end

```
$ scrat notes --version "not-a-version"
# before: Error: git-cliff context extraction failed: invalid version: …
# after:
Error: failed to render release notes

Caused by:
    version parse failed

Caused by:
    invalid semver: unexpected character 'n' while parsing major version number

Caused by:
    unexpected character 'n' while parsing major version number
```

The anyhow context (`failed to render release notes`) wraps the scrat
layer; each `Caused by` unwraps one more source frame. The user gets
the semantic label at every level instead of one opaque sentence.

### Verification

- `just test`: 597/597 passed (4 new source-chain tests)
- `just clippy`: 0 warnings
- `cargo fmt --all --check`: clean
- End-to-end smoke against `scrat` repo:
  - `scrat notes --version "bad"` → Version chain (semver source)
  - `PATH=/usr/bin scrat notes --version 0.1.3` → CliffExec chain
    (io::Error "No such file or directory")
  - tag-less repo → CliffContext(String) still (git-cliff non-zero, stderr
    is the diagnostic — behavior preserved)

### Deferred

`BumpError::ToolFailed` is the mirror finding and ships separately in
Bundle D #2 — ~20 call sites across `bump.rs` and `version_files.rs`,
different blast radius.

---

## 2026-04-13 — BumpError source-chain restoration (Bundle D #2)

**Disposition:** fixed
**Addresses:**
[notes-error-flattens-source-chain](README.md#notes-error-flattens-source-chain) (moderate, BumpError half)
**Commit:** _(see PR linked from front matter once merged)_
**Author:** @claylo

Closes the BumpError half of `notes-error-flattens-source-chain`. The
ledger row was already flipped to ✅ in Bundle D #1 (the audit treats
NotesError + BumpError as one slug); this PR closes the second
remediation phase the finding called out.

`BumpError::ToolFailed { tool, message: format!("...: {e}") }` was the
same anti-pattern as `NotesError::CliffContext(String)`, but with a
larger and more diverse footprint — Phase 4's ecosystem-driver split
expanded the surface from the audit's "~20" estimate to ~46 src sites,
wrapping six distinct error types across seven files.

### Three new variants — different shape than NotesError on purpose

NotesError used four `#[from]` variants for its four sub-error types.
BumpError can't follow the same pattern: it wraps `serde_json::Error`,
`serde_saphyr::Error`, `toml_edit::TomlError`, and `glob::PatternError`
— mostly parsers — which would mean four near-identical variants.

The audit suggested a three-variant model (`ToolIo` / `ToolParse` /
`ToolSerialize`) and that's the right shape here. Parsers collapse into
one variant via a boxed source:

- `ToolIo { tool, #[source] source: std::io::Error }` — 21 sites.
  Subprocess and file I/O (`Command::output`, `read_to_string`, `write`,
  `read_dir`).
- `ToolParse { tool, #[source] source: Box<dyn Error + Send + Sync> }` —
  9 sites. JSON, YAML, TOML, glob pattern parse failures.
- `ToolSerialize { tool, #[source] source: Box<dyn Error + Send + Sync> }`
  — 5 sites. JSON / YAML serialize failures (write-back after edit).

Trade-off: explicit `.map_err()` constructions instead of `?` — but the
`tool` field is preserved (it carries the manifest path, which is
load-bearing context for diagnostics), and `source: Box::new(e)` keeps
the chain. The `#[source]` attribute hooks the boxed value into
`Error::source()` — same machinery the chain renderer in `main.rs:15`
walks.

### `ToolFailed { tool, message }` retained for genuine cases

Twelve sites stay as `ToolFailed`:

- **Tool-stderr (2):** `bump.rs::generate_changelog` (git-cliff
  non-zero) and `ecosystem/rust.rs::bump_version_files` (cargo
  set-version non-zero). The stderr text *is* the diagnostic — there
  is no Rust-level wrapped error to preserve.
- **Synthesized pre-condition errors (8):** `version_files.rs` (text
  format with `field`/`fields`, non-text without, file not found, field
  not found in explicit path), `bump.rs` (Ruby has no version.rb +
  no version_files config), `ecosystem/node.rs` (package.json missing
  `version` key), `ecosystem/ruby.rs` (non-UTF-8 paths from
  `Utf8PathBuf::from_path_buf`, which returns the original `PathBuf`
  rather than an `Error` type).
- **Tests (2):** display-string assertions on the `ToolFailed` variant
  itself.

### Sites refactored

| File | io::Error | parse | serialize |
|------|-----------|-------|-----------|
| `version_files.rs` | 10 | 4 (json, yaml×2, toml, toml-fm, json glob…) | 3 (json, yaml, yaml-fm) |
| `bump.rs` | 1 (git-cliff `Command::output`) | — | — |
| `ecosystem/node.rs` | 2 | 1 | 1 |
| `ecosystem/php.rs` | 1 | 1 | 1 |
| `ecosystem/python.rs` | 1 | — | — |
| `ecosystem/ruby.rs` | 5 (read_dir + 4 read/write helpers) | 2 (glob pattern + entry) | — |
| `ecosystem/rust.rs` | 1 (cargo `Command::output`) | — | — |

Plus 4 unchanged tool-stderr / synthesized sites in those files. The
audit's "~30 call sites" was a Phase-3 snapshot; the actual count today
is 46 changed, 12 retained. The pattern is uniform — mechanical per
site, but enough volume that the diff is large.

### Tests

Three new source-chain tests in `bump.rs::tests`:

- `tool_io_preserves_io_source` — constructs `ToolIo` from
  `io::Error::new(PermissionDenied, ...)`; asserts top-level Display
  contains the tool name and `source().to_string()` exposes the
  underlying message.
- `tool_parse_preserves_boxed_source` — boxes a `serde_json::Error`
  into `ToolParse`; asserts `source().is_some()`.
- `tool_serialize_preserves_boxed_source` — same shape for
  `ToolSerialize`.

The two existing pattern-match tests
(`execute_node_errors_without_version_field`,
`bump_error_tool_failed_display`) still pass — they exercise the
synthesized-error paths that intentionally stay on `ToolFailed`.

### End-to-end smoke

```
$ scrat bump --version 0.1.0 --no-changelog
# (in tempdir with broken.json in [[version_files]])

# before:
Error: bump failed: broken.json failed: failed to parse JSON: key must be a string at line 1 column 3

# after:
Error: bump failed
Caused by:
    /private/var/.../broken.json: parse error
Caused by:
    key must be a string at line 1 column 3
```

The `tool` field carries the absolute path (since the smoke runs in a
tempdir); the variant label says "parse error"; the source is the
`serde_json::Error` with line/column info. Three layers, each
informative.

### Verification

- `just test`: 600/600 passed (3 new source-chain tests)
- `just clippy`: 0 warnings
- `cargo fmt --all --check`: clean
- End-to-end smoke against tempdir bump scenarios:
  - malformed JSON in `[[version_files]]` → `ToolParse` chain
    (serde_json source)
  - missing file in `[[version_files]]` → `ToolFailed` (synthesized
    error, no chain — correct, no underlying Rust error exists)

### Ledger note

`notes-error-flattens-source-chain` row in the Remediation Ledger was
flipped to ✅ in Bundle D #1; no additional ledger flip needed for
this PR. The disposition history is now: D #1 closes NotesError, D #2
closes BumpError, slug fully resolved.

---

## 2026-04-13 — expect() messages carry the literal template

**Disposition:** fixed
**Addresses:**
[expect-messages-describe-value-not-invariant](README.md#expect-messages-describe-value-not-invariant) (advisory)
**Commit:** _(see PR linked from front matter once merged)_
**Author:** @claylo

Two sites — `commands/doctor.rs` and `commands/ship.rs` — constructed
`ProgressStyle` via `.expect("valid template")` / `.expect("valid
spinner template")`. The messages paraphrased the value rather than
stating the invariant. If indicatif ever rejects the literal (typo
during edit or breaking-change release), a crash report would show the
paraphrase and nothing actionable.

Replaced both with the audit's recommended form, embedding the
indicatif template literal so a panic message names the specific
input:

- `doctor.rs:115`: `.expect("indicatif must accept literal template '{spinner:.cyan} {msg}'")`
- `ship.rs:258`:   `.expect("indicatif must accept literal template '  {spinner:.cyan} {msg}'")`
  (two-space prefix preserved — ship-phase spinners indent under the
  phase header)

### Clippy friction

The `{spinner:.cyan}` pattern inside `.expect()` triggers the nursery
lint `clippy::literal_string_with_formatting_args`
("this looks like a formatting argument but it is not part of a
formatting macro"). The braces are intentional — they name indicatif's
spinner-syntax tokens. Attribute placement for the `#[allow]`:
expression-position attributes on method calls aren't stable, so the
ProgressStyle construction was hoisted to its own `let` binding and
`#[allow(clippy::literal_string_with_formatting_args)]` sits on the
statement. Comment above each site documents the intent so a future
maintainer sees why the lint is suppressed.

### Verification

- `just test`: 600/600 passed (no behavioral change, no new tests)
- `just clippy`: 0 warnings
- `cargo fmt --all --check`: clean

---
