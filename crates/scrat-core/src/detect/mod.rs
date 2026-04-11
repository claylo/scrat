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

mod rust;

use std::process::Command;

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
/// files). [`Ecosystem::Generic`] is never auto-detected.
fn detect_ecosystem(project_root: &Utf8Path) -> Option<Ecosystem> {
    for ecosystem in Ecosystem::AUTO_DETECTABLE {
        if let Some(marker) = ecosystem.marker_file()
            && project_root.join(marker).is_file()
        {
            return Some(*ecosystem);
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

/// Detect Node.js tooling. Probes for `npm`/`yarn`/`pnpm` and picks a
/// sensible package manager for test/build/publish. The version bump is
/// always a direct `package.json` edit — scrat is not a lockfile manager.
fn detect_node(_project_root: &Utf8Path, version_strategy: VersionStrategy) -> ProjectDetection {
    use crate::ecosystem::DetectedTools;

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

/// Detect Go tooling. Probes for `go` on PATH.
fn detect_go(_project_root: &Utf8Path, version_strategy: VersionStrategy) -> ProjectDetection {
    use crate::ecosystem::DetectedTools;

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

/// Detect PHP/Composer tooling. Probes for `composer` on PATH.
fn detect_php(_project_root: &Utf8Path, version_strategy: VersionStrategy) -> ProjectDetection {
    use crate::ecosystem::DetectedTools;

    let has_composer = has_binary("composer");
    debug!(has_composer, "probed PHP tools");

    let changelog_tool = version_strategy.changelog_tool();
    ProjectDetection {
        ecosystem: Ecosystem::Php,
        version_strategy,
        tools: DetectedTools {
            test_cmd: if has_composer {
                "composer test".into()
            } else {
                String::new()
            },
            build_cmd: String::new(),
            publish_cmd: None,
            bump_cmd: None, // PHP bump is done directly in composer.json
            changelog_tool,
        },
    }
}

/// Detect Python tooling. Probes for `pytest`, `uv`, `python`, and `twine`.
fn detect_python(_project_root: &Utf8Path, version_strategy: VersionStrategy) -> ProjectDetection {
    use crate::ecosystem::DetectedTools;

    let has_uv = has_binary("uv");
    let has_pytest = has_binary("pytest");
    let has_python = has_binary("python3") || has_binary("python");
    let has_twine = has_binary("twine");
    debug!(
        has_uv,
        has_pytest, has_python, has_twine, "probed Python tools"
    );

    let test_cmd = if has_uv {
        "uv run pytest".into()
    } else if has_pytest {
        "pytest".into()
    } else {
        String::new()
    };
    let build_cmd = if has_uv {
        "uv build".into()
    } else if has_python {
        "python -m build".into()
    } else {
        String::new()
    };
    let publish_cmd = if has_uv {
        Some("uv publish".into())
    } else if has_twine {
        Some("twine upload dist/*".into())
    } else {
        None
    };

    let changelog_tool = version_strategy.changelog_tool();
    ProjectDetection {
        ecosystem: Ecosystem::Python,
        version_strategy,
        tools: DetectedTools {
            test_cmd,
            build_cmd,
            publish_cmd,
            bump_cmd: None, // Python bump is done directly in pyproject.toml
            changelog_tool,
        },
    }
}

/// Detect Ruby tooling. Probes for `bundle`, `rake`, and `gem`.
fn detect_ruby(_project_root: &Utf8Path, version_strategy: VersionStrategy) -> ProjectDetection {
    use crate::ecosystem::DetectedTools;

    let has_bundle = has_binary("bundle");
    let has_rake = has_binary("rake");
    let has_gem = has_binary("gem");
    debug!(has_bundle, has_rake, has_gem, "probed Ruby tools");

    let test_cmd = if has_bundle && has_rake {
        "bundle exec rake test".into()
    } else if has_rake {
        "rake test".into()
    } else {
        String::new()
    };
    let build_cmd = if has_gem {
        "gem build".into()
    } else {
        String::new()
    };
    let publish_cmd = has_gem.then(|| "gem push".to_string());

    let changelog_tool = version_strategy.changelog_tool();
    ProjectDetection {
        ecosystem: Ecosystem::Ruby,
        version_strategy,
        tools: DetectedTools {
            test_cmd,
            build_cmd,
            publish_cmd,
            bump_cmd: None, // handled via lib/**/version.rb + gemspec
            changelog_tool,
        },
    }
}

/// Detect Swift tooling. Probes for `swift` on PATH.
fn detect_swift(_project_root: &Utf8Path, version_strategy: VersionStrategy) -> ProjectDetection {
    use crate::ecosystem::DetectedTools;

    let has_swift = has_binary("swift");
    debug!(has_swift, "probed Swift tools");

    let changelog_tool = version_strategy.changelog_tool();
    ProjectDetection {
        ecosystem: Ecosystem::Swift,
        version_strategy,
        tools: DetectedTools {
            test_cmd: if has_swift {
                "swift test".into()
            } else {
                String::new()
            },
            build_cmd: if has_swift {
                "swift build -c release".into()
            } else {
                String::new()
            },
            publish_cmd: None, // SwiftPM publishes via git tags
            bump_cmd: None,
            changelog_tool,
        },
    }
}

/// Build a [`ProjectDetection`] for a user-selected ecosystem.
///
/// Called after the CLI prompts the user to choose an ecosystem when
/// auto-detection returns `None`.
pub fn build_detection(project_root: &Utf8Path, ecosystem: Ecosystem) -> ProjectDetection {
    let version_strategy = detect_version_strategy(project_root);
    build_detection_for(project_root, ecosystem, version_strategy)
}

/// Dispatch an ecosystem to its per-ecosystem detection helper.
///
/// Pure delegation table — every arm forwards to the function that
/// owns that ecosystem's detection logic. Used by `detect_project`,
/// `resolve_detection`, and `build_detection`.
fn build_detection_for(
    project_root: &Utf8Path,
    ecosystem: Ecosystem,
    version_strategy: VersionStrategy,
) -> ProjectDetection {
    match ecosystem {
        Ecosystem::Rust => rust::detect_rust(project_root, version_strategy),
        Ecosystem::Node => detect_node(project_root, version_strategy),
        Ecosystem::Go => detect_go(project_root, version_strategy),
        Ecosystem::Php => detect_php(project_root, version_strategy),
        Ecosystem::Python => detect_python(project_root, version_strategy),
        Ecosystem::Ruby => detect_ruby(project_root, version_strategy),
        Ecosystem::Swift => detect_swift(project_root, version_strategy),
        Ecosystem::Generic => ProjectDetection::generic(version_strategy),
    }
}

/// Check whether a binary is available on `PATH`.
pub fn has_binary(name: &str) -> bool {
    which::which(name).is_ok()
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
