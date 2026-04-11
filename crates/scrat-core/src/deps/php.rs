//! Lockfile diff parser for PHP's `composer.lock`.
//!
//! JSON state machine tracking `"name":` boundaries in the diff,
//! similar in shape to the Cargo.lock parser but matching JSON key
//! patterns instead of TOML.

use super::{LockfileDiffParser, emit_change, extract_json_string_value};
use crate::pipeline::DepChange;

/// Lockfile diff parser for PHP's `composer.lock`.
pub struct PhpLockfileParser;

impl LockfileDiffParser for PhpLockfileParser {
    fn parse_diff(&self, diff: &str) -> Vec<DepChange> {
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
        let changes = PhpLockfileParser.parse_diff(diff);
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
        let changes = PhpLockfileParser.parse_diff(diff);
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
        let changes = PhpLockfileParser.parse_diff(diff);
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
        let changes = PhpLockfileParser.parse_diff(diff);
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
        let changes = PhpLockfileParser.parse_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "vendor/lib");
        assert_eq!(changes[0].to.as_deref(), Some("1.0.1"));
    }

    #[test]
    fn parse_composer_lock_diff_empty() {
        assert!(PhpLockfileParser.parse_diff("").is_empty());
    }

    #[test]
    fn parse_composer_lock_diff_stability_suffix() {
        let diff = r#"
             "name": "vendor/lib",
-            "version": "1.12.17-patch7",
+            "version": "1.12.18",
"#;
        let changes = PhpLockfileParser.parse_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].from.as_deref(), Some("1.12.17-patch7"));
    }
}
