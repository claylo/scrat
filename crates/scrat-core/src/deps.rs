//! Dependency diff — parse lockfile diffs to extract dependency changes.
//!
//! Computes `Vec<DepChange>` from `git diff` of ecosystem-specific lockfiles
//! between a previous tag and HEAD. This data feeds release notes templates
//! and `filter:` hooks via the [`PipelineContext`](crate::pipeline::PipelineContext).
//!
//! Currently supports:
//! - **Cargo.lock** (Rust) — full parser
//! - **go.mod** (Go) — full parser (line-oriented collect-and-merge)
//! - **composer.lock** (PHP) — full parser (JSON state machine)
//! - **package-lock.json** (Node) — full parser (JSON state machine on
//!   lockfile v2/v3, reporting top-level dependencies only)
//! - **uv.lock** (Python) — delegates to Cargo.lock parser (identical TOML format)
//! - **Gemfile.lock** (Ruby) — collect-and-merge on 4-space-indent gem lines
//! - **Package.resolved** (Swift) — JSON state machine on `"identity"`/`"version"`

use tracing::{debug, warn};

use crate::ecosystem::Ecosystem;
use crate::git;
use crate::pipeline::DepChange;

/// Compute dependency changes between a ref and HEAD for the given ecosystem.
///
/// Returns an empty `Vec` if the lockfile doesn't exist or hasn't changed.
/// Deps diff failure is non-fatal — logs a warning and returns empty.
pub fn compute_deps(ecosystem: Ecosystem, previous_tag: &str) -> Vec<DepChange> {
    let Some(lockfile) = ecosystem.lockfile_path() else {
        debug!(%ecosystem, "no lockfile for ecosystem, skipping deps diff");
        return Vec::new();
    };

    let diff = match git::diff_file(previous_tag, lockfile) {
        Ok(d) => d,
        Err(e) => {
            warn!(%e, lockfile, "failed to diff lockfile, skipping deps");
            return Vec::new();
        }
    };

    if diff.is_empty() {
        debug!(lockfile, "no lockfile changes");
        return Vec::new();
    }

    let changes = match ecosystem {
        Ecosystem::Rust => parse_cargo_lock_diff(&diff),
        Ecosystem::Node => parse_package_lock_diff(&diff),
        Ecosystem::Go => parse_go_mod_diff(&diff),
        Ecosystem::Php => parse_composer_lock_diff(&diff),
        Ecosystem::Python => parse_uv_lock_diff(&diff),
        Ecosystem::Ruby => parse_gemfile_lock_diff(&diff),
        Ecosystem::Swift => parse_package_resolved_diff(&diff),
        Ecosystem::Generic => Vec::new(),
    };

    debug!(lockfile, count = changes.len(), "parsed dep changes");
    changes
}

