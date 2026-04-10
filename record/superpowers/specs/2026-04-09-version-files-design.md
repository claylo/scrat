# Version Files

**Date:** 2026-04-09
**Status:** Draft
**Scope:** Add `[[version_files]]` config for updating arbitrary files with the computed version during `scrat bump`

## Problem

Scrat's version bumping is hardcoded per ecosystem: `cargo set-version` for Rust, `composer.json` mutation for PHP, `pyproject.toml` for Python. Projects that carry version numbers in other files — Claude Code plugin manifests (`plugin.json`, `marketplace.json`), Agent Skill frontmatter (`SKILL.md`), plain `VERSION` files, or `package.json` sidecars in non-Node projects — have no way to keep those files in sync.

Today the only workaround is a `post_bump` hook running an external script. This works but has two problems: the script can't feed modified file paths back into `BumpOutcome`, so `PipelineContext.modified_files` is incomplete; and every project reinvents the same JSON/YAML/TOML mutation logic.

## Design

### Config model

New top-level array-of-tables in `.config/scrat.toml`:

```toml
[[version_files]]
path = ".claude-plugin/plugin.json"
format = "json"
field = "version"

[[version_files]]
path = ".claude-plugin/marketplace.json"
format = "json"
fields = ["metadata.version", "plugins.*.version"]

[[version_files]]
path = "package.json"
format = "json"
field = "version"

[[version_files]]
path = ".cursor-plugin/plugin.json"
format = "json"
field = "version"

[[version_files]]
path = "skills/*/SKILL.md"
format = "frontmatter"
field = "metadata.version"

[[version_files]]
path = "VERSION"
format = "text"
```

**Fields:**

| Field    | Type              | Required | Description |
|----------|-------------------|----------|-------------|
| `path`   | `String`          | yes      | Relative to project root. Supports globs (`*`, `**`). |
| `format` | `VersionFileFormat` | yes    | One of `json`, `toml`, `yaml`, `frontmatter`, `text`. |
| `field`  | `String`          | conditional | Dot-path to version value. Required for `json`, `toml`, `yaml`, `frontmatter`. Prohibited for `text`. |
| `fields` | `Vec<String>`     | conditional | Multiple dot-paths. Alternative to `field`. Same format constraints. |

`field` and `fields` are mutually exclusive. Specifying both is a config error.

### Dot-path syntax

Paths are dot-delimited. `*` matches all elements of an array.

| Path | Target |
|------|--------|
| `version` | Top-level `"version"` key |
| `metadata.version` | `"version"` nested under `"metadata"` |
| `plugins.*.version` | `"version"` in every element of the `"plugins"` array |

No deeper wildcard nesting (e.g., `*.*.version`) — one `*` per path is sufficient.

### Format details

**`json`** — Parse with `serde_json`. Preserve formatting by using `serde_json::to_string_pretty` with 2-space indent (matches standard JSON style). Version value is always written as a JSON string.

**`toml`** — Parse with `toml_edit` (preserves comments, formatting, order). Update the value at the dot-path in place. Version value is a TOML string.

