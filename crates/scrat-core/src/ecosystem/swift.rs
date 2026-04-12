//! Swift ecosystem driver (`Package.swift` / `Package.resolved`).
//!
//! `parse_lockfile_diff` walks `Package.resolved` as a JSON state
//! machine keyed on `"identity"`. Swift version bumping is a no-op
//! — versions live in git tags.

use camino::Utf8Path;
use semver::Version;
use tracing::debug;

use super::{EcosystemDriver, emit_change, extract_json_string_value};
use crate::bump::BumpResult;
use crate::detect::has_binary;
use crate::ecosystem::{DetectedTools, Ecosystem, ProjectDetection, VersionStrategy};
use crate::pipeline::DepChange;
use crate::preflight::CheckResult;

/// Swift ecosystem driver.
pub struct SwiftDriver;

impl EcosystemDriver for SwiftDriver {
    fn bump_version_files(
        &self,
        _project_root: &Utf8Path,
        _version: &Version,
        _detection: &ProjectDetection,
    ) -> BumpResult<Vec<String>> {
        tracing::debug!("version lives in git tags, no file to bump");
        Ok(Vec::new())
    }

    fn detect(
        &self,
        _project_root: &Utf8Path,
        version_strategy: VersionStrategy,
    ) -> ProjectDetection {
        let has_swift = has_binary("swift");
        debug!(has_swift, "probed Swift tools");

        let changelog_tool = version_strategy.changelog_tool();

        ProjectDetection {
            ecosystem: Ecosystem::Swift,
            version_strategy,
            tools: DetectedTools {
                test_cmd: if has_swift {
                    "swift test".into()
                } else {
                    String::new()
                },
                build_cmd: if has_swift {
                    "swift build -c release".into()
                } else {
                    String::new()
                },
                publish_cmd: None, // SwiftPM publishes via git tags
                bump_cmd: None,
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

            // "identity": boundary — emit pending, start new tracking
            if let Some(name) = extract_json_string_value(trimmed, "identity") {
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
    fn parse_package_resolved_diff_update() {
        let diff = r#"
       "identity" : "swift-nio",
       "kind" : "remoteSourceControl",
       "state" : {
-        "version" : "2.92.0"
+        "version" : "2.92.1"
       }
"#;
        let changes = SwiftDriver.parse_lockfile_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "swift-nio");
        assert_eq!(changes[0].from.as_deref(), Some("2.92.0"));
        assert_eq!(changes[0].to.as_deref(), Some("2.92.1"));
    }

    #[test]
    fn parse_package_resolved_diff_added() {
        let diff = r#"
+      "identity" : "swift-log",
+      "state" : {
+        "version" : "1.5.4"
+      }
"#;
        let changes = SwiftDriver.parse_lockfile_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "swift-log");
        assert_eq!(changes[0].from, None);
        assert_eq!(changes[0].to.as_deref(), Some("1.5.4"));
    }

    #[test]
    fn parse_package_resolved_diff_removed() {
        let diff = r#"
-      "identity" : "old-package",
-      "state" : {
-        "version" : "1.0.0"
-      }
"#;
        let changes = SwiftDriver.parse_lockfile_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].from.as_deref(), Some("1.0.0"));
        assert_eq!(changes[0].to, None);
    }

    #[test]
    fn parse_package_resolved_diff_ignores_revision() {
        let diff = r#"
       "identity" : "swift-nio",
       "state" : {
-        "revision" : "abc123",
-        "version" : "2.92.0"
+        "revision" : "def456",
+        "version" : "2.92.1"
       }
"#;
        let changes = SwiftDriver.parse_lockfile_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].to.as_deref(), Some("2.92.1"));
    }

    #[test]
    fn parse_package_resolved_diff_ignores_file_version() {
        // File-level "version": 3 should not be emitted as a dep change
        let diff = r#"
-  "version" : 2
+  "version" : 3
       "identity" : "swift-nio",
-        "version" : "2.92.0"
+        "version" : "2.92.1"
"#;
        let changes = SwiftDriver.parse_lockfile_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "swift-nio");
    }

    #[test]
    fn parse_package_resolved_diff_mixed() {
        let diff = r#"
       "identity" : "updated-pkg",
-        "version" : "1.0.0"
+        "version" : "1.1.0"
+      "identity" : "new-pkg",
+        "version" : "0.1.0"
-      "identity" : "old-pkg",
-        "version" : "3.0.0"
"#;
        let changes = SwiftDriver.parse_lockfile_diff(diff);
        assert_eq!(changes.len(), 3);
        assert_eq!(changes[0].name, "new-pkg");
        assert_eq!(changes[1].name, "old-pkg");
        assert_eq!(changes[2].name, "updated-pkg");
    }

    #[test]
    fn parse_package_resolved_diff_empty() {
        assert!(SwiftDriver.parse_lockfile_diff("").is_empty());
    }
}
