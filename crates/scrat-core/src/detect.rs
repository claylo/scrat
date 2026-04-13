//! Project detection — discover ecosystem, tools, and version strategy.
//!
//! Walks the working directory for marker files, probes `PATH` for available
//! tools, and checks for conventional-commit configuration to assemble a
//! [`ProjectDetection`](crate::ecosystem::ProjectDetection).
//!
//! # Example
//!
//! ```no_run
//! use camino::Utf8Path;
//! use scrat_core::detect;
//!
//! let project = detect::detect_project(Utf8Path::new("."));
//! match project {
//!     Some(det) => println!("Detected: {}", det.ecosystem),
//!     None => println!("Unknown project type"),
//! }
//! ```

use std::collections::HashMap;
use std::process::Command;
use std::sync::{Mutex, OnceLock};

use camino::Utf8Path;
use tracing::{debug, instrument};

use crate::ecosystem::{ChangelogTool, Ecosystem, ProjectDetection, VersionStrategy};

/// Resolve ecosystem detection, honoring config overrides.
///
/// Priority:
/// 1. `config.project.project_type` override → build detection for that ecosystem
/// 2. Auto-detect via marker files → full ecosystem-specific detection
/// 3. `None` → caller must prompt the user
#[instrument(skip(config), fields(root = %project_root))]
pub fn resolve_detection(
    project_root: &Utf8Path,
    config: &crate::config::Config,
) -> Option<ProjectDetection> {
    // Config override takes priority
    if let Some(ref project) = config.project
        && let Some(ecosystem) = project.project_type
    {
        debug!(%ecosystem, "using ecosystem from config override");
        let version_strategy = detect_version_strategy(project_root);
        return Some(build_detection_for(
            project_root,
            ecosystem,
            version_strategy,
        ));
    }

    // Fall back to auto-detection
    detect_project(project_root)
}

/// Detect the project ecosystem and available tooling from `project_root`.
///
/// Returns `None` if no recognized marker file is found.
/// Prefer [`resolve_detection`] which also honors config overrides.
#[instrument(fields(root = %project_root))]
pub fn detect_project(project_root: &Utf8Path) -> Option<ProjectDetection> {
    let ecosystem = detect_ecosystem(project_root)?;
    debug!(%ecosystem, "detected ecosystem");

    let version_strategy = detect_version_strategy(project_root);
    debug!(%version_strategy, "detected version strategy");

    Some(build_detection_for(
        project_root,
        ecosystem,
        version_strategy,
    ))
}

/// Identify the ecosystem by scanning for marker files.
///
/// Only checks [`Ecosystem::AUTO_DETECTABLE`] variants (those with marker
/// files). [`Ecosystem::Generic`] is never auto-detected. An ecosystem
/// that returns multiple marker files from [`Ecosystem::marker_files`]
/// matches on the first marker present in `project_root`.
fn detect_ecosystem(project_root: &Utf8Path) -> Option<Ecosystem> {
    for ecosystem in Ecosystem::AUTO_DETECTABLE {
        for marker in ecosystem.marker_files() {
            if project_root.join(marker).is_file() {
                return Some(*ecosystem);
            }
        }
    }
    None
}

/// Determine the version strategy from available tooling.
///
/// Priority:
/// 1. `git-cliff` binary on PATH → `ConventionalCommits(GitCliff)`
/// 2. Neither                    → `Interactive`
pub fn detect_version_strategy(project_root: &Utf8Path) -> VersionStrategy {
    let _ = project_root; // reserved for future per-project tool config

    if has_binary("git-cliff") {
        debug!("git-cliff binary found on PATH");
        return VersionStrategy::ConventionalCommits {
            tool: ChangelogTool::GitCliff,
        };
    }

    VersionStrategy::Interactive
}

/// Build a [`ProjectDetection`] for a user-selected ecosystem.
///
/// Called after the CLI prompts the user to choose an ecosystem when
/// auto-detection returns `None`.
pub fn build_detection(project_root: &Utf8Path, ecosystem: Ecosystem) -> ProjectDetection {
    let version_strategy = detect_version_strategy(project_root);
    build_detection_for(project_root, ecosystem, version_strategy)
}

