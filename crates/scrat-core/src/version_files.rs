//! Version file updater for arbitrary JSON, TOML, YAML, frontmatter, and text files.
//!
//! Called by [`crate::bump::ReadyBump::execute`] after ecosystem-specific bumping.
//! Resolves glob patterns, navigates dot-paths, and updates version values in place.

use camino::{Utf8Path, Utf8PathBuf};
use tracing::{debug, warn};

use crate::bump::{BumpError, BumpResult};
use crate::config::{VersionFileConfig, VersionFileFormat};

// ──────────────────────────────────────────────
// Dot-path parsing
// ──────────────────────────────────────────────

/// A segment in a dot-path like `"metadata.version"` or `"plugins.*.version"`.
#[derive(Debug, Clone, PartialEq, Eq)]
enum DotSegment {
    Key(String),
    Wildcard,
}

/// Parse a dot-path string into segments.
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

// ──────────────────────────────────────────────
// JSON
// ──────────────────────────────────────────────

fn apply_dot_path_json(
    value: &mut serde_json::Value,
    segments: &[DotSegment],
    version: &str,
) -> bool {
    match segments {
        [] => false,
        [DotSegment::Key(key)] => {
            if let Some(existing) = value.get(key) {
                if existing.is_string() {
                    value[key.as_str()] = serde_json::Value::String(version.to_owned());
                    return true;
                }
            }
            false
        }
        [DotSegment::Key(key), rest @ ..] => {
            if let Some(child) = value.get_mut(key.as_str()) {
                apply_dot_path_json(child, rest, version)
            } else {
                false
            }
        }
        [DotSegment::Wildcard, rest @ ..] => {
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
        let output = serde_json::to_string_pretty(&parsed).map_err(|e| BumpError::ToolFailed {
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

// ──────────────────────────────────────────────
// TOML (format-preserving via toml_edit)
// ──────────────────────────────────────────────

fn apply_dot_path_toml<'a>(
    doc: &'a mut toml_edit::DocumentMut,
    segments: &[DotSegment],
) -> Option<&'a mut toml_edit::Item> {
    let mut current: &mut toml_edit::Item = doc.as_item_mut();
    for seg in segments {
        match seg {
            DotSegment::Key(key) => {
                current = current.get_mut(key.as_str())?;
            }
            DotSegment::Wildcard => {
                return None;
            }
        }
    }
    Some(current)
}

fn update_toml(path: &Utf8Path, dot_paths: &[&str], version: &str) -> BumpResult<bool> {
    let content = std::fs::read_to_string(path).map_err(|e| BumpError::ToolFailed {
        tool: path.to_string(),
        message: format!("failed to read: {e}"),
    })?;

    let mut doc: toml_edit::DocumentMut = content.parse().map_err(|e| BumpError::ToolFailed {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_key() {
        assert_eq!(
            parse_dot_path("version"),
            vec![DotSegment::Key("version".into())]
        );
    }

    #[test]
    fn parse_nested_path() {
        assert_eq!(
            parse_dot_path("metadata.version"),
            vec![
                DotSegment::Key("metadata".into()),
                DotSegment::Key("version".into())
            ]
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

    // ── JSON updater ───────────────────────────────────────

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
    }

    #[test]
    fn json_update_wildcard_array() {
        let tmp = TempDir::new().unwrap();
        let path = Utf8PathBuf::try_from(tmp.path().join("mkt.json")).unwrap();
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
        let path = Utf8PathBuf::try_from(tmp.path().join("mkt.json")).unwrap();
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

    // ── TOML updater ───────────────────────────────────────

    #[test]
    fn toml_update_top_level_field() {
        let tmp = TempDir::new().unwrap();
        let path = Utf8PathBuf::try_from(tmp.path().join("test.toml")).unwrap();
        fs::write(&path, "# My config\nname = \"test\"\nversion = \"1.0.0\"\n").unwrap();
        let result = update_toml(&path, &["version"], "2.0.0").unwrap();
        assert!(result);
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("version = \"2.0.0\""));
        assert!(content.contains("# My config"));
    }

    #[test]
    fn toml_update_nested_field() {
        let tmp = TempDir::new().unwrap();
        let path = Utf8PathBuf::try_from(tmp.path().join("test.toml")).unwrap();
        fs::write(&path, "[package]\nname = \"test\"\nversion = \"1.0.0\"\n").unwrap();
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
}