**`yaml`** — Parse with `serde_yaml` (or the project's preferred YAML crate). Version value is a YAML string. Preserve existing quoting style if possible (quoted stays quoted, unquoted stays unquoted).

**`frontmatter`** — Split file at delimiters: `---` for YAML frontmatter, `+++` for TOML frontmatter. Parse only the frontmatter block. Update the field. Reassemble with the original delimiter and the markdown body untouched. Delimiter detection is by first line of the file.

**`text`** — Entire file content is the version string. Read, trim, replace, write back with trailing newline. No `field`/`fields` allowed.

### Execution placement

Inside `ReadyBump::execute()`, after the ecosystem-specific bumper, before changelog generation:

```
ReadyBump::execute()
  1. Ecosystem bumper (cargo set-version / composer.json / pyproject.toml)
  2. Version files bumper   <-- NEW
  3. Changelog generation (git-cliff)
  4. Return BumpOutcome { modified_files, ... }
```

Modified files from step 2 are appended to the same `modified_files` vec used by step 1. They flow into `BumpOutcome`, then into `PipelineContext`, and are available to hooks, release notes, and git commit staging.

### Behavior on missing fields

| Scenario | Behavior |
|----------|----------|
| Glob matches no files | Warning, not error. No files to update. |
| Explicit path doesn't exist | Error. Config points to a nonexistent file. |
| Field exists in file | Update it. Append file to `modified_files`. |
| Field missing in globbed file | Warning, skip that file. |
| Field missing in explicit file | Error. Config says to update a field that doesn't exist. |
| File can't be parsed | Error. Malformed file is a real problem. |
| `field` + `fields` both set | Config validation error at load time. |
| `field` set with `format = "text"` | Config validation error at load time. |

The distinction: explicit paths are promises ("this file has this field"), globs are queries ("update this field wherever it exists"). Promises that fail are errors; queries that find nothing are warnings.

### Dry-run support

`scrat bump --dry-run` already skips file writes. Version files respects this: parse and resolve paths, report what would change, but don't write. Output matches the existing dry-run format.

### JSON output

`scrat bump --json` includes version files in the output. The `modified_files` array already carries them. No new top-level fields needed.

## Implementation

### New types in `config.rs`

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct VersionFileConfig {
    pub path: String,
    pub format: VersionFileFormat,
    pub field: Option<String>,
    pub fields: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VersionFileFormat {
    Json,
    Toml,
    Yaml,
    Frontmatter,
    Text,
}
```

Added to `Config`:

```rust
pub struct Config {
    // ... existing fields ...
    pub version_files: Option<Vec<VersionFileConfig>>,
}
```

Config validation (at load time or pre-bump):
- `field` and `fields` are mutually exclusive
- `text` format rejects `field`/`fields`
- Non-`text` formats require exactly one of `field` or `fields`
- Dot-paths are syntactically valid (non-empty segments, at most one `*`)

### New module: `version_files.rs`

```rust
/// Update version in all configured version files.
/// Returns list of modified file paths (relative to root).
pub fn bump_version_files(
    root: &Utf8Path,
    configs: &[VersionFileConfig],
    new_version: &str,
    dry_run: bool,
    on_event: &dyn Fn(ShipEvent),
) -> BumpResult<Vec<String>>
```

Internal structure:

```rust
// Resolve a single VersionFileConfig to concrete file paths
fn resolve_paths(root: &Utf8Path, config: &VersionFileConfig) -> BumpResult<Vec<Utf8PathBuf>>

// Per-format updaters — each returns true if any field was modified
fn update_json(path: &Utf8Path, dot_paths: &[&str], version: &str) -> BumpResult<bool>
fn update_toml(path: &Utf8Path, dot_paths: &[&str], version: &str) -> BumpResult<bool>
fn update_yaml(path: &Utf8Path, dot_paths: &[&str], version: &str) -> BumpResult<bool>
fn update_frontmatter(path: &Utf8Path, dot_paths: &[&str], version: &str) -> BumpResult<bool>
fn update_text(path: &Utf8Path, version: &str) -> BumpResult<bool>

// Walk a parsed structure to find + replace at a dot-path
fn apply_dot_path(value: &mut serde_json::Value, path: &str, version: &str) -> bool
```

### Integration in `bump.rs`

In `ReadyBump::execute()`, after the ecosystem bump call and before changelog:

```rust
// Existing ecosystem bump
let mut modified = self.bump_ecosystem(&new_version)?;

// Version files
if let Some(ref vf_configs) = self.config.version_files {
    let vf_modified = version_files::bump_version_files(
        &self.root,
        vf_configs,
        &new_version.to_string(),
        self.dry_run,
        &on_event,
    )?;
    modified.extend(vf_modified);
}

// Changelog generation follows...
```

### Dependencies

Current scrat-core dependencies that can be reused:
- `serde_json` — already used for pipeline context and composer.json
- `toml_edit` — check if already present; if not, add for format-preserving TOML edits
- `glob` — for path resolution; check if already present

For YAML: use whatever YAML crate scrat already depends on (check Cargo.toml). Prefer `serde_saphyr` over `serde_yaml`.

For frontmatter: no crate needed. Split on `---`/`+++` delimiter lines, parse the middle, reassemble. ~30 lines of code.

## What this does NOT do

- **Create missing fields.** Version files only updates existing values. Adding `metadata.version` to a SKILL.md for the first time is a manual one-time edit.
- **Transform version formats.** Writes the semver string as-is. A file with `"1.0"` becomes `"1.1.0"` after bump.
- **Support conditional logic.** Every matched file gets the same version string.
- **Replace ecosystem bumpers.** Ecosystem-specific bumping runs first, version_files runs after. They're additive.

## Validation

After implementation:
- `scrat bump --dry-run` in a Claude plugin project with `version_files` config shows all files that would be modified
- `scrat bump` updates all configured files and reports them in `modified_files`
- `scrat bump --json` includes version file paths in the JSON output
- Glob patterns resolve correctly, missing-field warnings appear for globbed files
- Explicit path with missing file or missing field produces an error
- Frontmatter updates preserve the markdown body byte-for-byte
- Config validation catches `field` + `fields` together, `text` + `field`, etc.
