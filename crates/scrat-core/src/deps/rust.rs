//! Lockfile diff parser for Rust's `Cargo.lock`.
//!
//! Implements [`LockfileDiffParser`] via a TOML state machine that tracks
//! per-`[[package]]` blocks, extracting `name` and `version` fields from
//! context, removed, and added lines in a unified diff.

use super::{LockfileDiffParser, emit_change, extract_toml_string_value};
use crate::pipeline::DepChange;

/// Lockfile diff parser for Rust's `Cargo.lock`.
pub struct RustLockfileParser;

impl LockfileDiffParser for RustLockfileParser {
    /// Parse a unified diff of `Cargo.lock` into dependency changes.
    ///
    /// State machine tracking per-`[[package]]` blocks:
    /// - `name` from any `name = "..."` line (context, removed, or added)
    /// - `old_version` from `-version = "..."` lines
    /// - `new_version` from `+version = "..."` lines
    ///
    /// At each `[[package]]` boundary or EOF, emits a [`DepChange`] if
    /// we have a name and at least one version that changed.
    fn parse_diff(&self, diff: &str) -> Vec<DepChange> {
        let mut changes: Vec<DepChange> = Vec::new();

        let mut current_name: Option<String> = None;
        let mut old_version: Option<String> = None;
        let mut new_version: Option<String> = None;

        for line in diff.lines() {
            // [[package]] boundary — any prefix (context, +, -)
            let trimmed = line
                .strip_prefix(' ')
                .or_else(|| line.strip_prefix('+'))
                .or_else(|| line.strip_prefix('-'))
                .unwrap_or(line);

            if trimmed.starts_with("[[package]]") {
                // Emit pending change from previous block
                emit_change(&mut changes, &current_name, &old_version, &new_version);
                current_name = None;
                old_version = None;
                new_version = None;
                continue;
            }

            // name = "..." — appears in context, removed, or added lines
            if let Some(name) = extract_toml_string_value(trimmed, "name") {
                current_name = Some(name);
                continue;
            }

            // -version = "..." — old version (removed line)
            if line.starts_with('-') {
                if let Some(ver) = extract_toml_string_value(trimmed, "version") {
                    old_version = Some(ver);
                }
                continue;
            }

            // +version = "..." — new version (added line)
            if line.starts_with('+')
                && let Some(ver) = extract_toml_string_value(trimmed, "version")
            {
                new_version = Some(ver);
            }
        }

        // Emit final pending block
        emit_change(&mut changes, &current_name, &old_version, &new_version);

        // Stable ordering
        changes.sort_by(|a, b| a.name.cmp(&b.name));
        changes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_cargo_lock_diff_update() {
        let diff = r#"
 [[package]]
 name = "serde"
-version = "1.0.0"
+version = "1.0.1"
 source = "registry+https://github.com/rust-lang/crates.io-index"
"#;
        let changes = RustLockfileParser.parse_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "serde");
        assert_eq!(changes[0].from.as_deref(), Some("1.0.0"));
        assert_eq!(changes[0].to.as_deref(), Some("1.0.1"));
    }

    #[test]
    fn parse_cargo_lock_diff_added() {
        let diff = r#"
+[[package]]
+name = "new-crate"
+version = "0.1.0"
+source = "registry+https://github.com/rust-lang/crates.io-index"
"#;
        let changes = RustLockfileParser.parse_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "new-crate");
        assert_eq!(changes[0].from, None);
        assert_eq!(changes[0].to.as_deref(), Some("0.1.0"));
    }

    #[test]
    fn parse_cargo_lock_diff_removed() {
        let diff = r#"
-[[package]]
-name = "old-crate"
-version = "2.0.0"
-source = "registry+https://github.com/rust-lang/crates.io-index"
"#;
        let changes = RustLockfileParser.parse_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "old-crate");
        assert_eq!(changes[0].from.as_deref(), Some("2.0.0"));
        assert_eq!(changes[0].to, None);
    }

    #[test]
    fn parse_cargo_lock_diff_mixed() {
        let diff = r#"
 [[package]]
 name = "serde"
-version = "1.0.0"
+version = "1.0.1"
 source = "registry+https://github.com/rust-lang/crates.io-index"
+[[package]]
+name = "new-crate"
+version = "0.1.0"
+source = "registry+https://github.com/rust-lang/crates.io-index"
-[[package]]
-name = "old-crate"
-version = "2.0.0"
-source = "registry+https://github.com/rust-lang/crates.io-index"
"#;
        let changes = RustLockfileParser.parse_diff(diff);
        assert_eq!(changes.len(), 3);
        // Sorted by name
        assert_eq!(changes[0].name, "new-crate");
        assert_eq!(changes[1].name, "old-crate");
        assert_eq!(changes[2].name, "serde");
    }

    #[test]
    fn parse_cargo_lock_diff_empty() {
        let changes = RustLockfileParser.parse_diff("");
        assert!(changes.is_empty());
    }

    #[test]
    fn parse_cargo_lock_diff_no_version_change() {
        // A block where name appears but no version lines changed — no dep change
        let diff = r#"
 [[package]]
 name = "unchanged"
 version = "1.0.0"
 source = "registry+https://github.com/rust-lang/crates.io-index"
-dependencies = []
+dependencies = ["foo"]
"#;
        let changes = RustLockfileParser.parse_diff(diff);
        assert!(changes.is_empty());
    }

    #[test]
    fn parse_cargo_lock_diff_sorted() {
        let diff = r#"
 [[package]]
 name = "zebra"
-version = "1.0.0"
+version = "2.0.0"
 [[package]]
 name = "alpha"
-version = "0.1.0"
+version = "0.2.0"
 [[package]]
 name = "middle"
-version = "3.0.0"
+version = "3.1.0"
"#;
        let changes = RustLockfileParser.parse_diff(diff);
        assert_eq!(changes.len(), 3);
        assert_eq!(changes[0].name, "alpha");
        assert_eq!(changes[1].name, "middle");
        assert_eq!(changes[2].name, "zebra");
    }
}
