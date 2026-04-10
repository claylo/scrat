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
// YAML (via serde-saphyr, using serde_json::Value as intermediate)
// ──────────────────────────────────────────────

/// YAML has no `Value` type in serde-saphyr, so we deserialize into `serde_json::Value`,
/// reuse the JSON dot-path walker, and serialize back to YAML.
fn update_yaml(path: &Utf8Path, dot_paths: &[&str], version: &str) -> BumpResult<bool> {
    let content = std::fs::read_to_string(path).map_err(|e| BumpError::ToolFailed {
        tool: path.to_string(),
        message: format!("failed to read: {e}"),
    })?;

    let mut parsed: serde_json::Value =
        serde_saphyr::from_str(&content).map_err(|e| BumpError::ToolFailed {
            tool: path.to_string(),
            message: format!("failed to parse YAML: {e}"),
        })?;

    let mut any_modified = false;
    for dp in dot_paths {
        let segments = parse_dot_path(dp);
        if apply_dot_path_json(&mut parsed, &segments, version) {
            any_modified = true;
        }
    }

    if any_modified {
        let output = serde_saphyr::to_string(&parsed).map_err(|e| BumpError::ToolFailed {
            tool: path.to_string(),
            message: format!("failed to serialize YAML: {e}"),
        })?;
        std::fs::write(path, &output).map_err(|e| BumpError::ToolFailed {
            tool: path.to_string(),
            message: format!("failed to write: {e}"),
        })?;
        debug!(%version, %path, "updated YAML version file");
    }

    Ok(any_modified)
}

// ──────────────────────────────────────────────
// Frontmatter (YAML or TOML frontmatter in Markdown)
// ──────────────────────────────────────────────

/// Which delimiter style the frontmatter uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FrontmatterDelim {
    /// YAML frontmatter delimited by `---`
    Yaml,
    /// TOML frontmatter delimited by `+++`
    Toml,
}

/// Split file content into (delimiter, frontmatter_text, body).
///
/// Returns `None` if the content doesn't start with a valid frontmatter delimiter.
/// The body includes everything after the closing delimiter line (including the newline
/// after the delimiter itself).
fn split_frontmatter(content: &str) -> Option<(FrontmatterDelim, &str, &str)> {
    let (delim, marker) = if content.starts_with("---\n") || content.starts_with("---\r\n") {
        (FrontmatterDelim::Yaml, "---")
    } else if content.starts_with("+++\n") || content.starts_with("+++\r\n") {
        (FrontmatterDelim::Toml, "+++")
    } else {
        return None;
    };

    // Skip past the opening delimiter line
    let after_open = &content[marker.len()..];
    let after_open = if after_open.starts_with('\n') {
        &after_open[1..]
    } else if after_open.starts_with("\r\n") {
        &after_open[2..]
    } else {
        return None;
    };

    // Find the closing delimiter: must be at the start of a line
    let closing = format!("\n{marker}\n");
    let closing_crlf = format!("\r\n{marker}\r\n");

    if let Some(pos) = after_open.find(&closing) {
        let fm = &after_open[..pos];
        let body = &after_open[pos + closing.len()..];
        Some((delim, fm, body))
    } else if let Some(pos) = after_open.find(&closing_crlf) {
        let fm = &after_open[..pos];
        let body = &after_open[pos + closing_crlf.len()..];
        Some((delim, fm, body))
    } else {
        None
    }
}

/// Update a version field inside frontmatter (YAML or TOML) of a markdown file.
///
/// The body after the closing delimiter is preserved byte-for-byte.
fn update_frontmatter(path: &Utf8Path, dot_paths: &[&str], version: &str) -> BumpResult<bool> {
    let content = std::fs::read_to_string(path).map_err(|e| BumpError::ToolFailed {
        tool: path.to_string(),
        message: format!("failed to read: {e}"),
    })?;

    let (delim, fm_text, body) = match split_frontmatter(&content) {
        Some(parts) => parts,
        None => {
            warn!(%path, "no frontmatter found");
            return Ok(false);
        }
    };

    let marker = match delim {
        FrontmatterDelim::Yaml => "---",
        FrontmatterDelim::Toml => "+++",
    };

    match delim {
        FrontmatterDelim::Yaml => {
            // Parse YAML frontmatter into serde_json::Value, update, serialize back
            let mut parsed: serde_json::Value =
                serde_saphyr::from_str(fm_text).map_err(|e| BumpError::ToolFailed {
                    tool: path.to_string(),
                    message: format!("failed to parse YAML frontmatter: {e}"),
                })?;

            let mut any_modified = false;
            for dp in dot_paths {
                let segments = parse_dot_path(dp);
                if apply_dot_path_json(&mut parsed, &segments, version) {
                    any_modified = true;
                }
            }

            if !any_modified {
                return Ok(false);
            }

            let mut yaml_out =
                serde_saphyr::to_string(&parsed).map_err(|e| BumpError::ToolFailed {
                    tool: path.to_string(),
                    message: format!("failed to serialize YAML frontmatter: {e}"),
                })?;

            // serde_saphyr::to_string always ends with \n, strip trailing newline
            // since we add our own delimiter line
            if yaml_out.ends_with('\n') {
                yaml_out.truncate(yaml_out.len() - 1);
            }

            let output = format!("{marker}\n{yaml_out}\n{marker}\n{body}");
            std::fs::write(path, output).map_err(|e| BumpError::ToolFailed {
                tool: path.to_string(),
                message: format!("failed to write: {e}"),
            })?;
            debug!(%version, %path, "updated YAML frontmatter version");
            Ok(true)
        }
        FrontmatterDelim::Toml => {
            // Use toml_edit for format-preserving TOML frontmatter
            let fm_with_newline = format!("{fm_text}\n");
            let mut doc: toml_edit::DocumentMut =
                fm_with_newline.parse().map_err(|e| BumpError::ToolFailed {
                    tool: path.to_string(),
                    message: format!("failed to parse TOML frontmatter: {e}"),
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

            if !any_modified {
                return Ok(false);
            }

            let mut toml_out = doc.to_string();
            // Strip trailing newline — we add our own delimiter line
            if toml_out.ends_with('\n') {
                toml_out.truncate(toml_out.len() - 1);
            }

            let output = format!("{marker}\n{toml_out}\n{marker}\n{body}");
            std::fs::write(path, output).map_err(|e| BumpError::ToolFailed {
                tool: path.to_string(),
                message: format!("failed to write: {e}"),
            })?;
            debug!(%version, %path, "updated TOML frontmatter version");
            Ok(true)
        }
    }
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

    // ── YAML updater ───────────────────────────────────────

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

    // ── Frontmatter updater ───────────────────────────────

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
        assert!(content.contains("# My Skill"));
        assert!(content.contains("Body content here."));
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
        let body =
            "\n# Complex Body\n\nWith **markdown** and `code`.\n\n```rust\nfn main() {}\n```\n";
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
}
