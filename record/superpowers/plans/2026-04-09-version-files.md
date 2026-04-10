# Version Files Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `[[version_files]]` config to scrat so `scrat bump` can update version numbers in arbitrary JSON, TOML, YAML, frontmatter, and text files alongside ecosystem-specific bumping.

**Architecture:** New `version_files` config section parsed into `Vec<VersionFileConfig>`, stored on `ReadyBump` during planning, executed after ecosystem bump in `ReadyBump::execute()`. A new `version_files.rs` module handles glob resolution, dot-path navigation, and per-format file mutation. Modified file paths flow into the existing `BumpOutcome.modified_files` vec.

**Tech Stack:** Rust, serde, serde_json (JSON), toml_edit (TOML, format-preserving), serde-saphyr (YAML), glob (path resolution)

**Spec:** `record/superpowers/specs/2026-04-09-version-files-design.md`

---

### Task 1: Add dependencies and config types

**Files:**
- Modify: `crates/scrat-core/Cargo.toml:30-45`
- Modify: `crates/scrat-core/src/config.rs:50-69`

- [ ] **Step 1: Write config deserialization test**

In `crates/scrat-core/src/config.rs`, add to the existing `#[cfg(test)] mod tests` block (after line 544):

```rust
#[test]
fn test_version_files_config_deserializes() {
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("config.toml");
    fs::write(
        &config_path,
        r#"
[[version_files]]
path = ".claude-plugin/plugin.json"
format = "json"
field = "version"

[[version_files]]
path = ".claude-plugin/marketplace.json"
format = "json"
fields = ["metadata.version", "plugins.*.version"]

[[version_files]]
path = "VERSION"
format = "text"
"#,
    )
    .unwrap();

    let config_path = Utf8PathBuf::try_from(config_path).unwrap();
    let (config, _) = ConfigLoader::new()
        .with_user_config(false)
        .with_file(&config_path)
        .load()
        .unwrap();

    let vf = config.version_files.unwrap();
    assert_eq!(vf.len(), 3);
    assert_eq!(vf[0].path, ".claude-plugin/plugin.json");
    assert!(matches!(vf[0].format, VersionFileFormat::Json));
    assert_eq!(vf[0].field.as_deref(), Some("version"));
    assert!(vf[0].fields.is_none());

    assert_eq!(vf[1].fields.as_ref().unwrap().len(), 2);
    assert!(vf[1].field.is_none());

    assert!(matches!(vf[2].format, VersionFileFormat::Text));
    assert!(vf[2].field.is_none());
    assert!(vf[2].fields.is_none());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p scrat-core test_version_files_config_deserializes 2>&1 | tail -10`
Expected: FAIL — `VersionFileConfig` and `VersionFileFormat` don't exist yet.

- [ ] **Step 3: Add dependencies to Cargo.toml**

In `crates/scrat-core/Cargo.toml`, add to `[dependencies]` (after the existing entries):

```toml
glob = "0.3"
toml_edit = "0.22"
```

- [ ] **Step 4: Add config types to config.rs**

In `crates/scrat-core/src/config.rs`, add after `ShipConfig` (after line ~230):

```rust
/// Configuration for a version file to update during bump.
///
/// Each entry describes a file (or glob pattern) containing a version string
/// that scrat should update when bumping the project version.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct VersionFileConfig {
    /// File path relative to project root. Supports globs (`*`, `**`).
    pub path: String,
    /// File format — determines how the file is parsed and updated.
    pub format: VersionFileFormat,
    /// Dot-path to the version field (e.g., `"version"`, `"metadata.version"`).
    /// Mutually exclusive with `fields`.
    pub field: Option<String>,
    /// Multiple dot-paths to update in one file.
    /// Mutually exclusive with `field`.
    pub fields: Option<Vec<String>>,
}

/// Supported file formats for version files.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum VersionFileFormat {
    /// JSON file — parsed with serde_json, written with 2-space pretty-print.
    Json,
    /// TOML file — parsed with toml_edit for format-preserving updates.
    Toml,
    /// YAML file — parsed with serde-saphyr.
    Yaml,
    /// Markdown with YAML (`---`) or TOML (`+++`) frontmatter.
    Frontmatter,
    /// Plain text file — entire content is the version string.
    Text,
}
```

- [ ] **Step 5: Add `version_files` field to `Config` struct**

In `crates/scrat-core/src/config.rs`, add to the `Config` struct (after `ship` field, line ~68):

```rust
    /// Version files to update during bump (in addition to ecosystem files).
    pub version_files: Option<Vec<VersionFileConfig>>,
```

- [ ] **Step 6: Run test to verify it passes**

Run: `cargo nextest run -p scrat-core test_version_files_config_deserializes 2>&1 | tail -10`
Expected: PASS

- [ ] **Step 7: Run all existing tests to confirm no regressions**

Run: `cargo nextest run -p scrat-core 2>&1 | tail -5`
Expected: All existing tests pass. The new `Option<Vec<..>>` field defaults to `None` via `#[serde(default)]`.

- [ ] **Step 8: Commit**

```
feat(config): add version_files config types

VersionFileConfig and VersionFileFormat support json, toml, yaml,
frontmatter, and text formats with dot-path field navigation.
```

---

### Task 2: Dot-path parser

**Files:**
- Create: `crates/scrat-core/src/version_files.rs`
- Modify: `crates/scrat-core/src/lib.rs:38-68`

- [ ] **Step 1: Register the new module**

In `crates/scrat-core/src/lib.rs`, add after the `pub mod version;` line:

```rust
pub mod version_files;
```

- [ ] **Step 2: Write dot-path parser tests**

Create `crates/scrat-core/src/version_files.rs`:

