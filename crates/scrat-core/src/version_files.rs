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

// ──────────────────────────────────────────────
// Text (plain version file)
// ──────────────────────────────────────────────

fn update_text(path: &Utf8Path, version: &str) -> BumpResult<bool> {
    std::fs::write(path, format!("{version}\n")).map_err(|e| BumpError::ToolFailed {
        tool: path.to_string(),
        message: format!("failed to write: {e}"),
    })?;
    debug!(%version, %path, "updated text version file");
    Ok(true)
}

// ──────────────────────────────────────────────
// Config validation
// ──────────────────────────────────────────────

fn validate_config(config: &VersionFileConfig) -> BumpResult<()> {
    if config.field.is_some() && config.fields.is_some() {
        return Err(BumpError::ToolFailed {
            tool: config.path.clone(),
            message: "`field` and `fields` are mutually exclusive".into(),
        });
    }
    if config.format == VersionFileFormat::Text {
        if config.field.is_some() || config.fields.is_some() {
            return Err(BumpError::ToolFailed {
                tool: config.path.clone(),
                message: "`text` format does not use `field` or `fields`".into(),
            });
        }
    } else if config.field.is_none() && config.fields.is_none() {
        return Err(BumpError::ToolFailed {
            tool: config.path.clone(),
            message: "non-text formats require `field` or `fields`".into(),
        });
    }
    Ok(())
}

// ──────────────────────────────────────────────
// Orchestrator
// ──────────────────────────────────────────────

fn is_glob(path: &str) -> bool {
    path.contains('*') || path.contains('?') || path.contains('[')
}

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
                let relative = path.strip_prefix(root).unwrap_or(path).to_string();
                modified_files.push(relative);
            } else if !from_glob {
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

// ──────────────────────────────────────────────
// TOML (format-preserving via toml_edit)
// ──────────────────────────────────────────────

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

    // ── Text updater ──────────────────────────────────────

    #[test]
    fn text_update_replaces_content() {
        let tmp = TempDir::new().unwrap();
        let path = Utf8PathBuf::try_from(tmp.path().join("VERSION")).unwrap();
        fs::write(&path, "1.0.0\n").unwrap();
        let result = update_text(&path, "2.0.0").unwrap();
        assert!(result);
        assert_eq!(fs::read_to_string(&path).unwrap(), "2.0.0\n");
    }

    // ── Orchestrator ──────────────────────────────────────

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
        let modified = bump_version_files(&root, &configs, "1.0.0").unwrap();
        assert!(modified.is_empty());
    }

    #[test]
    fn orchestrator_glob_matches_and_updates() {
        let tmp = TempDir::new().unwrap();
        let root = Utf8PathBuf::try_from(tmp.path().to_path_buf()).unwrap();
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
        assert!(
            fs::read_to_string(skill_a.join("SKILL.md"))
                .unwrap()
                .contains("2.0.0")
        );
        assert!(
            fs::read_to_string(skill_b.join("SKILL.md"))
                .unwrap()
                .contains("2.0.0")
        );
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
        let modified = bump_version_files(&root, &configs, "2.0.0").unwrap();
        assert_eq!(modified.len(), 1);
    }

    #[test]
    fn orchestrator_explicit_missing_field_errors() {
        let tmp = TempDir::new().unwrap();
        let root = Utf8PathBuf::try_from(tmp.path().to_path_buf()).unwrap();
        fs::write(tmp.path().join("plugin.json"), r#"{"name": "test"}"#).unwrap();
        let configs = vec![VersionFileConfig {
            path: "plugin.json".into(),
            format: VersionFileFormat::Json,
            field: Some("version".into()),
            fields: None,
        }];
        let result = bump_version_files(&root, &configs, "1.0.0");
        assert!(result.is_err());
    }
}
