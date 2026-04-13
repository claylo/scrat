//! Node ecosystem driver (`package.json` / `package-lock.json`).
//!
//! `parse_lockfile_diff` reports **top-level dependencies only** —
//! this is intentional. npm lockfile v2/v3 carries thousands of
//! transitive entries; release notes want direct deps. The parser
//! walks top-level `packages` entries via a JSON state machine and
//! ignores nested `node_modules/*` subpackages. This is NOT a stub
//! or partial implementation; it is the design.
//!
//! Scoped packages (`"node_modules/@scope/name"`) are preserved as
//! `@scope/name`.

use camino::Utf8Path;
use semver::Version;
use tracing::debug;

use super::{EcosystemDriver, emit_change};
use crate::bump::{BumpError, BumpResult};
use crate::detect::has_binary;
use crate::ecosystem::{DetectedTools, Ecosystem, ProjectDetection, VersionStrategy};
use crate::pipeline::DepChange;
use crate::preflight::CheckResult;

/// Node ecosystem driver.
pub struct NodeDriver;

impl EcosystemDriver for NodeDriver {
    fn bump_version_files(
        &self,
        project_root: &Utf8Path,
        version: &Version,
        _detection: &ProjectDetection,
    ) -> BumpResult<Vec<String>> {
        let package_path = project_root.join("package.json");
        let content = std::fs::read_to_string(&package_path).map_err(|e| BumpError::ToolIo {
            tool: "package.json".into(),
            source: e,
        })?;

        let mut parsed: serde_json::Value =
            serde_json::from_str(&content).map_err(|e| BumpError::ToolParse {
                tool: "package.json".into(),
                source: Box::new(e),
            })?;

        if parsed.get("version").and_then(|v| v.as_str()).is_none() {
            return Err(BumpError::ToolFailed {
                tool: "package.json".into(),
                message: "no `version` field found — cannot bump".into(),
            });
        }

        parsed["version"] = serde_json::Value::String(version.to_string());

        // npm convention: 2-space indent, trailing newline
        let output =
            serde_json::to_string_pretty(&parsed).map_err(|e| BumpError::ToolSerialize {
                tool: "package.json".into(),
                source: Box::new(e),
            })?;

        std::fs::write(&package_path, format!("{output}\n")).map_err(|e| BumpError::ToolIo {
            tool: "package.json".into(),
            source: e,
        })?;

        debug!(%version, "bumped package.json version");
        Ok(vec!["package.json".into()])
    }

    fn detect(
        &self,
        _project_root: &Utf8Path,
        version_strategy: VersionStrategy,
    ) -> ProjectDetection {
        let has_npm = has_binary("npm");
        let has_yarn = has_binary("yarn");
        let has_pnpm = has_binary("pnpm");
        debug!(has_npm, has_yarn, has_pnpm, "probed Node tools");

        let (test_cmd, build_cmd, publish_cmd) = if has_pnpm {
            (
                "pnpm test".to_string(),
                "pnpm run build".to_string(),
                Some("pnpm publish".to_string()),
            )
        } else if has_yarn {
            (
                "yarn test".to_string(),
                "yarn build".to_string(),
                Some("yarn publish".to_string()),
            )
        } else {
            (
                "npm test".to_string(),
                "npm run build".to_string(),
                has_npm.then(|| "npm publish".to_string()),
            )
        };

        let changelog_tool = version_strategy.changelog_tool();

        ProjectDetection {
            ecosystem: Ecosystem::Node,
            version_strategy,
            tools: DetectedTools {
                test_cmd,
                build_cmd,
                publish_cmd,
                bump_cmd: None, // handled via direct package.json edit
                changelog_tool,
            },
        }
    }

    fn check_registry_auth(&self) -> CheckResult {
        let env_vars = ["NPM_TOKEN", "NODE_AUTH_TOKEN"];
        super::check_registry_auth_impl(&env_vars, "npm", "set NPM_TOKEN or run `npm login`")
    }

