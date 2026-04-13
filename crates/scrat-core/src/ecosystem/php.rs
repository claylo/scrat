//! PHP ecosystem driver (`composer.json` / `composer.lock`).
//!
//! `parse_lockfile_diff` walks `composer.lock` as a JSON state
//! machine keyed on `"name"` (namespaced vendor/package form).

use camino::Utf8Path;
use semver::Version;
use tracing::debug;

use super::{EcosystemDriver, emit_change, extract_json_string_value};
use crate::bump::{BumpError, BumpResult};
use crate::detect::has_binary;
use crate::ecosystem::{DetectedTools, Ecosystem, ProjectDetection, VersionStrategy};
use crate::pipeline::DepChange;
use crate::preflight::CheckResult;

/// PHP ecosystem driver.
pub struct PhpDriver;

impl EcosystemDriver for PhpDriver {
    fn bump_version_files(
        &self,
        project_root: &Utf8Path,
        version: &Version,
        _detection: &ProjectDetection,
    ) -> BumpResult<Vec<String>> {
        let composer_path = project_root.join("composer.json");
        let content = match std::fs::read_to_string(&composer_path) {
            Ok(c) => c,
            Err(_) => return Ok(vec![]),
        };

        let mut parsed: serde_json::Value =
            serde_json::from_str(&content).map_err(|e| BumpError::ToolParse {
                tool: "composer.json".into(),
                source: Box::new(e),
            })?;

        // Only write if the field already exists — don't add it if absent
        if parsed.get("version").and_then(|v| v.as_str()).is_none() {
            return Ok(vec![]);
        }

        parsed["version"] = serde_json::Value::String(version.to_string());

        let output =
            serde_json::to_string_pretty(&parsed).map_err(|e| BumpError::ToolSerialize {
                tool: "composer.json".into(),
                source: Box::new(e),
            })?;

        // Composer convention: trailing newline
        std::fs::write(&composer_path, format!("{output}\n")).map_err(|e| BumpError::ToolIo {
            tool: "composer.json".into(),
            source: e,
        })?;

        debug!(%version, "bumped composer.json version");
        Ok(vec!["composer.json".into()])
    }

    fn detect(
        &self,
        _project_root: &Utf8Path,
        version_strategy: VersionStrategy,
    ) -> ProjectDetection {
        let has_composer = has_binary("composer");
        debug!(has_composer, "probed PHP tools");

        let changelog_tool = version_strategy.changelog_tool();

        ProjectDetection {
            ecosystem: Ecosystem::Php,
            version_strategy,
            tools: DetectedTools {
                test_cmd: if has_composer {
                    "composer test".into()
                } else {
                    String::new()
                },
                build_cmd: String::new(),
                publish_cmd: None,
                bump_cmd: None, // PHP bump is done directly in composer.json
                changelog_tool,
            },
        }
    }

    fn check_registry_auth(&self) -> CheckResult {
        CheckResult {
            name: "Registry auth".into(),
            passed: true,
            message: "No registry publish for this ecosystem".into(),
            skip_flag: None,
        }
    }

    fn parse_lockfile_diff(&self, diff: &str) -> Vec<DepChange> {
        let mut changes: Vec<DepChange> = Vec::new();

        let mut current_name: Option<String> = None;
        let mut old_version: Option<String> = None;
        let mut new_version: Option<String> = None;

        for line in diff.lines() {
            let trimmed = line
                .strip_prefix(' ')
                .or_else(|| line.strip_prefix('+'))
                .or_else(|| line.strip_prefix('-'))
                .unwrap_or(line)
                .trim();

            // "name": boundary — emit pending, start new tracking
            if let Some(name) = extract_json_string_value(trimmed, "name") {
                emit_change(&mut changes, &current_name, &old_version, &new_version);
                current_name = Some(name);
                old_version = None;
                new_version = None;
                continue;
            }

            // -"version": — old version
            if line.starts_with('-') {
                if let Some(ver) = extract_json_string_value(trimmed, "version") {
                    old_version = Some(ver);
                }
                continue;
            }

            // +"version": — new version
            if line.starts_with('+')
                && let Some(ver) = extract_json_string_value(trimmed, "version")
            {
                new_version = Some(ver);
            }
        }

        emit_change(&mut changes, &current_name, &old_version, &new_version);
        changes.sort_by(|a, b| a.name.cmp(&b.name));
        changes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_composer_lock_diff_update() {
        let diff = r#"
             "name": "sendgrid/php-http-client",
-            "version": "3.14.3",
+            "version": "3.14.4",
             "source": {
"#;
        let changes = PhpDriver.parse_lockfile_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "sendgrid/php-http-client");
        assert_eq!(changes[0].from.as_deref(), Some("3.14.3"));
        assert_eq!(changes[0].to.as_deref(), Some("3.14.4"));
    }

    #[test]
    fn parse_composer_lock_diff_added() {
        let diff = r#"
+            "name": "new/package",
+            "version": "1.0.0",
"#;
        let changes = PhpDriver.parse_lockfile_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "new/package");
        assert_eq!(changes[0].from, None);
        assert_eq!(changes[0].to.as_deref(), Some("1.0.0"));
    }

    #[test]
    fn parse_composer_lock_diff_removed() {
        let diff = r#"
-            "name": "old/package",
-            "version": "2.0.0",
"#;
        let changes = PhpDriver.parse_lockfile_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "old/package");
        assert_eq!(changes[0].from.as_deref(), Some("2.0.0"));
        assert_eq!(changes[0].to, None);
    }

    #[test]
    fn parse_composer_lock_diff_mixed() {
        let diff = r#"
             "name": "updated/pkg",
-            "version": "1.0.0",
+            "version": "1.1.0",
+            "name": "new/pkg",
+            "version": "0.1.0",
-            "name": "old/pkg",
-            "version": "3.0.0",
"#;
        let changes = PhpDriver.parse_lockfile_diff(diff);
        assert_eq!(changes.len(), 3);
        assert_eq!(changes[0].name, "new/pkg");
        assert_eq!(changes[1].name, "old/pkg");
        assert_eq!(changes[2].name, "updated/pkg");
    }

    #[test]
    fn parse_composer_lock_diff_ignores_reference() {
        let diff = r#"
             "name": "vendor/lib",
-            "version": "1.0.0",
+            "version": "1.0.1",
-                "reference": "abc123"
+                "reference": "def456"
"#;
        let changes = PhpDriver.parse_lockfile_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "vendor/lib");
        assert_eq!(changes[0].to.as_deref(), Some("1.0.1"));
    }

    #[test]
    fn parse_composer_lock_diff_empty() {
        assert!(PhpDriver.parse_lockfile_diff("").is_empty());
    }

    #[test]
    fn parse_composer_lock_diff_stability_suffix() {
        let diff = r#"
             "name": "vendor/lib",
-            "version": "1.12.17-patch7",
+            "version": "1.12.18",
"#;
        let changes = PhpDriver.parse_lockfile_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].from.as_deref(), Some("1.12.17-patch7"));
    }
}