/// Parse a unified diff of `Cargo.lock` into dependency changes.
///
/// State machine tracking per-`[[package]]` blocks:
/// - `name` from any `name = "..."` line (context, removed, or added)
/// - `old_version` from `-version = "..."` lines
/// - `new_version` from `+version = "..."` lines
///
/// At each `[[package]]` boundary or EOF, emits a [`DepChange`] if
/// we have a name and at least one version that changed.
fn parse_cargo_lock_diff(diff: &str) -> Vec<DepChange> {
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

/// Emit a `DepChange` if we have a name and at least one version.
///
/// Skips if both versions are present but equal (no actual change).
fn emit_change(
    changes: &mut Vec<DepChange>,
    name: &Option<String>,
    old_version: &Option<String>,
    new_version: &Option<String>,
) {
    let Some(name) = name else { return };

    // Need at least one version to be interesting
    if old_version.is_none() && new_version.is_none() {
        return;
    }

    // Skip if versions are equal (no change)
    if old_version.is_some() && old_version == new_version {
        return;
    }

    changes.push(DepChange {
        name: name.clone(),
        from: old_version.clone(),
        to: new_version.clone(),
    });
}

/// Extract a TOML string value for a given key.
///
/// Matches lines like `key = "value"` and returns `value`.
fn extract_toml_string_value(line: &str, key: &str) -> Option<String> {
    let line = line.trim();
    let rest = line.strip_prefix(key)?;
    let rest = rest.trim_start();
    let rest = rest.strip_prefix('=')?;
    let rest = rest.trim_start();
    let rest = rest.strip_prefix('"')?;
    let value = rest.strip_suffix('"')?;
    Some(value.to_string())
}

/// Parse a unified diff of `package-lock.json` into dependency changes.
///
/// Targets lockfile version 2 and 3 (npm v7+), which use the `packages`
/// key with paths like `"node_modules/<name>": { "version": "..." }`.
/// Only top-level packages are reported — nested entries like
/// `"node_modules/foo/node_modules/bar"` are intentionally skipped so the
/// release notes focus on direct dependency changes.
///
/// Scoped packages (`"node_modules/@scope/name"`) are preserved as
/// `@scope/name`.
fn parse_package_lock_diff(diff: &str) -> Vec<DepChange> {
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
                // Context lines (unchanged) provide the baseline version for
                // blocks where only one side actually changes a field.
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

/// Parse a unified diff of `uv.lock` into dependency changes.
///
/// `uv.lock` uses TOML with `[[package]]` blocks — structurally identical
/// to `Cargo.lock`. Delegates directly to the Cargo.lock parser.
fn parse_uv_lock_diff(diff: &str) -> Vec<DepChange> {
    parse_cargo_lock_diff(diff)
}

/// Parse a unified diff of `Gemfile.lock` into dependency changes.
///
/// Line-oriented collect-and-merge. Only matches lines with exactly 4 spaces
/// of indent (top-level gems under `specs:`), ignoring sub-dependency lines
/// at 6+ spaces.
fn parse_gemfile_lock_diff(diff: &str) -> Vec<DepChange> {
    use std::collections::HashMap;

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

/// Parse a unified diff of `Package.resolved` (Swift) into dependency changes.
///
/// JSON state machine keyed on `"identity":` boundaries, same pattern as
/// composer.lock but using `"identity"` instead of `"name"`.
fn parse_package_resolved_diff(diff: &str) -> Vec<DepChange> {
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

/// Parse a unified diff of `go.mod` into dependency changes.
///
/// Line-oriented: each `require` line is `<module> <version>`.
/// Collect removed/added lines into maps, then merge to produce changes.
fn parse_go_mod_diff(diff: &str) -> Vec<DepChange> {
    use std::collections::HashMap;

    let mut removed: HashMap<String, String> = HashMap::new();
    let mut added: HashMap<String, String> = HashMap::new();

    for line in diff.lines() {
        let (is_remove, is_add) = (line.starts_with('-'), line.starts_with('+'));
        if !is_remove && !is_add {
            continue;
        }

        // Strip diff prefix and whitespace
        let content = line[1..].trim();

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

/// Parse a unified diff of `composer.lock` into dependency changes.
///
/// State machine tracking `"name":` boundaries in the JSON diff,
/// similar to the Cargo.lock parser but matching JSON key patterns.
fn parse_composer_lock_diff(diff: &str) -> Vec<DepChange> {
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

/// Extract a JSON string value for a given key.
///
/// Matches lines like `"key": "value"` or `"key": "value",` and returns `value`.
fn extract_json_string_value(line: &str, key: &str) -> Option<String> {
    let line = line.trim();
    let rest = line.strip_prefix('"')?;
    let rest = rest.strip_prefix(key)?;
    let rest = rest.strip_prefix('"')?;
    let rest = rest.trim_start();
    let rest = rest.strip_prefix(':')?;
    let rest = rest.trim_start();
    let rest = rest.strip_prefix('"')?;
    let value = rest.strip_suffix(',').unwrap_or(rest);
    let value = value.strip_suffix('"')?;
    Some(value.to_string())
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
        let changes = parse_cargo_lock_diff(diff);
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
        let changes = parse_cargo_lock_diff(diff);
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
        let changes = parse_cargo_lock_diff(diff);
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
        let changes = parse_cargo_lock_diff(diff);
        assert_eq!(changes.len(), 3);
        // Sorted by name
        assert_eq!(changes[0].name, "new-crate");
        assert_eq!(changes[1].name, "old-crate");
        assert_eq!(changes[2].name, "serde");
    }

    #[test]
    fn parse_cargo_lock_diff_empty() {
        let changes = parse_cargo_lock_diff("");
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
        let changes = parse_cargo_lock_diff(diff);
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
        let changes = parse_cargo_lock_diff(diff);
        assert_eq!(changes.len(), 3);
        assert_eq!(changes[0].name, "alpha");
        assert_eq!(changes[1].name, "middle");
        assert_eq!(changes[2].name, "zebra");
    }

    // ── package-lock.json parser (Node) ─────────────────────────────

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
        let changes = parse_package_lock_diff(diff);
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
        let changes = parse_package_lock_diff(diff);
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
        let changes = parse_package_lock_diff(diff);
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
        let changes = parse_package_lock_diff(diff);
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
        let changes = parse_package_lock_diff(diff);
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
        let changes = parse_package_lock_diff(diff);
        assert_eq!(changes.len(), 3);
        // Sorted alphabetically
        assert_eq!(changes[0].name, "chalk");
        assert_eq!(changes[1].name, "express");
        assert_eq!(changes[2].name, "lodash");
    }

    #[test]
    fn parse_package_lock_diff_empty_diff() {
        assert!(parse_package_lock_diff("").is_empty());
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
        let changes = parse_package_lock_diff(diff);
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
        let changes = parse_package_lock_diff(diff);
        assert!(
            changes.is_empty(),
            "only version changes should be reported"
        );
    }

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
    fn extract_toml_string_value_basic() {
        assert_eq!(
            extract_toml_string_value(r#"name = "serde""#, "name"),
            Some("serde".into())
        );
        assert_eq!(
            extract_toml_string_value(r#"version = "1.0.0""#, "version"),
            Some("1.0.0".into())
        );
    }

    #[test]
    fn extract_toml_string_value_no_match() {
        assert_eq!(
            extract_toml_string_value(r#"source = "registry""#, "name"),
            None
        );
        assert_eq!(extract_toml_string_value("not a toml line", "name"), None);
    }

    // ── go.mod parser ───────────────────────────────────────────────

    #[test]
    fn parse_go_mod_diff_update() {
        let diff = "\
-\tgithub.com/spf13/cobra v1.7.0
+\tgithub.com/spf13/cobra v1.8.0";
        let changes = parse_go_mod_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "github.com/spf13/cobra");
        assert_eq!(changes[0].from.as_deref(), Some("v1.7.0"));
        assert_eq!(changes[0].to.as_deref(), Some("v1.8.0"));
    }

    #[test]
    fn parse_go_mod_diff_added() {
        let diff = "\
+\tgithub.com/new/dep v1.0.0";
        let changes = parse_go_mod_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "github.com/new/dep");
        assert_eq!(changes[0].from, None);
        assert_eq!(changes[0].to.as_deref(), Some("v1.0.0"));
    }

    #[test]
    fn parse_go_mod_diff_removed() {
        let diff = "\
-\tgithub.com/old/dep v2.0.0";
        let changes = parse_go_mod_diff(diff);
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
        let changes = parse_go_mod_diff(diff);
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
        let changes = parse_go_mod_diff(diff);
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
        let changes = parse_go_mod_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "github.com/foo/bar");
    }

    #[test]
    fn parse_go_mod_diff_major_version_path() {
        let diff = "\
-\tgithub.com/pelletier/go-toml/v2 v2.1.0
+\tgithub.com/pelletier/go-toml/v2 v2.2.0";
        let changes = parse_go_mod_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "github.com/pelletier/go-toml/v2");
    }

    #[test]
    fn parse_go_mod_diff_empty() {
        assert!(parse_go_mod_diff("").is_empty());
    }

    #[test]
    fn parse_go_mod_diff_pseudo_version() {
        let diff = "\
-\tgithub.com/foo/bar v0.0.0-20230905200255-921286631fa9
+\tgithub.com/foo/bar v0.0.0-20240101120000-abcdef123456";
        let changes = parse_go_mod_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(
            changes[0].from.as_deref(),
            Some("v0.0.0-20230905200255-921286631fa9")
        );
    }

    // ── composer.lock parser ────────────────────────────────────────

    #[test]
    fn parse_composer_lock_diff_update() {
        let diff = r#"
             "name": "sendgrid/php-http-client",
-            "version": "3.14.3",
+            "version": "3.14.4",
             "source": {
"#;
        let changes = parse_composer_lock_diff(diff);
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
        let changes = parse_composer_lock_diff(diff);
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
        let changes = parse_composer_lock_diff(diff);
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
        let changes = parse_composer_lock_diff(diff);
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
        let changes = parse_composer_lock_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "vendor/lib");
        assert_eq!(changes[0].to.as_deref(), Some("1.0.1"));
    }

    #[test]
    fn parse_composer_lock_diff_empty() {
        assert!(parse_composer_lock_diff("").is_empty());
    }

    #[test]
    fn parse_composer_lock_diff_stability_suffix() {
        let diff = r#"
             "name": "vendor/lib",
-            "version": "1.12.17-patch7",
+            "version": "1.12.18",
"#;
        let changes = parse_composer_lock_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].from.as_deref(), Some("1.12.17-patch7"));
    }

    // ── JSON string extractor ───────────────────────────────────────

    #[test]
    fn extract_json_string_value_basic() {
        assert_eq!(
            extract_json_string_value(r#""name": "vendor/lib""#, "name"),
            Some("vendor/lib".into())
        );
        assert_eq!(
            extract_json_string_value(r#""version": "1.0.0","#, "version"),
            Some("1.0.0".into())
        );
    }

    #[test]
    fn extract_json_string_value_no_match() {
        assert_eq!(
            extract_json_string_value(r#""source": "git""#, "name"),
            None
        );
        assert_eq!(extract_json_string_value("not a json line", "name"), None);
    }

    // ── uv.lock (Python) parser ─────────────────────────────────────

    #[test]
    fn parse_uv_lock_diff_update() {
        // Identical to Cargo.lock format
        let diff = r#"
 [[package]]
 name = "requests"
-version = "2.31.0"
+version = "2.32.0"
 source = { registry = "https://pypi.org/simple" }
"#;
        let changes = parse_uv_lock_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "requests");
        assert_eq!(changes[0].from.as_deref(), Some("2.31.0"));
        assert_eq!(changes[0].to.as_deref(), Some("2.32.0"));
    }

    #[test]
    fn parse_uv_lock_diff_added() {
        let diff = r#"
+[[package]]
+name = "new-dep"
+version = "1.0.0"
"#;
        let changes = parse_uv_lock_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].from, None);
        assert_eq!(changes[0].to.as_deref(), Some("1.0.0"));
    }

    #[test]
    fn parse_uv_lock_diff_skips_header() {
        // uv.lock has file-level version/requires-python before [[package]]
        let diff = r#"
-version = 1
+version = 2
 requires-python = ">=3.14"
 [[package]]
 name = "foo"
-version = "1.0.0"
+version = "1.1.0"
"#;
        let changes = parse_uv_lock_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "foo");
    }

    // ── Gemfile.lock (Ruby) parser ──────────────────────────────────

    #[test]
    fn parse_gemfile_lock_diff_update() {
        let diff = "\
-    rails (7.1.2)\n\
+    rails (7.1.3)";
        let changes = parse_gemfile_lock_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "rails");
        assert_eq!(changes[0].from.as_deref(), Some("7.1.2"));
        assert_eq!(changes[0].to.as_deref(), Some("7.1.3"));
    }

    #[test]
    fn parse_gemfile_lock_diff_added() {
        let diff = "+    new-gem (1.0.0)";
        let changes = parse_gemfile_lock_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "new-gem");
        assert_eq!(changes[0].from, None);
        assert_eq!(changes[0].to.as_deref(), Some("1.0.0"));
    }

    #[test]
    fn parse_gemfile_lock_diff_removed() {
        let diff = "-    old-gem (2.0.0)";
        let changes = parse_gemfile_lock_diff(diff);
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
        let changes = parse_gemfile_lock_diff(diff);
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
        let changes = parse_gemfile_lock_diff(diff);
        assert_eq!(changes.len(), 3);
        assert_eq!(changes[0].name, "new-gem");
        assert_eq!(changes[1].name, "old-gem");
        assert_eq!(changes[2].name, "rails");
    }

    #[test]
    fn parse_gemfile_lock_diff_empty() {
        assert!(parse_gemfile_lock_diff("").is_empty());
    }

    #[test]
    fn parse_gemfile_lock_diff_prerelease() {
        let diff = "\
-    nokogiri (1.16.0.rc1)\n\
+    nokogiri (1.16.0)";
        let changes = parse_gemfile_lock_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].from.as_deref(), Some("1.16.0.rc1"));
    }

    // ── Package.resolved (Swift) parser ─────────────────────────────

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
        let changes = parse_package_resolved_diff(diff);
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
        let changes = parse_package_resolved_diff(diff);
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
        let changes = parse_package_resolved_diff(diff);
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
        let changes = parse_package_resolved_diff(diff);
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
        let changes = parse_package_resolved_diff(diff);
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
        let changes = parse_package_resolved_diff(diff);
        assert_eq!(changes.len(), 3);
        assert_eq!(changes[0].name, "new-pkg");
        assert_eq!(changes[1].name, "old-pkg");
        assert_eq!(changes[2].name, "updated-pkg");
    }

    #[test]
    fn parse_package_resolved_diff_empty() {
        assert!(parse_package_resolved_diff("").is_empty());
    }
}