```rust
//! Version file updater for arbitrary JSON, TOML, YAML, frontmatter, and text files.
//!
//! Called by [`crate::bump::ReadyBump::execute`] after ecosystem-specific bumping.
//! Resolves glob patterns, navigates dot-paths, and updates version values in place.

use std::path::Path;

use camino::{Utf8Path, Utf8PathBuf};
use tracing::{debug, warn};

use crate::bump::BumpResult;
use crate::config::{VersionFileConfig, VersionFileFormat};

// ──────────────────────────────────────────────
// Dot-path parsing
// ──────────────────────────────────────────────

/// A segment in a dot-path like `"metadata.version"` or `"plugins.*.version"`.
#[derive(Debug, Clone, PartialEq, Eq)]
enum DotSegment {
    /// A named key (e.g., `"metadata"`).
    Key(String),
    /// Wildcard — iterate all elements of an array/sequence.
    Wildcard,
}

/// Parse a dot-path string into segments.
///
/// `"metadata.version"` -> `[Key("metadata"), Key("version")]`
/// `"plugins.*.version"` -> `[Key("plugins"), Wildcard, Key("version")]`
fn parse_dot_path(path: &str) -> Vec<DotSegment> {
    path.split('.')
        .map(|s| {
            if s == "*" {
                DotSegment::Wildcard
            } else {
                DotSegment::Key(s.to_owned())
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_key() {
        assert_eq!(parse_dot_path("version"), vec![DotSegment::Key("version".into())]);
    }

    #[test]
    fn parse_nested_path() {
        assert_eq!(
            parse_dot_path("metadata.version"),
            vec![DotSegment::Key("metadata".into()), DotSegment::Key("version".into())]
        );
    }

    #[test]
    fn parse_wildcard_path() {
        assert_eq!(
            parse_dot_path("plugins.*.version"),
            vec![
                DotSegment::Key("plugins".into()),
                DotSegment::Wildcard,
                DotSegment::Key("version".into()),
            ]
        );
    }
}
```

- [ ] **Step 3: Run tests to verify they pass**

Run: `cargo nextest run -p scrat-core parse_dot_path 2>&1 | tail -10`
Expected: 3 tests PASS

- [ ] **Step 4: Commit**

```
feat(version-files): add dot-path parser

Parses "metadata.version" and "plugins.*.version" into navigable
segments for use by format-specific updaters.
```

---

### Task 3: JSON updater

**Files:**
- Modify: `crates/scrat-core/src/version_files.rs`

- [ ] **Step 1: Write JSON updater tests**

Add to the `tests` module in `version_files.rs`:

```rust
use std::fs;
use tempfile::TempDir;

#[test]
fn json_update_top_level_field() {
    let tmp = TempDir::new().unwrap();
    let path = Utf8PathBuf::try_from(tmp.path().join("plugin.json")).unwrap();
    fs::write(&path, r#"{"name": "test", "version": "1.0.0"}"#).unwrap();

    let result = update_json(&path, &["version"], "2.0.0").unwrap();
    assert!(result);

    let content: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(content["version"], "2.0.0");
    assert_eq!(content["name"], "test");
}

#[test]
fn json_update_nested_field() {
    let tmp = TempDir::new().unwrap();
    let path = Utf8PathBuf::try_from(tmp.path().join("marketplace.json")).unwrap();
    fs::write(
        &path,
        r#"{"metadata": {"version": "1.0.0", "desc": "test"}}"#,
    )
    .unwrap();

    let result = update_json(&path, &["metadata.version"], "2.0.0").unwrap();
    assert!(result);

    let content: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(content["metadata"]["version"], "2.0.0");
    assert_eq!(content["metadata"]["desc"], "test");
}

#[test]
fn json_update_wildcard_array() {
    let tmp = TempDir::new().unwrap();
    let path = Utf8PathBuf::try_from(tmp.path().join("marketplace.json")).unwrap();
    fs::write(
        &path,
        r#"{"plugins": [{"name": "a", "version": "1.0.0"}, {"name": "b", "version": "1.0.0"}]}"#,
    )
    .unwrap();

    let result = update_json(&path, &["plugins.*.version"], "2.0.0").unwrap();
    assert!(result);

    let content: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(content["plugins"][0]["version"], "2.0.0");
    assert_eq!(content["plugins"][1]["version"], "2.0.0");
}

#[test]
fn json_update_multiple_fields() {
    let tmp = TempDir::new().unwrap();
    let path = Utf8PathBuf::try_from(tmp.path().join("marketplace.json")).unwrap();
    fs::write(
        &path,
        r#"{"metadata": {"version": "1.0.0"}, "plugins": [{"version": "1.0.0"}]}"#,
    )
    .unwrap();

    let result =
        update_json(&path, &["metadata.version", "plugins.*.version"], "2.0.0").unwrap();
    assert!(result);

    let content: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(content["metadata"]["version"], "2.0.0");
    assert_eq!(content["plugins"][0]["version"], "2.0.0");
}

#[test]
fn json_missing_field_returns_false() {
    let tmp = TempDir::new().unwrap();
    let path = Utf8PathBuf::try_from(tmp.path().join("test.json")).unwrap();
    fs::write(&path, r#"{"name": "test"}"#).unwrap();

    let result = update_json(&path, &["version"], "1.0.0").unwrap();
    assert!(!result);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo nextest run -p scrat-core json_update 2>&1 | tail -10`
Expected: FAIL — `update_json` doesn't exist yet.

- [ ] **Step 3: Implement JSON dot-path walker and updater**

Add above the `tests` module in `version_files.rs`:

```rust
use crate::bump::BumpError;

// ──────────────────────────────────────────────
// JSON
// ──────────────────────────────────────────────

/// Navigate a `serde_json::Value` via dot-path segments and replace the target string.
/// Returns `true` if the field was found and updated.
fn apply_dot_path_json(
    value: &mut serde_json::Value,
    segments: &[DotSegment],
    version: &str,
) -> bool {
    match segments {
        [] => false,
        [DotSegment::Key(key)] => {
            // Terminal key — replace if it's a string
            if let Some(existing) = value.get(key) {
                if existing.is_string() {
                    value[key.as_str()] = serde_json::Value::String(version.to_owned());
                    return true;
                }
            }
            false
        }
        [DotSegment::Key(key), rest @ ..] => {
            // Descend into object
            if let Some(child) = value.get_mut(key.as_str()) {
                apply_dot_path_json(child, rest, version)
            } else {
                false
            }
        }
        [DotSegment::Wildcard, rest @ ..] => {
            // Iterate array elements
            if let Some(arr) = value.as_array_mut() {
                let mut any = false;
                for elem in arr.iter_mut() {
                    if apply_dot_path_json(elem, rest, version) {
                        any = true;
                    }
                }
                any
            } else {
                false
            }
        }
    }
}

/// Update version in a JSON file at the given dot-paths.
/// Returns `true` if any field was modified.
fn update_json(path: &Utf8Path, dot_paths: &[&str], version: &str) -> BumpResult<bool> {
    let content = std::fs::read_to_string(path).map_err(|e| BumpError::ToolFailed {
        tool: path.to_string(),
        message: format!("failed to read: {e}"),
    })?;

    let mut parsed: serde_json::Value =
        serde_json::from_str(&content).map_err(|e| BumpError::ToolFailed {
            tool: path.to_string(),
            message: format!("failed to parse JSON: {e}"),
        })?;

    let mut any_modified = false;
    for dp in dot_paths {
        let segments = parse_dot_path(dp);
        if apply_dot_path_json(&mut parsed, &segments, version) {
            any_modified = true;
        }
    }

    if any_modified {
        let output =
            serde_json::to_string_pretty(&parsed).map_err(|e| BumpError::ToolFailed {
                tool: path.to_string(),
                message: format!("failed to serialize JSON: {e}"),
            })?;
        std::fs::write(path, format!("{output}\n")).map_err(|e| BumpError::ToolFailed {
            tool: path.to_string(),
            message: format!("failed to write: {e}"),
        })?;
        debug!(%version, %path, "updated JSON version file");
    }

    Ok(any_modified)
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo nextest run -p scrat-core json_update 2>&1 | tail -10`
Expected: 5 tests PASS

- [ ] **Step 5: Commit**

```
feat(version-files): add JSON updater with dot-path navigation

Handles top-level, nested, and wildcard array paths. Returns false
for missing fields (caller decides error vs warning).
```

---

### Task 4: TOML updater

**Files:**
- Modify: `crates/scrat-core/src/version_files.rs`

- [ ] **Step 1: Write TOML updater tests**

Add to the `tests` module:

```rust
#[test]
fn toml_update_top_level_field() {
    let tmp = TempDir::new().unwrap();
    let path = Utf8PathBuf::try_from(tmp.path().join("test.toml")).unwrap();
    fs::write(
        &path,
        "# My config\nname = \"test\"\nversion = \"1.0.0\"\n",
    )
    .unwrap();

    let result = update_toml(&path, &["version"], "2.0.0").unwrap();
    assert!(result);

    let content = fs::read_to_string(&path).unwrap();
    assert!(content.contains("version = \"2.0.0\""));
    // Comment preserved
    assert!(content.contains("# My config"));
}

#[test]
fn toml_update_nested_field() {
    let tmp = TempDir::new().unwrap();
    let path = Utf8PathBuf::try_from(tmp.path().join("test.toml")).unwrap();
    fs::write(
        &path,
        "[package]\nname = \"test\"\nversion = \"1.0.0\"\n",
    )
    .unwrap();

    let result = update_toml(&path, &["package.version"], "2.0.0").unwrap();
    assert!(result);

    let content = fs::read_to_string(&path).unwrap();
    assert!(content.contains("version = \"2.0.0\""));
}

#[test]
fn toml_missing_field_returns_false() {
    let tmp = TempDir::new().unwrap();
    let path = Utf8PathBuf::try_from(tmp.path().join("test.toml")).unwrap();
    fs::write(&path, "[package]\nname = \"test\"\n").unwrap();

    let result = update_toml(&path, &["package.version"], "1.0.0").unwrap();
    assert!(!result);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo nextest run -p scrat-core toml_update 2>&1 | tail -10`
Expected: FAIL — `update_toml` doesn't exist yet.

- [ ] **Step 3: Implement TOML updater**

Add above the `tests` module in `version_files.rs`:

```rust
// ──────────────────────────────────────────────
// TOML (format-preserving via toml_edit)
// ──────────────────────────────────────────────

/// Navigate a `toml_edit::DocumentMut` via dot-path segments and replace the target value.
/// Returns `true` if the field was found and updated.
fn apply_dot_path_toml(doc: &mut toml_edit::DocumentMut, segments: &[DotSegment]) -> Option<&mut toml_edit::Item> {
    let mut current: &mut toml_edit::Item = doc.as_item_mut();
    for seg in segments {
        match seg {
            DotSegment::Key(key) => {
                current = current.get_mut(key.as_str())?;
            }
            DotSegment::Wildcard => {
                // toml_edit arrays of tables — not needed for current use cases.
                // Return None to signal unsupported.
                return None;
            }
        }
    }
    Some(current)
}

/// Update version in a TOML file at the given dot-paths.
/// Uses `toml_edit` for format-preserving updates (comments, whitespace, order).
/// Returns `true` if any field was modified.
fn update_toml(path: &Utf8Path, dot_paths: &[&str], version: &str) -> BumpResult<bool> {
    let content = std::fs::read_to_string(path).map_err(|e| BumpError::ToolFailed {
        tool: path.to_string(),
        message: format!("failed to read: {e}"),
    })?;

    let mut doc: toml_edit::DocumentMut =
        content.parse().map_err(|e| BumpError::ToolFailed {
            tool: path.to_string(),
            message: format!("failed to parse TOML: {e}"),
        })?;

    let mut any_modified = false;
    for dp in dot_paths {
        let segments = parse_dot_path(dp);
        if let Some(item) = apply_dot_path_toml(&mut doc, &segments) {
            if item.is_str() {
                *item = toml_edit::value(version);
                any_modified = true;
            }
        }
    }

    if any_modified {
        std::fs::write(path, doc.to_string()).map_err(|e| BumpError::ToolFailed {
            tool: path.to_string(),
            message: format!("failed to write: {e}"),
        })?;
        debug!(%version, %path, "updated TOML version file");
    }

    Ok(any_modified)
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo nextest run -p scrat-core toml_update 2>&1 | tail -10`
Expected: 3 tests PASS

- [ ] **Step 5: Commit**

```
feat(version-files): add format-preserving TOML updater

Uses toml_edit to modify version fields in-place without destroying
comments, whitespace, or key ordering.
```

---