    fn parse_lockfile_diff(&self, diff: &str) -> Vec<DepChange> {
        let mut changes: Vec<DepChange> = Vec::new();

        let mut current_name: Option<String> = None;
        let mut old_version: Option<String> = None;
        let mut new_version: Option<String> = None;

        for line in diff.lines() {
            // Classify the diff line and strip the leading marker ('+', '-', ' ').
            let (is_removal, is_addition, content) = if let Some(s) = line.strip_prefix('-') {
                // Ignore the `--- a/path` header
                if s.starts_with("-- ") || s.is_empty() {
                    continue;
                }
                (true, false, s)
            } else if let Some(s) = line.strip_prefix('+') {
                // Ignore the `+++ b/path` header
                if s.starts_with("++ ") || s.is_empty() {
                    continue;
                }
                (false, true, s)
            } else if let Some(s) = line.strip_prefix(' ') {
                (false, false, s)
            } else {
                // Hunk headers (`@@`) and anything else we don't care about.
                continue;
            };

            let trimmed = content.trim_start();

            // A new `"node_modules/..."` package block starts a new logical unit.
            if let Some(name) = extract_top_level_node_modules_name(trimmed) {
                // Flush the previous block
                emit_change(&mut changes, &current_name, &old_version, &new_version);
                current_name = Some(name);
                old_version = None;
                new_version = None;
                continue;
            }

            // Version lines within the current block
            if current_name.is_some()
                && let Some(version) = extract_json_version(trimmed)
            {
                match (is_removal, is_addition) {
                    (true, _) => old_version = Some(version),
                    (_, true) => new_version = Some(version),
                    // Context lines (unchanged) seed both `old_version` and
                    // `new_version` with the same value. The deliberate
                    // equality is caught by `emit_change` which suppresses
                    // entries with equal from/to — so blocks where only a
                    // non-version field (e.g., `resolved`) moved correctly
                    // emit nothing.
                    _ => {
                        if old_version.is_none() {
                            old_version = Some(version.clone());
                        }
                        if new_version.is_none() {
                            new_version = Some(version);
                        }
                    }
                }
            }
        }

        // Emit final pending block
        emit_change(&mut changes, &current_name, &old_version, &new_version);

        // Stable ordering for deterministic output
        changes.sort_by(|a, b| a.name.cmp(&b.name));
        changes
    }
}

/// Extract a top-level `node_modules/<name>` path from a JSON key line.
/// Returns `None` for nested entries or non-matching lines.
fn extract_top_level_node_modules_name(line: &str) -> Option<String> {
    let rest = line.strip_prefix('"')?;
    let close = rest.find('"')?;
    let path = &rest[..close];
    let name = path.strip_prefix("node_modules/")?;
    // Reject nested entries like `node_modules/express/node_modules/debug`.
    if name.contains("/node_modules/") {
        return None;
    }
    // Ensure this really is a key (next significant chars are `": {`).
    let after = &rest[close + 1..];
    if !after.trim_start().starts_with(": {") {
        return None;
    }
    Some(name.to_string())
}

