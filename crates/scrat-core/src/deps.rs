//! Dependency diff — parse lockfile diffs to extract dependency changes.
//!
//! Computes `Vec<DepChange>` from `git diff` of ecosystem-specific lockfiles
//! between a previous tag and HEAD. This data feeds release notes templates
//! and `filter:` hooks via the [`PipelineContext`](crate::pipeline::PipelineContext).
//!
//! Currently supports:
//! - **Cargo.lock** (Rust) — full parser
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
    let lockfile = ecosystem.lockfile_path();

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
}