### Task 5: YAML and frontmatter updaters

**Files:**
- Modify: `crates/scrat-core/src/version_files.rs`

- [ ] **Step 1: Write YAML updater tests**

Add to the `tests` module:

```rust
#[test]
fn yaml_update_top_level_field() {
    let tmp = TempDir::new().unwrap();
    let path = Utf8PathBuf::try_from(tmp.path().join("test.yaml")).unwrap();
    fs::write(&path, "name: test\nversion: \"1.0.0\"\n").unwrap();

    let result = update_yaml(&path, &["version"], "2.0.0").unwrap();
    assert!(result);

    let content = fs::read_to_string(&path).unwrap();
    assert!(content.contains("2.0.0"));
}

#[test]
fn yaml_update_nested_field() {
    let tmp = TempDir::new().unwrap();
    let path = Utf8PathBuf::try_from(tmp.path().join("test.yaml")).unwrap();
    fs::write(&path, "metadata:\n  author: clay\n  version: \"1.0.0\"\n").unwrap();

    let result = update_yaml(&path, &["metadata.version"], "2.0.0").unwrap();
    assert!(result);

    let content = fs::read_to_string(&path).unwrap();
    assert!(content.contains("2.0.0"));
}

#[test]
fn yaml_missing_field_returns_false() {
    let tmp = TempDir::new().unwrap();
    let path = Utf8PathBuf::try_from(tmp.path().join("test.yaml")).unwrap();
    fs::write(&path, "name: test\n").unwrap();

    let result = update_yaml(&path, &["version"], "1.0.0").unwrap();
    assert!(!result);
}
```

- [ ] **Step 2: Write frontmatter updater tests**

Add to the `tests` module:

```rust
#[test]
fn frontmatter_yaml_update() {
    let tmp = TempDir::new().unwrap();
    let path = Utf8PathBuf::try_from(tmp.path().join("SKILL.md")).unwrap();
    fs::write(
        &path,
        "---\nname: my-skill\nmetadata:\n  version: \"1.0.0\"\n---\n\n# My Skill\n\nBody content here.\n",
    )
    .unwrap();

    let result = update_frontmatter(&path, &["metadata.version"], "2.0.0").unwrap();
    assert!(result);

    let content = fs::read_to_string(&path).unwrap();
    assert!(content.contains("2.0.0"));
    // Body preserved
    assert!(content.contains("# My Skill"));
    assert!(content.contains("Body content here."));
    // Delimiters preserved
    assert!(content.starts_with("---\n"));
}

#[test]
fn frontmatter_toml_update() {
    let tmp = TempDir::new().unwrap();
    let path = Utf8PathBuf::try_from(tmp.path().join("page.md")).unwrap();
    fs::write(
        &path,
        "+++\nversion = \"1.0.0\"\ntitle = \"Test\"\n+++\n\n# Page\n",
    )
    .unwrap();

    let result = update_frontmatter(&path, &["version"], "2.0.0").unwrap();
    assert!(result);

    let content = fs::read_to_string(&path).unwrap();
    assert!(content.contains("2.0.0"));
    assert!(content.contains("# Page"));
    assert!(content.starts_with("+++\n"));
}

#[test]
fn frontmatter_preserves_body_exactly() {
    let tmp = TempDir::new().unwrap();
    let path = Utf8PathBuf::try_from(tmp.path().join("SKILL.md")).unwrap();
    let body = "\n# Complex Body\n\nWith **markdown** and `code`.\n\n```rust\nfn main() {}\n```\n";
    let content = format!("---\nname: test\nmetadata:\n  version: \"1.0.0\"\n---\n{body}");
    fs::write(&path, &content).unwrap();

    update_frontmatter(&path, &["metadata.version"], "2.0.0").unwrap();

    let result = fs::read_to_string(&path).unwrap();
    assert!(result.ends_with(body));
}

#[test]
fn frontmatter_missing_field_returns_false() {
    let tmp = TempDir::new().unwrap();
    let path = Utf8PathBuf::try_from(tmp.path().join("SKILL.md")).unwrap();
    fs::write(&path, "---\nname: test\n---\n\n# Body\n").unwrap();

    let result = update_frontmatter(&path, &["metadata.version"], "1.0.0").unwrap();
    assert!(!result);
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo nextest run -p scrat-core yaml_update 2>&1 | tail -5`
Run: `cargo nextest run -p scrat-core frontmatter_ 2>&1 | tail -5`
Expected: FAIL — functions don't exist yet.

- [ ] **Step 4: Implement YAML updater**

Add above the `tests` module in `version_files.rs`:

```rust
// ──────────────────────────────────────────────
// YAML
// ──────────────────────────────────────────────

/// Navigate a serde-saphyr Value via dot-path segments and replace the target string.
/// Returns `true` if the field was found and updated.
fn apply_dot_path_yaml(
    value: &mut serde_saphyr::Value,
    segments: &[DotSegment],
    version: &str,
) -> bool {
    match segments {
        [] => false,
        [DotSegment::Key(key)] => {
            if let serde_saphyr::Value::Mapping(map) = value {
                let yaml_key = serde_saphyr::Value::String(key.clone());
                if let Some(existing) = map.get(&yaml_key) {
                    if existing.is_string() {
                        map.insert(yaml_key, serde_saphyr::Value::String(version.to_owned()));
                        return true;
                    }
                }
            }
            false
        }
        [DotSegment::Key(key), rest @ ..] => {
            if let serde_saphyr::Value::Mapping(map) = value {
                let yaml_key = serde_saphyr::Value::String(key.clone());
                if let Some(child) = map.get_mut(&yaml_key) {
                    return apply_dot_path_yaml(child, rest, version);
                }
            }
            false
        }
        [DotSegment::Wildcard, rest @ ..] => {
            if let serde_saphyr::Value::Sequence(seq) = value {
                let mut any = false;
                for elem in seq.iter_mut() {
                    if apply_dot_path_yaml(elem, rest, version) {
                        any = true;
                    }
                }
                any
            } else {
                false
            }
        }
    }
}

/// Update version in a YAML file at the given dot-paths.
/// Returns `true` if any field was modified.
fn update_yaml(path: &Utf8Path, dot_paths: &[&str], version: &str) -> BumpResult<bool> {
    let content = std::fs::read_to_string(path).map_err(|e| BumpError::ToolFailed {
        tool: path.to_string(),
        message: format!("failed to read: {e}"),
    })?;

    let mut parsed: serde_saphyr::Value =
        serde_saphyr::from_str(&content).map_err(|e| BumpError::ToolFailed {
            tool: path.to_string(),
            message: format!("failed to parse YAML: {e}"),
        })?;

    let mut any_modified = false;
    for dp in dot_paths {
        let segments = parse_dot_path(dp);
        if apply_dot_path_yaml(&mut parsed, &segments, version) {
            any_modified = true;
        }
    }

    if any_modified {
        let output =
            serde_saphyr::to_string(&parsed).map_err(|e| BumpError::ToolFailed {
                tool: path.to_string(),
                message: format!("failed to serialize YAML: {e}"),
            })?;
        std::fs::write(path, output).map_err(|e| BumpError::ToolFailed {
            tool: path.to_string(),
            message: format!("failed to write: {e}"),
        })?;
        debug!(%version, %path, "updated YAML version file");
    }

    Ok(any_modified)
}
```