/// Extract a version string from a `"version": "x.y.z"` JSON line.
fn extract_json_version(line: &str) -> Option<String> {
    let rest = line.strip_prefix("\"version\":")?;
    let rest = rest.trim_start();
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── NodeDriver.parse_lockfile_diff ──────────────────────────────

    #[test]
    fn parse_package_lock_diff_version_update() {
        // npm lockfile v3 format — common case: version bump for express
        let diff = r#"
     "node_modules/express": {
-      "version": "4.17.1",
+      "version": "4.18.2",
       "resolved": "https://registry.npmjs.org/express/-/express-4.18.2.tgz",
       "integrity": "sha512-..."
     },
"#;
        let changes = NodeDriver.parse_lockfile_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "express");
        assert_eq!(changes[0].from.as_deref(), Some("4.17.1"));
        assert_eq!(changes[0].to.as_deref(), Some("4.18.2"));
    }

    #[test]
    fn parse_package_lock_diff_added_dependency() {
        let diff = r#"
+    "node_modules/chalk": {
+      "version": "5.3.0",
+      "resolved": "https://registry.npmjs.org/chalk/-/chalk-5.3.0.tgz",
+      "license": "MIT"
+    },
"#;
        let changes = NodeDriver.parse_lockfile_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "chalk");
        assert_eq!(changes[0].from, None);
        assert_eq!(changes[0].to.as_deref(), Some("5.3.0"));
    }

    #[test]
    fn parse_package_lock_diff_removed_dependency() {
        let diff = r#"
-    "node_modules/lodash": {
-      "version": "4.17.21",
-      "resolved": "https://registry.npmjs.org/lodash/-/lodash-4.17.21.tgz",
-      "license": "MIT"
-    },
"#;
        let changes = NodeDriver.parse_lockfile_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "lodash");
        assert_eq!(changes[0].from.as_deref(), Some("4.17.21"));
        assert_eq!(changes[0].to, None);
    }

    #[test]
    fn parse_package_lock_diff_scoped_package() {
        let diff = r#"
     "node_modules/@babel/core": {
-      "version": "7.22.5",
+      "version": "7.23.0",
       "resolved": "https://registry.npmjs.org/@babel/core/-/core-7.23.0.tgz"
     },
"#;
        let changes = NodeDriver.parse_lockfile_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "@babel/core");
        assert_eq!(changes[0].from.as_deref(), Some("7.22.5"));
        assert_eq!(changes[0].to.as_deref(), Some("7.23.0"));
    }

    #[test]
    fn parse_package_lock_diff_skips_nested_dependencies() {
        // Nested node_modules (deep dedup) should NOT appear in output —
        // release notes focus on top-level changes only.
        let diff = r#"
     "node_modules/express/node_modules/debug": {
-      "version": "2.6.8",
+      "version": "2.6.9"
     },
"#;
        let changes = NodeDriver.parse_lockfile_diff(diff);
        assert!(changes.is_empty(), "nested entries should be skipped");
    }

    #[test]
    fn parse_package_lock_diff_mixed_changes() {
        let diff = r#"
     "node_modules/express": {
-      "version": "4.17.1",
+      "version": "4.18.2",
       "resolved": "https://..."
     },
+    "node_modules/chalk": {
+      "version": "5.3.0"
+    },
-    "node_modules/lodash": {
-      "version": "4.17.21"
-    },
"#;
        let changes = NodeDriver.parse_lockfile_diff(diff);
        assert_eq!(changes.len(), 3);
        // Sorted alphabetically
        assert_eq!(changes[0].name, "chalk");
        assert_eq!(changes[1].name, "express");
        assert_eq!(changes[2].name, "lodash");
    }

    #[test]
    fn parse_package_lock_diff_empty_diff() {
        assert!(NodeDriver.parse_lockfile_diff("").is_empty());
    }

    #[test]
    fn parse_package_lock_diff_ignores_diff_headers() {
        // Don't mistake `--- a/package-lock.json` / `+++ b/...` for content
        let diff = r#"--- a/package-lock.json
+++ b/package-lock.json
@@ -12,7 +12,7 @@
     "node_modules/express": {
-      "version": "4.17.1",
+      "version": "4.18.2"
     }"#;
        let changes = NodeDriver.parse_lockfile_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "express");
    }

    #[test]
    fn parse_package_lock_diff_no_version_change() {
        // Package block mentioned but only non-version fields changed
        let diff = r#"
     "node_modules/express": {
       "version": "4.18.2",
-      "resolved": "https://old-registry/..."
+      "resolved": "https://new-registry/..."
     },
"#;
        let changes = NodeDriver.parse_lockfile_diff(diff);
        assert!(
            changes.is_empty(),
            "only version changes should be reported"
        );
    }

    // ── extract_top_level_node_modules_name ────────────────────────

    #[test]
    fn extract_top_level_node_modules_name_basic() {
        assert_eq!(
            extract_top_level_node_modules_name("\"node_modules/express\": {"),
            Some("express".to_string())
        );
    }

    #[test]
    fn extract_top_level_node_modules_name_scoped() {
        assert_eq!(
            extract_top_level_node_modules_name("\"node_modules/@babel/core\": {"),
            Some("@babel/core".to_string())
        );
    }

    #[test]
    fn extract_top_level_node_modules_name_rejects_nested() {
        assert_eq!(
            extract_top_level_node_modules_name("\"node_modules/express/node_modules/debug\": {"),
            None
        );
    }

    #[test]
    fn extract_top_level_node_modules_name_rejects_non_package_key() {
        assert_eq!(
            extract_top_level_node_modules_name("\"name\": \"foo\""),
            None
        );
        assert_eq!(
            extract_top_level_node_modules_name("\"dependencies\": {}"),
            None
        );
    }

    // ── extract_json_version ───────────────────────────────────────

    #[test]
    fn extract_json_version_basic() {
        assert_eq!(
            extract_json_version("\"version\": \"1.2.3\""),
            Some("1.2.3".to_string())
        );
        assert_eq!(
            extract_json_version("\"version\":\"1.2.3\","),
            Some("1.2.3".to_string())
        );
    }

    #[test]
    fn extract_json_version_no_match() {
        assert_eq!(extract_json_version("\"name\": \"foo\""), None);
        assert_eq!(extract_json_version("\"resolved\": \"http://\""), None);
    }

    #[test]
    fn check_registry_auth_node() {
        let result = NodeDriver.check_registry_auth();
        assert_eq!(result.name, "Registry auth");
        if !result.passed {
            assert!(result.message.contains("NPM_TOKEN"));
            assert_eq!(result.skip_flag.as_deref(), Some("--no-publish"));
        }
    }
}
