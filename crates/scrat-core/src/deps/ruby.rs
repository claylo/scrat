//! Lockfile diff parser for Ruby's `Gemfile.lock`.
//!
//! Line-oriented collect-and-merge. Only matches lines with exactly 4
//! spaces of indent (top-level gems under `specs:`), ignoring
//! sub-dependency lines at 6+ spaces. Hash-map-based merge — not a
//! stateful parser.

use std::collections::HashMap;

use super::LockfileDiffParser;
use crate::pipeline::DepChange;

/// Lockfile diff parser for Ruby's `Gemfile.lock`.
pub struct RubyLockfileParser;

impl LockfileDiffParser for RubyLockfileParser {
    fn parse_diff(&self, diff: &str) -> Vec<DepChange> {
        let mut removed: HashMap<String, String> = HashMap::new();
        let mut added: HashMap<String, String> = HashMap::new();

        for line in diff.lines() {
            let (is_remove, is_add) = (line.starts_with('-'), line.starts_with('+'));
            if !is_remove && !is_add {
                continue;
            }

            let content = &line[1..];

            // Skip diff headers
            if content.starts_with("++") || content.starts_with("--") {
                continue;
            }

            // Must be exactly 4 spaces indent (top-level gem, not a sub-dep at 6+)
            if !content.starts_with("    ") || content.starts_with("      ") {
                continue;
            }

            let trimmed = content.trim();

            // Parse "gem-name (1.2.3)" or "gem-name (1.2.3.alpha)"
            if let Some((name, rest)) = trimmed.split_once(" (")
                && let Some(version) = rest.strip_suffix(')')
            {
                if is_remove {
                    removed.insert(name.to_string(), version.to_string());
                } else {
                    added.insert(name.to_string(), version.to_string());
                }
            }
        }

        let mut changes: Vec<DepChange> = Vec::new();

        for (name, old_ver) in &removed {
            if let Some(new_ver) = added.get(name) {
                if old_ver != new_ver {
                    changes.push(DepChange {
                        name: name.clone(),
                        from: Some(old_ver.clone()),
                        to: Some(new_ver.clone()),
                    });
                }
            } else {
                changes.push(DepChange {
                    name: name.clone(),
                    from: Some(old_ver.clone()),
                    to: None,
                });
            }
        }

        for (name, new_ver) in &added {
            if !removed.contains_key(name) {
                changes.push(DepChange {
                    name: name.clone(),
                    from: None,
                    to: Some(new_ver.clone()),
                });
            }
        }

        changes.sort_by(|a, b| a.name.cmp(&b.name));
        changes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_gemfile_lock_diff_update() {
        let diff = "\
-    rails (7.1.2)\n\
+    rails (7.1.3)";
        let changes = RubyLockfileParser.parse_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "rails");
        assert_eq!(changes[0].from.as_deref(), Some("7.1.2"));
        assert_eq!(changes[0].to.as_deref(), Some("7.1.3"));
    }

    #[test]
    fn parse_gemfile_lock_diff_added() {
        let diff = "+    new-gem (1.0.0)";
        let changes = RubyLockfileParser.parse_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "new-gem");
        assert_eq!(changes[0].from, None);
        assert_eq!(changes[0].to.as_deref(), Some("1.0.0"));
    }

    #[test]
    fn parse_gemfile_lock_diff_removed() {
        let diff = "-    old-gem (2.0.0)";
        let changes = RubyLockfileParser.parse_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].from.as_deref(), Some("2.0.0"));
        assert_eq!(changes[0].to, None);
    }

    #[test]
    fn parse_gemfile_lock_diff_ignores_subdeps() {
        // Sub-deps have 6+ spaces indent — must be ignored
        let diff = "\
-    rails (7.1.2)\n\
+    rails (7.1.3)\n\
-      actionpack (= 7.1.2)\n\
+      actionpack (= 7.1.3)";
        let changes = RubyLockfileParser.parse_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "rails");
    }

    #[test]
    fn parse_gemfile_lock_diff_mixed() {
        let diff = "\
-    rails (7.1.2)\n\
+    rails (7.1.3)\n\
+    new-gem (1.0.0)\n\
-    old-gem (2.0.0)";
        let changes = RubyLockfileParser.parse_diff(diff);
        assert_eq!(changes.len(), 3);
        assert_eq!(changes[0].name, "new-gem");
        assert_eq!(changes[1].name, "old-gem");
        assert_eq!(changes[2].name, "rails");
    }

    #[test]
    fn parse_gemfile_lock_diff_empty() {
        assert!(RubyLockfileParser.parse_diff("").is_empty());
    }

    #[test]
    fn parse_gemfile_lock_diff_prerelease() {
        let diff = "\
-    nokogiri (1.16.0.rc1)\n\
+    nokogiri (1.16.0)";
        let changes = RubyLockfileParser.parse_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].from.as_deref(), Some("1.16.0.rc1"));
    }
}