**Note for implementor:** Verify `serde_saphyr::Value` has `Mapping` and `Sequence` variants and that `Mapping` supports `get()`, `get_mut()`, `insert()`. If the API differs, adapt the field access accordingly. The `is_string()` method should exist on `Value`.

- [ ] **Step 5: Implement frontmatter updater**

Add after the YAML section:

```rust
// ──────────────────────────────────────────────
// Frontmatter (YAML or TOML in markdown files)
// ──────────────────────────────────────────────

/// Delimiter type for frontmatter blocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FrontmatterDelim {
    /// `---` (YAML)
    Yaml,
    /// `+++` (TOML)
    Toml,
}

/// Split a markdown file into (delimiter, frontmatter, body).
/// Returns `None` if the file doesn't start with a recognized delimiter.
fn split_frontmatter(content: &str) -> Option<(FrontmatterDelim, &str, &str)> {
    let delim = if content.starts_with("---") {
        FrontmatterDelim::Yaml
    } else if content.starts_with("+++") {
        FrontmatterDelim::Toml
    } else {
        return None;
    };

    let delim_str = match delim {
        FrontmatterDelim::Yaml => "---",
        FrontmatterDelim::Toml => "+++",
    };

    // Find the closing delimiter (skip the opening line)
    let after_open = &content[delim_str.len()..];
    let after_open = after_open.strip_prefix('\n').unwrap_or(after_open);

    let close_pos = after_open.find(&format!("\n{delim_str}"))
        .map(|p| p + 1) // include the newline before delimiter
        .or_else(|| {
            // Handle case where frontmatter is at the very start
            if after_open.starts_with(delim_str) { Some(0) } else { None }
        })?;

    let frontmatter = &after_open[..close_pos];
    let rest = &after_open[close_pos + delim_str.len()..];

    Some((delim, frontmatter, rest))
}

/// Update version in a markdown file's frontmatter at the given dot-paths.
/// Auto-detects YAML (`---`) or TOML (`+++`) frontmatter.
/// Returns `true` if any field was modified.
fn update_frontmatter(path: &Utf8Path, dot_paths: &[&str], version: &str) -> BumpResult<bool> {
    let content = std::fs::read_to_string(path).map_err(|e| BumpError::ToolFailed {
        tool: path.to_string(),
        message: format!("failed to read: {e}"),
    })?;

    let (delim, fm_str, body) =
        split_frontmatter(&content).ok_or_else(|| BumpError::ToolFailed {
            tool: path.to_string(),
            message: "no frontmatter delimiter found (expected --- or +++)".into(),
        })?;

    let delim_str = match delim {
        FrontmatterDelim::Yaml => "---",
        FrontmatterDelim::Toml => "+++",
    };

    let (modified, new_fm) = match delim {
        FrontmatterDelim::Yaml => {
            let mut parsed: serde_saphyr::Value =
                serde_saphyr::from_str(fm_str).map_err(|e| BumpError::ToolFailed {
                    tool: path.to_string(),
                    message: format!("failed to parse YAML frontmatter: {e}"),
                })?;

            let mut any = false;
            for dp in dot_paths {
                let segments = parse_dot_path(dp);
                if apply_dot_path_yaml(&mut parsed, &segments, version) {
                    any = true;
                }
            }

            let serialized =
                serde_saphyr::to_string(&parsed).map_err(|e| BumpError::ToolFailed {
                    tool: path.to_string(),
                    message: format!("failed to serialize YAML frontmatter: {e}"),
                })?;
            // serde_saphyr::to_string includes trailing newline and may prepend "---\n"
            // Strip any leading "---\n" since we add our own delimiter
            let cleaned = serialized
                .strip_prefix("---\n")
                .unwrap_or(&serialized)
                .trim_end();
            (any, cleaned.to_owned())
        }
        FrontmatterDelim::Toml => {
            let mut doc: toml_edit::DocumentMut =
                fm_str.parse().map_err(|e| BumpError::ToolFailed {
                    tool: path.to_string(),
                    message: format!("failed to parse TOML frontmatter: {e}"),
                })?;

            let mut any = false;
            for dp in dot_paths {
                let segments = parse_dot_path(dp);
                if let Some(item) = apply_dot_path_toml(&mut doc, &segments) {
                    if item.is_str() {
                        *item = toml_edit::value(version);
                        any = true;
                    }
                }
            }
            let serialized = doc.to_string();
            (any, serialized.trim_end().to_owned())
        }
    };

    if modified {
        let output = format!("{delim_str}\n{new_fm}\n{delim_str}{body}");
        std::fs::write(path, output).map_err(|e| BumpError::ToolFailed {
            tool: path.to_string(),
            message: format!("failed to write: {e}"),
        })?;
        debug!(%version, %path, "updated frontmatter version");
    }

    Ok(modified)
}
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo nextest run -p scrat-core yaml_update 2>&1 | tail -10`
Run: `cargo nextest run -p scrat-core frontmatter_ 2>&1 | tail -10`
Expected: All PASS