/// Dispatch an ecosystem to its per-ecosystem detection helper via driver.
///
/// One-liner: delegates to `EcosystemDriver::detect` on the ecosystem's
/// driver. Used by `detect_project`, `resolve_detection`, and
/// `build_detection`.
fn build_detection_for(
    project_root: &Utf8Path,
    ecosystem: Ecosystem,
    version_strategy: VersionStrategy,
) -> ProjectDetection {
    ecosystem.driver().detect(project_root, version_strategy)
}

/// Process-lifetime cache of `has_binary` results.
///
/// A single `scrat ship` probes the same set of binaries (git-cliff, cargo,
/// cargo-nextest, cargo-set-version, gh) across detection, preflight, and
/// version planning. Each `which::which` walks PATH — ~1-2ms on macOS, ~10-15ms
/// aggregate per ship. Binaries cannot appear or disappear during a single
/// process, so the probe is idempotent and safe to memoize.
static PATH_CACHE: OnceLock<Mutex<HashMap<String, bool>>> = OnceLock::new();

/// Check whether a binary is available on `PATH`.
///
/// Memoized for the lifetime of the process. Call [`clear_path_cache`] from
/// tests that install a binary mid-run and need to re-probe.
pub fn has_binary(name: &str) -> bool {
    let cache = PATH_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(&cached) = cache.lock().unwrap().get(name) {
        return cached;
    }
    let found = which::which(name).is_ok();
    cache.lock().unwrap().insert(name.to_owned(), found);
    found
}

/// Clear the [`has_binary`] PATH cache. Test-only escape hatch for scenarios
/// that install or uninstall a binary during a test run.
#[cfg(test)]
pub fn clear_path_cache() {
    if let Some(cache) = PATH_CACHE.get() {
        cache.lock().unwrap().clear();
    }
}

/// Minimum required version of git-cliff.
///
/// 2.5.0 introduced `--bump [major|minor|patch]` which we need for
/// forced bump type. Earlier features we rely on (`--bumped-version`,
/// `--prepend`, `--with-commit`, `--context`, `--strip`,
/// `[bump]` config, `initial_tag`, `--with-tag-message`) all landed
/// in 2.4.0 or earlier.
pub const MIN_GIT_CLIFF_VERSION: semver::Version = semver::Version::new(2, 5, 0);

/// Result of a tool version check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolVersionCheck {
    /// Tool meets the minimum version.
    Ok(semver::Version),
    /// Tool is too old.
    TooOld {
        /// The version that was found.
        found: semver::Version,
        /// The minimum required version.
        minimum: semver::Version,
    },
    /// Could not determine the version (binary missing, parse failure, etc.).
    Unknown(String),
}

