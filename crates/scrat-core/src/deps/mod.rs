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

mod rust;
pub use rust::RustLockfileParser;

mod node;
pub use node::NodeLockfileParser;

mod go;
pub use go::GoLockfileParser;

mod php;
pub use php::PhpLockfileParser;

mod python;
pub use python::PythonLockfileParser;

mod ruby;
pub use ruby::RubyLockfileParser;

mod swift;
pub use swift::SwiftLockfileParser;

/// Parses a unified diff of an ecosystem-specific lockfile into
/// [`DepChange`] entries.
///
/// Implemented by zero-sized unit structs per ecosystem
/// (`RustLockfileParser`, `NodeLockfileParser`, …). The `&self` receiver
/// carries no state today, but preserves Phase 4's flexibility to attach
/// per-ecosystem state (e.g., `RustDriver { bump_cmd }`) without changing
/// the method signature.
///
/// Parsers are infallible by convention: malformed input returns an empty
/// `Vec` rather than an error, matching the existing "deps diff failure is
/// non-fatal" contract established by [`compute_deps`].
pub trait LockfileDiffParser {
    /// Parse a unified diff into dependency changes.
    ///
    /// Returns an empty `Vec` if the diff contains no recognizable
    /// dependency changes. Implementations must sort the result by
    /// `DepChange.name` for deterministic output.
    fn parse_diff(&self, diff: &str) -> Vec<DepChange>;
}

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
        Ecosystem::Rust => RustLockfileParser.parse_diff(&diff),
        Ecosystem::Node => NodeLockfileParser.parse_diff(&diff),
        Ecosystem::Go => GoLockfileParser.parse_diff(&diff),
        Ecosystem::Php => PhpLockfileParser.parse_diff(&diff),
        Ecosystem::Python => PythonLockfileParser.parse_diff(&diff),
        Ecosystem::Ruby => RubyLockfileParser.parse_diff(&diff),
        Ecosystem::Swift => SwiftLockfileParser.parse_diff(&diff),
        Ecosystem::Generic => Vec::new(),
    };

    debug!(lockfile, count = changes.len(), "parsed dep changes");
    changes
}

/// Emit a `DepChange` if we have a name and at least one version.
///
/// Skips if both versions are present but equal (no actual change).
pub(super) fn emit_change(
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
pub(super) fn extract_toml_string_value(line: &str, key: &str) -> Option<String> {
    let line = line.trim();
    let rest = line.strip_prefix(key)?;
    let rest = rest.trim_start();
    let rest = rest.strip_prefix('=')?;
    let rest = rest.trim_start();
    let rest = rest.strip_prefix('"')?;
    let value = rest.strip_suffix('"')?;
    Some(value.to_string())
}

/// Extract a JSON string value for a given key.
///
/// Matches lines like `"key": "value"` or `"key": "value",` and returns `value`.
pub(super) fn extract_json_string_value(line: &str, key: &str) -> Option<String> {
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

    // ── TOML string extractor ───────────────────────────────────────

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