**Note for implementor:** The `serde_saphyr::to_string` output format may need adjustment. If it prepends `---\n`, the `strip_prefix` handles that. If it doesn't include a trailing newline, the `trim_end()` + explicit `\n` in reassembly handles that. Run the tests and adjust the stripping logic to match actual output.

- [ ] **Step 7: Commit**

```
feat(version-files): add YAML and frontmatter updaters

YAML frontmatter (---) uses serde-saphyr. TOML frontmatter (+++)
uses toml_edit. Markdown body is preserved byte-for-byte.
```

---

### Task 6: Text updater and orchestrator

**Files:**
- Modify: `crates/scrat-core/src/version_files.rs`

- [ ] **Step 1: Write text updater and orchestrator tests**

Add to the `tests` module:

```rust
#[test]
fn text_update_replaces_content() {
    let tmp = TempDir::new().unwrap();
    let path = Utf8PathBuf::try_from(tmp.path().join("VERSION")).unwrap();
    fs::write(&path, "1.0.0\n").unwrap();

    let result = update_text(&path, "2.0.0").unwrap();
    assert!(result);

    let content = fs::read_to_string(&path).unwrap();
    assert_eq!(content, "2.0.0\n");
}

#[test]
fn text_update_trims_whitespace() {
    let tmp = TempDir::new().unwrap();
    let path = Utf8PathBuf::try_from(tmp.path().join("VERSION")).unwrap();
    fs::write(&path, "  1.0.0  \n").unwrap();

    update_text(&path, "2.0.0").unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), "2.0.0\n");
}

#[test]
fn orchestrator_explicit_missing_file_errors() {
    let tmp = TempDir::new().unwrap();
    let root = Utf8PathBuf::try_from(tmp.path().to_path_buf()).unwrap();

    let configs = vec![VersionFileConfig {
        path: "nonexistent.json".into(),
        format: VersionFileFormat::Json,
        field: Some("version".into()),
        fields: None,
    }];

    let result = bump_version_files(&root, &configs, "1.0.0");
    assert!(result.is_err());
}

#[test]
fn orchestrator_glob_no_matches_warns() {
    let tmp = TempDir::new().unwrap();
    let root = Utf8PathBuf::try_from(tmp.path().to_path_buf()).unwrap();

    let configs = vec![VersionFileConfig {
        path: "skills/*/SKILL.md".into(),
        format: VersionFileFormat::Frontmatter,
        field: Some("metadata.version".into()),
        fields: None,
    }];

    // Should succeed (warning, not error) with no modified files
    let modified = bump_version_files(&root, &configs, "1.0.0").unwrap();
    assert!(modified.is_empty());
}

#[test]
fn orchestrator_glob_matches_and_updates() {
    let tmp = TempDir::new().unwrap();
    let root = Utf8PathBuf::try_from(tmp.path().to_path_buf()).unwrap();

    // Create two skill files
    let skill_a = tmp.path().join("skills/alpha");
    let skill_b = tmp.path().join("skills/beta");
    fs::create_dir_all(&skill_a).unwrap();
    fs::create_dir_all(&skill_b).unwrap();
    fs::write(
        skill_a.join("SKILL.md"),
        "---\nname: alpha\nmetadata:\n  version: \"1.0.0\"\n---\n\n# Alpha\n",
    )
    .unwrap();
    fs::write(
        skill_b.join("SKILL.md"),
        "---\nname: beta\nmetadata:\n  version: \"1.0.0\"\n---\n\n# Beta\n",
    )
    .unwrap();

    let configs = vec![VersionFileConfig {
        path: "skills/*/SKILL.md".into(),
        format: VersionFileFormat::Frontmatter,
        field: Some("metadata.version".into()),
        fields: None,
    }];

    let modified = bump_version_files(&root, &configs, "2.0.0").unwrap();
    assert_eq!(modified.len(), 2);

    // Verify both files updated
    let a_content = fs::read_to_string(skill_a.join("SKILL.md")).unwrap();
    let b_content = fs::read_to_string(skill_b.join("SKILL.md")).unwrap();
    assert!(a_content.contains("2.0.0"));
    assert!(b_content.contains("2.0.0"));
}

#[test]
fn orchestrator_glob_skips_files_without_field() {
    let tmp = TempDir::new().unwrap();
    let root = Utf8PathBuf::try_from(tmp.path().to_path_buf()).unwrap();

    let skill_a = tmp.path().join("skills/has-version");
    let skill_b = tmp.path().join("skills/no-version");
    fs::create_dir_all(&skill_a).unwrap();
    fs::create_dir_all(&skill_b).unwrap();
    fs::write(
        skill_a.join("SKILL.md"),
        "---\nname: has-version\nmetadata:\n  version: \"1.0.0\"\n---\n\n# A\n",
    )
    .unwrap();
    fs::write(
        skill_b.join("SKILL.md"),
        "---\nname: no-version\n---\n\n# B\n",
    )
    .unwrap();

    let configs = vec![VersionFileConfig {
        path: "skills/*/SKILL.md".into(),
        format: VersionFileFormat::Frontmatter,
        field: Some("metadata.version".into()),
        fields: None,
    }];

    // Only one file modified, no error for the other
    let modified = bump_version_files(&root, &configs, "2.0.0").unwrap();
    assert_eq!(modified.len(), 1);
}

#[test]
fn orchestrator_explicit_missing_field_errors() {
    let tmp = TempDir::new().unwrap();
    let root = Utf8PathBuf::try_from(tmp.path().to_path_buf()).unwrap();

    fs::write(
        tmp.path().join("plugin.json"),
        r#"{"name": "test"}"#,
    )
    .unwrap();

    let configs = vec![VersionFileConfig {
        path: "plugin.json".into(),
        format: VersionFileFormat::Json,
        field: Some("version".into()),
        fields: None,
    }];

    let result = bump_version_files(&root, &configs, "1.0.0");
    assert!(result.is_err());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo nextest run -p scrat-core orchestrator_ 2>&1 | tail -5`
Run: `cargo nextest run -p scrat-core text_update 2>&1 | tail -5`
Expected: FAIL

- [ ] **Step 3: Implement text updater**

