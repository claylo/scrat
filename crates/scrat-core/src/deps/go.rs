//! Lockfile diff parser for Go's `go.mod`.
//!
//! Line-oriented collect-and-merge: each `require` line is
//! `<module> <version>`. Collects removed/added lines into maps, then
//! merges to produce [`DepChange`] entries.

use std::collections::HashMap;

use super::LockfileDiffParser;
use crate::pipeline::DepChange;

/// Lockfile diff parser for Go's `go.mod`.
pub struct GoLockfileParser;

impl LockfileDiffParser for GoLockfileParser {
    fn parse_diff(&self, diff: &str) -> Vec<DepChange> {
        let mut removed: HashMap<String, String> = HashMap::new();
        let mut added: HashMap<String, String> = HashMap::new();

        for line in diff.lines() {
            // `strip_prefix` gives us the content with the marker removed
            // and naturally skips context / hunk-header lines. Go's
            // collect-and-merge still needs an `is_remove` flag to route
            // entries into the right map, so we can't use the exact
            // state-machine pattern from `deps/{rust,php,swift}.rs`.
            let (is_remove, content) = if let Some(s) = line.strip_prefix('-') {
                (true, s)
            } else if let Some(s) = line.strip_prefix('+') {
                (false, s)
            } else {
                continue;
            };

            // Strip any leading whitespace (tabs in `go.mod`, spaces in blocks)
            let content = content.trim();

            // Skip diff headers and require/block markers
            if content.starts_with("++")
                || content.starts_with("--")
                || content == "require ("
                || content == ")"
                || content.starts_with("module ")
                || content.starts_with("go ")
                || content.starts_with("toolchain ")
            {
                continue;
            }

            // Strip `// indirect` suffix
            let content = content.split("//").next().unwrap_or(content).trim_end();

            // Parse: <module-path> <version>
            let mut parts = content.split_whitespace();
            let Some(module) = parts.next() else {
                continue;
            };
            let Some(version) = parts.next() else {
                continue;
            };

            if is_remove {
                removed.insert(module.to_string(), version.to_string());
            } else {
                added.insert(module.to_string(), version.to_string());
            }
        }

        let mut changes: Vec<DepChange> = Vec::new();

        // Updated: in both removed and added
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
                // Removed only
                changes.push(DepChange {
                    name: name.clone(),
                    from: Some(old_ver.clone()),
                    to: None,
                });
            }
        }

        // Added only: in added but not removed
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
    fn parse_go_mod_diff_update() {
        let diff = "\
-\tgithub.com/spf13/cobra v1.7.0
+\tgithub.com/spf13/cobra v1.8.0";
        let changes = GoLockfileParser.parse_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "github.com/spf13/cobra");
        assert_eq!(changes[0].from.as_deref(), Some("v1.7.0"));
        assert_eq!(changes[0].to.as_deref(), Some("v1.8.0"));
    }

    #[test]
    fn parse_go_mod_diff_added() {
        let diff = "\
+\tgithub.com/new/dep v1.0.0";
        let changes = GoLockfileParser.parse_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "github.com/new/dep");
        assert_eq!(changes[0].from, None);
        assert_eq!(changes[0].to.as_deref(), Some("v1.0.0"));
    }

    #[test]
    fn parse_go_mod_diff_removed() {
        let diff = "\
-\tgithub.com/old/dep v2.0.0";
        let changes = GoLockfileParser.parse_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "github.com/old/dep");
        assert_eq!(changes[0].from.as_deref(), Some("v2.0.0"));
        assert_eq!(changes[0].to, None);
    }

    #[test]
    fn parse_go_mod_diff_indirect_stripped() {
        let diff = "\
-\tgolang.org/x/sys v0.14.0 // indirect
+\tgolang.org/x/sys v0.15.0 // indirect";
        let changes = GoLockfileParser.parse_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "golang.org/x/sys");
        assert_eq!(changes[0].from.as_deref(), Some("v0.14.0"));
        assert_eq!(changes[0].to.as_deref(), Some("v0.15.0"));
    }

    #[test]
    fn parse_go_mod_diff_mixed() {
        let diff = "\
-\tgithub.com/spf13/cobra v1.7.0
+\tgithub.com/spf13/cobra v1.8.0
+\tgithub.com/new/dep v1.0.0
-\tgithub.com/old/dep v2.0.0";
        let changes = GoLockfileParser.parse_diff(diff);
        assert_eq!(changes.len(), 3);
        assert_eq!(changes[0].name, "github.com/new/dep");
        assert_eq!(changes[1].name, "github.com/old/dep");
        assert_eq!(changes[2].name, "github.com/spf13/cobra");
    }

    #[test]
    fn parse_go_mod_diff_skips_headers() {
        let diff = "\
--- a/go.mod
+++ b/go.mod
-\tgithub.com/foo/bar v1.0.0
+\tgithub.com/foo/bar v1.1.0
-module github.com/my/project
+module github.com/my/project
-go 1.21
+go 1.22";
        let changes = GoLockfileParser.parse_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "github.com/foo/bar");
    }

    #[test]
    fn parse_go_mod_diff_major_version_path() {
        let diff = "\
-\tgithub.com/pelletier/go-toml/v2 v2.1.0
+\tgithub.com/pelletier/go-toml/v2 v2.2.0";
        let changes = GoLockfileParser.parse_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "github.com/pelletier/go-toml/v2");
    }

    #[test]
    fn parse_go_mod_diff_empty() {
        assert!(GoLockfileParser.parse_diff("").is_empty());
    }

    #[test]
    fn parse_go_mod_diff_pseudo_version() {
        let diff = "\
-\tgithub.com/foo/bar v0.0.0-20230905200255-921286631fa9
+\tgithub.com/foo/bar v0.0.0-20240101120000-abcdef123456";
        let changes = GoLockfileParser.parse_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(
            changes[0].from.as_deref(),
            Some("v0.0.0-20230905200255-921286631fa9")
        );
    }
}