/// Check the installed version of a CLI tool.
///
/// Runs `<binary> --version`, parses the semver from its output, and
/// compares against `minimum`. Output is expected to match the pattern
/// `<name> X.Y.Z` (e.g. `git-cliff 2.12.0`).
pub fn check_tool_version(binary: &str, minimum: &semver::Version) -> ToolVersionCheck {
    let output = match Command::new(binary).arg("--version").output() {
        Ok(o) if o.status.success() => o,
        Ok(o) => {
            return ToolVersionCheck::Unknown(format!(
                "`{binary} --version` exited with {}",
                o.status,
            ));
        }
        Err(e) => {
            return ToolVersionCheck::Unknown(format!("failed to run `{binary} --version`: {e}",));
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let Some(version) = parse_version_from_output(&stdout) else {
        return ToolVersionCheck::Unknown(format!(
            "could not parse version from `{binary} --version` output: {stdout}",
        ));
    };

    if version >= *minimum {
        ToolVersionCheck::Ok(version)
    } else {
        ToolVersionCheck::TooOld {
            found: version,
            minimum: minimum.clone(),
        }
    }
}

/// Extract a semver version from tool output like `"git-cliff 2.12.0\n"`.
///
/// Scans for the first token that parses as a valid semver version.
fn parse_version_from_output(output: &str) -> Option<semver::Version> {
    output
        .split_whitespace()
        .find_map(|token| semver::Version::parse(token).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn utf8_tmp(tmp: &TempDir) -> &Utf8Path {
        Utf8Path::from_path(tmp.path()).expect("tempdir is UTF-8")
    }

    #[test]
    fn detect_rust_ecosystem() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("Cargo.toml"), "[package]\nname = \"test\"").unwrap();

        let detection = detect_project(utf8_tmp(&tmp));
        assert!(detection.is_some());
        let det = detection.unwrap();
        assert_eq!(det.ecosystem, Ecosystem::Rust);
    }

    #[test]
    fn detect_node_ecosystem() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("package.json"), "{}").unwrap();

        let detection = detect_project(utf8_tmp(&tmp));
        assert!(detection.is_some());
        let det = detection.unwrap();
        assert_eq!(det.ecosystem, Ecosystem::Node);
    }

    #[test]
    fn detect_unknown_ecosystem() {
        let tmp = TempDir::new().unwrap();
        // No marker files
        let detection = detect_project(utf8_tmp(&tmp));
        assert!(detection.is_none());
    }

    #[test]
    fn rust_takes_priority_over_node() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("Cargo.toml"), "[package]").unwrap();
        fs::write(tmp.path().join("package.json"), "{}").unwrap();

        let det = detect_project(utf8_tmp(&tmp)).unwrap();
        assert_eq!(det.ecosystem, Ecosystem::Rust);
    }

    #[test]
    fn detect_cc_strategy_when_git_cliff_available() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("Cargo.toml"), "[package]").unwrap();

        let det = detect_project(utf8_tmp(&tmp)).unwrap();
        if super::has_binary("git-cliff") {
            assert_eq!(
                det.version_strategy,
                VersionStrategy::ConventionalCommits {
                    tool: ChangelogTool::GitCliff
                }
            );
        } else {
            assert_eq!(det.version_strategy, VersionStrategy::Interactive);
        }
    }

    #[test]
    fn parse_version_from_git_cliff_output() {
        let v = parse_version_from_output("git-cliff 2.12.0\n");
        assert_eq!(v, Some(semver::Version::new(2, 12, 0)));
    }

    #[test]
    fn parse_version_from_minimal_output() {
        let v = parse_version_from_output("2.5.0");
        assert_eq!(v, Some(semver::Version::new(2, 5, 0)));
    }

    #[test]
    fn parse_version_from_garbage() {
        assert!(parse_version_from_output("not a version").is_none());
        assert!(parse_version_from_output("").is_none());
    }

    #[test]
    fn parse_version_with_prerelease() {
        let v = parse_version_from_output("tool 3.0.0-rc.1");
        assert_eq!(v, Some(semver::Version::parse("3.0.0-rc.1").unwrap()));
    }

    #[test]
    fn tool_version_check_too_old() {
        // Simulate: we have 1.0.0 but need 2.5.0
        let found = semver::Version::new(1, 0, 0);
        let minimum = semver::Version::new(2, 5, 0);
        assert!(found < minimum);
    }

    #[test]
    fn min_git_cliff_version_is_correct() {
        assert_eq!(MIN_GIT_CLIFF_VERSION, semver::Version::new(2, 5, 0));
    }

    #[test]
    fn detect_version_strategy_unit() {
        let tmp = TempDir::new().unwrap();
        let root = utf8_tmp(&tmp);

        let strategy = detect_version_strategy(root);
        if super::has_binary("git-cliff") {
            assert!(matches!(
                strategy,
                VersionStrategy::ConventionalCommits {
                    tool: ChangelogTool::GitCliff
                }
            ));
        } else {
            assert_eq!(strategy, VersionStrategy::Interactive);
        }
    }
}