Add above the `tests` module:

```rust
// ──────────────────────────────────────────────
// Text (entire file is the version string)
// ──────────────────────────────────────────────

/// Update a plain text version file. Replaces the entire content with the version string.
/// Returns `true` (text files are always "modified" since we overwrite).
fn update_text(path: &Utf8Path, version: &str) -> BumpResult<bool> {
    std::fs::write(path, format!("{version}\n")).map_err(|e| BumpError::ToolFailed {
        tool: path.to_string(),
        message: format!("failed to write: {e}"),
    })?;
    debug!(%version, %path, "updated text version file");
    Ok(true)
}
```

- [ ] **Step 4: Implement config validation**

Add above the `tests` module:

```rust
// ──────────────────────────────────────────────
// Validation
// ──────────────────────────────────────────────

/// Validate a version file config entry.
fn validate_config(config: &VersionFileConfig) -> BumpResult<()> {
    // field and fields are mutually exclusive
    if config.field.is_some() && config.fields.is_some() {
        return Err(BumpError::ToolFailed {
            tool: config.path.clone(),
            message: "`field` and `fields` are mutually exclusive".into(),
        });
    }

    // text format rejects field/fields
    if config.format == VersionFileFormat::Text {
        if config.field.is_some() || config.fields.is_some() {
            return Err(BumpError::ToolFailed {
                tool: config.path.clone(),
                message: "`text` format does not use `field` or `fields`".into(),
            });
        }
    } else {
        // non-text formats require exactly one of field or fields
        if config.field.is_none() && config.fields.is_none() {
            return Err(BumpError::ToolFailed {
                tool: config.path.clone(),
                message: "non-text formats require `field` or `fields`".into(),
            });
        }
    }

    Ok(())
}
```

- [ ] **Step 5: Implement orchestrator**

Add above the `tests` module:

```rust
// ──────────────────────────────────────────────
// Orchestrator
// ──────────────────────────────────────────────

/// Returns `true` if a path string contains glob metacharacters.
fn is_glob(path: &str) -> bool {
    path.contains('*') || path.contains('?') || path.contains('[')
}

/// Resolve a path (possibly a glob) relative to project root.
/// Returns concrete file paths.
fn resolve_paths(root: &Utf8Path, path_pattern: &str) -> BumpResult<(Vec<Utf8PathBuf>, bool)> {
    let is_glob_pattern = is_glob(path_pattern);
    if is_glob_pattern {
        let full_pattern = root.join(path_pattern).to_string();
        let matches: Vec<Utf8PathBuf> = glob::glob(&full_pattern)
            .map_err(|e| BumpError::ToolFailed {
                tool: path_pattern.to_string(),
                message: format!("invalid glob pattern: {e}"),
            })?
            .filter_map(|entry| entry.ok())
            .filter_map(|p| Utf8PathBuf::try_from(p).ok())
            .collect();
        Ok((matches, true))
    } else {
        let full_path = root.join(path_pattern);
        if !full_path.exists() {
            return Err(BumpError::ToolFailed {
                tool: path_pattern.to_string(),
                message: format!("file not found: {full_path}"),
            });
        }
        Ok((vec![full_path], false))
    }
}

/// Collect dot-paths from a VersionFileConfig into a Vec<&str>.
fn collect_dot_paths(config: &VersionFileConfig) -> Vec<&str> {
    if let Some(ref f) = config.field {
        vec![f.as_str()]
    } else if let Some(ref fs) = config.fields {
        fs.iter().map(|s| s.as_str()).collect()
    } else {
        vec![]
    }
}

/// Update version in all configured version files.
///
/// Returns list of modified file paths (relative to root).
pub fn bump_version_files(
    root: &Utf8Path,
    configs: &[VersionFileConfig],
    version: &str,
) -> BumpResult<Vec<String>> {
    let mut modified_files = Vec::new();

    for config in configs {
        validate_config(config)?;

        let (paths, from_glob) = resolve_paths(root, &config.path)?;

        if from_glob && paths.is_empty() {
            warn!(pattern = %config.path, "glob matched no files");
            continue;
        }

        let dot_paths = collect_dot_paths(config);

        for path in &paths {
            let updated = match config.format {
                VersionFileFormat::Json => update_json(path, &dot_paths, version)?,
                VersionFileFormat::Toml => update_toml(path, &dot_paths, version)?,
                VersionFileFormat::Yaml => update_yaml(path, &dot_paths, version)?,
                VersionFileFormat::Frontmatter => update_frontmatter(path, &dot_paths, version)?,
                VersionFileFormat::Text => update_text(path, version)?,
            };

            if updated {
                // Store path relative to root for BumpOutcome.modified_files
                let relative = path
                    .strip_prefix(root)
                    .unwrap_or(path)
                    .to_string();
                modified_files.push(relative);
            } else if !from_glob {
                // Explicit path with missing field is an error
                return Err(BumpError::ToolFailed {
                    tool: config.path.clone(),
                    message: format!(
                        "field(s) not found in {path} (expected: {})",
                        dot_paths.join(", ")
                    ),
                });
            } else {
                warn!(%path, "field not found in globbed file, skipping");
            }
        }
    }

    Ok(modified_files)
}
```

- [ ] **Step 6: Run all tests to verify they pass**

Run: `cargo nextest run -p scrat-core -E 'test(/version_files/)' 2>&1 | tail -15`
Expected: All version_files tests PASS

- [ ] **Step 7: Commit**

```
feat(version-files): add text updater, config validation, and orchestrator

bump_version_files() resolves globs, dispatches to per-format updaters,
and returns modified file paths. Explicit paths with missing fields
error; globbed paths with missing fields warn and skip.
```

---

### Task 7: Wire into ReadyBump

**Files:**
- Modify: `crates/scrat-core/src/bump.rs:76-87` (ReadyBump struct)
- Modify: `crates/scrat-core/src/bump.rs:113-172` (plan_bump)
- Modify: `crates/scrat-core/src/bump.rs:176-188` (resolve_interactive)
- Modify: `crates/scrat-core/src/bump.rs:239-307` (execute)

- [ ] **Step 1: Write integration test**

Add to the `#[cfg(test)] mod tests` in `bump.rs`:

```rust
#[test]
fn ready_bump_carries_version_files() {
    use crate::config::VersionFileConfig;
    use crate::config::VersionFileFormat;

    let vf = vec![VersionFileConfig {
        path: "plugin.json".into(),
        format: VersionFileFormat::Json,
        field: Some("version".into()),
        fields: None,
    }];

    let bump = ReadyBump {
        previous: Version::new(1, 0, 0),
        next: Version::new(2, 0, 0),
        strategy: VersionStrategy::Interactive,
        detection: generic_detection(),
        version_files: vf.clone(),
    };

    assert_eq!(bump.version_files.len(), 1);
    assert_eq!(bump.version_files[0].path, "plugin.json");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p scrat-core ready_bump_carries 2>&1 | tail -10`
Expected: FAIL — `version_files` field doesn't exist on ReadyBump.

- [ ] **Step 3: Add `version_files` to `ReadyBump` and `InteractiveBump`**

In `crates/scrat-core/src/bump.rs`, update the structs:

```rust
/// A bump plan that is ready to execute.
#[derive(Debug, Clone)]
pub struct ReadyBump {
    /// The previous version (from tags, or 0.0.0 for first release).
    pub previous: Version,
    /// The computed next version.
    pub next: Version,
    /// How the version was determined.
    pub strategy: VersionStrategy,
    /// Detected ecosystem and tools.
    pub detection: ProjectDetection,
    /// Additional files to update with the new version.
    pub version_files: Vec<crate::config::VersionFileConfig>,
}

/// A bump plan that requires the user to pick a version interactively.
#[derive(Debug)]
pub struct InteractiveBump {
    /// Context for the interactive picker (commits, candidates).
    pub context: interactive::InteractiveContext,
    /// Detected ecosystem and tools.
    pub detection: ProjectDetection,
    /// Additional files to update with the new version.
    pub version_files: Vec<crate::config::VersionFileConfig>,
}
```

- [ ] **Step 4: Update `plan_bump()` to populate `version_files`**

In `plan_bump()`, extract version_files from config and add to all `ReadyBump` and `InteractiveBump` construction sites.

Add a local binding early in the function (after the detection step):

```rust
let version_files = config
    .version_files
    .clone()
    .unwrap_or_default();
```

Then add `version_files: version_files.clone(),` to every `ReadyBump { .. }` and `InteractiveBump { .. }` constructor in `plan_bump()`. There are three:

1. `VersionStrategy::Explicit` branch (~line 140):
   ```rust
   Ok(BumpPlan::Ready(ReadyBump {
       previous,
       next,
       strategy,
       detection,
       version_files: version_files.clone(),
   }))
   ```

2. `VersionStrategy::ConventionalCommits` branch (~line 158):
   ```rust
   Ok(BumpPlan::Ready(ReadyBump {
       previous,
       next,
       strategy: VersionStrategy::ConventionalCommits { tool },
       detection,
       version_files: version_files.clone(),
   }))
   ```

3. `VersionStrategy::Interactive` branch (~line 167):
   ```rust
   Ok(BumpPlan::NeedsInteraction(InteractiveBump {
       context,
       detection,
       version_files,
   }))
   ```

- [ ] **Step 5: Update `resolve_interactive()` to carry through `version_files`**

```rust
pub fn resolve_interactive(plan: InteractiveBump, chosen_version: Version) -> ReadyBump {
    let previous = plan
        .context
        .current_version
        .clone()
        .unwrap_or_else(|| Version::new(0, 0, 0));
    ReadyBump {
        previous,
        next: chosen_version,
        strategy: VersionStrategy::Interactive,
        detection: plan.detection,
        version_files: plan.version_files,
    }
}
```

- [ ] **Step 6: Call `bump_version_files()` in `execute()`**

In `ReadyBump::execute()`, add after the ecosystem `match` block (after line ~278) and before the changelog section (before line ~280):

```rust
        // Update configured version files
        if !self.version_files.is_empty() {
            let vf_modified = crate::version_files::bump_version_files(
                project_root,
                &self.version_files,
                &self.next.to_string(),
            )?;
            modified_files.extend(vf_modified);
        }
```

- [ ] **Step 7: Fix any remaining compilation errors**

Run: `cargo check -p scrat-core 2>&1 | head -30`

Grep for other `ReadyBump {` construction sites outside of bump.rs:

Run: `grep -rn 'ReadyBump {' crates/ --include='*.rs'`

If ship.rs or other files construct `ReadyBump` directly, add `version_files: vec![],` to those sites.

- [ ] **Step 8: Run test to verify it passes**

Run: `cargo nextest run -p scrat-core ready_bump_carries 2>&1 | tail -10`
Expected: PASS

- [ ] **Step 9: Run full test suite**

Run: `cargo nextest run -p scrat-core 2>&1 | tail -5`

Also check the CLI crate compiles:

Run: `cargo check -p scrat 2>&1 | tail -5`

Expected: Everything compiles and passes.

- [ ] **Step 10: Commit**

```
feat(version-files): wire into ReadyBump execution

version_files config is stored on ReadyBump during planning, carried
through interactive resolution, and executed after ecosystem bump.
Modified file paths flow into BumpOutcome.modified_files.
```

---

### Post-implementation notes

**Dry-run behavior:** The existing `scrat bump --dry-run` path in the CLI decides whether to call `execute()` at all. If the CLI currently skips `execute()` during dry-run and just prints what would happen, version_files will be included in that output via `ReadyBump.version_files`. If the CLI calls `execute()` with a dry-run flag, a follow-up task would thread that flag into `bump_version_files()`. Check the CLI's dry-run path and handle accordingly.

**serde-saphyr API verification:** The YAML code assumes `serde_saphyr::Value` has `Mapping`/`Sequence` variants with `get()`/`get_mut()`/`insert()` methods. If the API differs from serde_yaml, adapt accordingly — the logic is the same, only method names may vary.

**`glob` crate path conversion:** `glob::glob()` returns `std::path::PathBuf`. The code converts to `Utf8PathBuf` via `try_from()`, filtering out non-UTF-8 paths. This is safe — scrat already requires UTF-8 paths throughout.
