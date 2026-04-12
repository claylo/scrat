//! Go ecosystem driver (`go.mod`).
//!
//! `parse_lockfile_diff` walks `go.mod` as a line-oriented
//! collect-and-merge pass, tracking `require` and `replace` entries.
//! Go version bumping is a no-op — versions live in git tags, not
//! in any file that scrat rewrites.

use std::collections::HashMap;

use camino::Utf8Path;
use semver::Version;
use tracing::debug;

use super::EcosystemDriver;
use crate::bump::BumpResult;
use crate::detect::has_binary;
use crate::ecosystem::{DetectedTools, Ecosystem, ProjectDetection, VersionStrategy};
use crate::pipeline::DepChange;
use crate::preflight::CheckResult;

/// Go ecosystem driver.
pub struct GoDriver;

impl EcosystemDriver for GoDriver {
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
        let has_go = has_binary("go");
        debug!(has_go, "probed Go tools");

        let changelog_tool = version_strategy.changelog_tool();

        ProjectDetection {
            ecosystem: Ecosystem::Go,
            version_strategy,
            tools: DetectedTools {
                test_cmd: if has_go {
                    "go test ./...".into()
                } else {
                    String::new()
                },
                build_cmd: if has_go {
                    "go build ./...".into()
                } else {
                    String::new()
                },
                publish_cmd: None,
                bump_cmd: None, // Go modules version lives in git tags
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
        let mut removed: HashMap<String, String> = HashMap::new();
        let mut added: HashMap<String, String> = HashMap::new();

        for line in diff.lines() {
            // `strip_prefix` gives us the content with the marker removed
            // and naturally skips context / hunk-header lines. Go's
            // collect-and-merge still needs an `is_remove` flag to route
            // entries into the right map, so we can't use the exact
            // state-machine pattern from `ecosystem/{rust,php,swift}.rs`.
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
        let changes = GoDriver.parse_lockfile_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "github.com/spf13/cobra");
        assert_eq!(changes[0].from.as_deref(), Some("v1.7.0"));
        assert_eq!(changes[0].to.as_deref(), Some("v1.8.0"));
    }

    #[test]
    fn parse_go_mod_diff_added() {
        let diff = "\
+\tgithub.com/new/dep v1.0.0";
        let changes = GoDriver.parse_lockfile_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "github.com/new/dep");
        assert_eq!(changes[0].from, None);
        assert_eq!(changes[0].to.as_deref(), Some("v1.0.0"));
    }

    #[test]
    fn parse_go_mod_diff_removed() {
        let diff = "\
-\tgithub.com/old/dep v2.0.0";
        let changes = GoDriver.parse_lockfile_diff(diff);
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
        let changes = GoDriver.parse_lockfile_diff(diff);
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
        let changes = GoDriver.parse_lockfile_diff(diff);
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
        let changes = GoDriver.parse_lockfile_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "github.com/foo/bar");
    }

    #[test]
    fn parse_go_mod_diff_major_version_path() {
        let diff = "\
-\tgithub.com/pelletier/go-toml/v2 v2.1.0
+\tgithub.com/pelletier/go-toml/v2 v2.2.0";
        let changes = GoDriver.parse_lockfile_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "github.com/pelletier/go-toml/v2");
    }

    #[test]
    fn parse_go_mod_diff_empty() {
        assert!(GoDriver.parse_lockfile_diff("").is_empty());
    }

    #[test]
    fn parse_go_mod_diff_pseudo_version() {
        let diff = "\
-\tgithub.com/foo/bar v0.0.0-20230905200255-921286631fa9
+\tgithub.com/foo/bar v0.0.0-20240101120000-abcdef123456";
        let changes = GoDriver.parse_lockfile_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(
            changes[0].from.as_deref(),
            Some("v0.0.0-20230905200255-921286631fa9")
        );
    }

    #[test]
    fn check_registry_auth_go_skips() {
        // Go doesn't publish via registry
        let result = GoDriver.check_registry_auth();
        assert!(result.passed);
        assert!(result.message.contains("No registry publish"));
    }
}
