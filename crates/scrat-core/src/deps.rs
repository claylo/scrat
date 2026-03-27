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
//! - **package-lock.json** (Node) — stub, returns empty

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
/// Stub — returns empty for now. Full implementation deferred.
const fn parse_package_lock_diff(_diff: &str) -> Vec<DepChange> {
    Vec::new()
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

    #[test]
    fn compute_deps_node_returns_empty() {
        // Node ecosystem stub always returns empty
        let changes = parse_package_lock_diff("some diff content");
        assert!(changes.is_empty());
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
}
