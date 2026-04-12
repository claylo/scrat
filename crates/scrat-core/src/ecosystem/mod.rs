//! Ecosystem types, drivers, and smart defaults for release workflows.
//!
//! This module groups per-ecosystem logic (detection, version bumping,
//! dependency diff parsing, registry auth) behind the [`EcosystemDriver`]
//! trait. Types live in `types.rs`; each per-ecosystem driver lives in
//! its own file (e.g., `rust.rs`, `node.rs`).
//!
//! The trait contains `parse_lockfile_diff`, `detect`, `bump_version_files`,
//! and `check_registry_auth` — the four methods needed to drive the release
//! pipeline for each supported ecosystem.

mod types;

pub mod generic;
pub mod go;
pub mod node;
pub mod php;
pub mod python;
pub mod ruby;
pub mod rust;
pub mod swift;

pub use types::*;

use camino::Utf8Path;
use semver::Version;

use crate::bump::BumpResult;
use crate::pipeline::DepChange;
use crate::preflight::CheckResult;

/// Per-ecosystem behavior for release workflows.
///
/// Implemented by a zero-sized unit struct per ecosystem
/// (`RustDriver`, `NodeDriver`, `GenericDriver`, …). The `&self` receiver
/// carries no state today but reserves the slot for per-ecosystem state
/// attachment (e.g., `RustDriver { bump_cmd }`) without changing signatures.
///
/// Static data (`marker_files`, `lockfile_path`, `bump_config`) stays on
/// [`Ecosystem`] — this trait owns *behavior*, not pure data lookups.
pub trait EcosystemDriver {
    /// Parse a unified diff of this ecosystem's lockfile into
    /// [`DepChange`] entries.
    ///
    /// Infallible by convention: malformed input returns an empty `Vec`
    /// rather than an error, matching the existing "deps diff failure is
    /// non-fatal" contract established by [`compute_deps`](crate::deps::compute_deps).
    /// Implementations must sort the result by `DepChange.name` for
    /// deterministic output.
    fn parse_lockfile_diff(&self, diff: &str) -> Vec<DepChange>;

    /// Rewrite on-disk version files for this ecosystem.
    ///
    /// Returns the repo-relative paths of files that were actually
    /// modified. Returns an empty `Vec` for ecosystems where the version
    /// lives in git tags (Go, Swift) or there is no project file to
    /// rewrite (Generic).
    ///
    /// The `&ProjectDetection` argument is load-bearing for Rust, which
    /// reads `detection.tools.bump_cmd` to find `cargo set-version`.
    /// Other drivers currently ignore it, but the parameter is passed
    /// uniformly so future drivers can opt in without signature churn.
    fn bump_version_files(
        &self,
        project_root: &Utf8Path,
        version: &Version,
        detection: &ProjectDetection,
    ) -> BumpResult<Vec<String>>;

    /// Build a [`ProjectDetection`] by probing `PATH` and assembling
    /// the smart-default tool commands for this ecosystem.
    fn detect(
        &self,
        project_root: &Utf8Path,
        version_strategy: VersionStrategy,
    ) -> ProjectDetection;

    /// Check registry auth for the publish phase.
    ///
    /// Currently checks credentials for this ecosystem's default public
    /// registry (crates.io, npmjs, PyPI, RubyGems, Packagist). Multi-registry
    /// and private-registry support is tracked as a follow-up — users with
    /// private registries can override by setting the appropriate env var
    /// directly. See `record/superpowers/specs/2026-04-11-multi-registry-check-auth-design.md`.
    ///
    /// Uses fast env-var checks (no network). Returns a passing "no registry
    /// for this ecosystem" `CheckResult` for Go, Swift, PHP, and Generic.
    fn check_registry_auth(&self) -> CheckResult;
}

// ─── Shared lockfile-diff helpers ────────────────────────────────

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

// ─── Shared registry-auth helpers ────────────────────────────────

/// Shared registry-auth env-var check body.
///
/// Used by the real `check_registry_auth` impls in rust/node/python/ruby
/// drivers. Returns a passing `CheckResult` when any env var is set,
/// failing (with `--no-publish` skip flag) otherwise.
pub(super) fn check_registry_auth_impl(
    env_vars: &[&str],
    registry_name: &str,
    login_hint: &str,
) -> CheckResult {
    let found = env_vars.iter().any(|v| std::env::var(v).is_ok());

    if found {
        CheckResult {
            name: "Registry auth".into(),
            passed: true,
            message: format!("{registry_name} credentials found"),
            skip_flag: None,
        }
    } else {
        let vars = env_vars.join(" or ");
        CheckResult {
            name: "Registry auth".into(),
            passed: false,
            message: format!("{vars} not set — {login_hint}"),
            skip_flag: Some("--no-publish".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
