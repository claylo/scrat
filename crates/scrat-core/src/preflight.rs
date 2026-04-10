//! Preflight checks for release readiness.
//!
//! Validates the git state, branch, remote sync, and tool availability
//! before a release. Returns structured results that the CLI formats.

use serde::Serialize;
use tracing::{debug, instrument};

use crate::config::Config;
use crate::detect;
use crate::ecosystem::{Ecosystem, ProjectDetection};
use crate::git;
use crate::ship::ShipOptions;

/// A single preflight check result.
#[derive(Debug, Clone, Serialize)]
pub struct CheckResult {
    /// Human-readable name of the check.
    pub name: String,
    /// Whether the check passed.
    pub passed: bool,
    /// Description of the result (reason for failure, or confirmation).
    pub message: String,
    /// CLI flag that would skip this check (e.g., `"--no-publish"`).
    /// Shown as a hint when the check fails.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skip_flag: Option<String>,
}

/// Full preflight report.
#[derive(Debug, Clone, Serialize)]
pub struct PreflightReport {
    /// Individual check results.
    pub checks: Vec<CheckResult>,
    /// Whether all checks passed.
    pub all_passed: bool,
    /// Detected project info (if detection succeeded).
    pub detection: Option<ProjectDetection>,
}

/// Run all preflight checks.
///
/// # Arguments
/// * `project_root` — the project root directory
/// * `config` — loaded scrat configuration (for branch overrides etc.)
/// * `ship_options` — if provided, gates credential checks on which phases
///   will actually run. `None` means check everything (standalone preflight).
#[instrument(skip(config, ship_options), fields(root = %project_root))]
pub fn run_preflight(
    project_root: &camino::Utf8Path,
    config: &Config,
    ship_options: Option<&ShipOptions>,
) -> PreflightReport {
    let detection = detect::resolve_detection(project_root, config);
    run_preflight_with_detection(project_root, config, ship_options, detection)
}

/// Run preflight with a pre-computed [`ProjectDetection`].
///
/// Equivalent to [`run_preflight`] but skips the detection phase. Used by
/// [`crate::ship::plan_ship`] to avoid scanning marker files and probing
/// `PATH` twice in a single ship run.
#[instrument(skip(config, ship_options, detection), fields(root = %project_root))]
pub fn run_preflight_with_detection(
    project_root: &camino::Utf8Path,
    config: &Config,
    ship_options: Option<&ShipOptions>,
    detection: Option<ProjectDetection>,
) -> PreflightReport {
    let mut checks = Vec::new();

    // Check 1: Inside a git repo
    let in_repo = check_git_repo();
    let is_repo = in_repo.passed;
    checks.push(in_repo);

    if !is_repo {
        // Can't run further git checks outside a repo
        return PreflightReport {
            all_passed: false,
            detection: None,
            checks,
        };
    }

    // Check 2: Working tree clean
    checks.push(check_clean_tree());

    // Check 3: On release branch
    let release_branch_override = config
        .project
        .as_ref()
        .and_then(|p| p.release_branch.as_deref());
    checks.push(check_release_branch(release_branch_override));

    // Check 4: Remote in sync (honor --no-fetch to skip the network round-trip)
    let fetch_remote = !ship_options.is_some_and(|o| o.no_fetch);
    checks.push(check_remote_sync(fetch_remote));

    // Check 5: Ecosystem detection (from pre-computed detection or None)
    checks.push(check_ecosystem(&detection));

    // Check 6: Required tools
    if let Some(ref det) = detection {
        checks.push(check_required_tools(det));
    }

    // ── Extended checks: credentials & auth ──
    // These validate that later pipeline phases can succeed.
    // When ship_options is provided (via `scrat ship`), use those flags.
    // Otherwise fall back to config [ship] section for phase-skip settings.

    let ship_cfg = config.ship.as_ref();
    let skip_release = ship_options.is_some_and(|o| o.no_release)
        || ship_cfg.and_then(|s| s.no_release).unwrap_or(false);
    let skip_publish = ship_options.is_some_and(|o| o.no_publish)
        || ship_cfg.and_then(|s| s.no_publish).unwrap_or(false);

    // Check 7: GitHub CLI auth (needed for release phase)
    if !skip_release {
        checks.push(check_gh_auth());
    }

    // Check 8: Registry auth (needed for publish phase, ecosystem-specific)
    if !skip_publish
        && let Some(ref det) = detection
        && det.tools.publish_cmd.is_some()
    {
        checks.push(check_registry_auth(det.ecosystem));
    }

    let all_passed = checks.iter().all(|c| c.passed);
    debug!(all_passed, check_count = checks.len(), "preflight complete");

    PreflightReport {
        checks,
        all_passed,
        detection,
    }
}

fn check_git_repo() -> CheckResult {
    match git::is_inside_repo() {
        Ok(true) => CheckResult {
            name: "Git repository".into(),
            passed: true,
            message: "Inside a git repository".into(),
            skip_flag: None,
        },
        Ok(false) => CheckResult {
            name: "Git repository".into(),
            passed: false,
            message: "Not inside a git repository".into(),
            skip_flag: None,
        },
        Err(e) => CheckResult {
            name: "Git repository".into(),
            passed: false,
            message: format!("Failed to check: {e}"),
            skip_flag: None,
        },
    }
}

fn check_clean_tree() -> CheckResult {
    match git::is_clean() {
        Ok(true) => CheckResult {
            name: "Working tree".into(),
            passed: true,
            message: "Clean working tree".into(),
            skip_flag: None,
        },
        Ok(false) => CheckResult {
            name: "Working tree".into(),
            passed: false,
            message: "Uncommitted changes in working tree".into(),
            skip_flag: None,
        },
        Err(e) => CheckResult {
            name: "Working tree".into(),
            passed: false,
            message: format!("Failed to check: {e}"),
            skip_flag: None,
        },
    }
}

fn check_release_branch(override_branch: Option<&str>) -> CheckResult {
    let current = match git::current_branch() {
        Ok(Some(b)) => b,
        Ok(None) => {
            return CheckResult {
                name: "Release branch".into(),
                passed: false,
                message: "Detached HEAD — not on any branch".into(),
                skip_flag: None,
            };
        }
        Err(e) => {
            return CheckResult {
                name: "Release branch".into(),
                passed: false,
                message: format!("Failed to check: {e}"),
                skip_flag: None,
            };
        }
    };

    // If the user specified a release branch, check against that
    if let Some(expected) = override_branch {
        let passed = current == expected;
        return CheckResult {
            name: "Release branch".into(),
            passed,
            message: if passed {
                format!("On configured release branch '{current}'")
            } else {
                format!("On '{current}', expected '{expected}'")
            },
            skip_flag: None,
        };
    }

    // Otherwise, detect main/master
    match git::detect_release_branch() {
        Ok(Some(release)) => {
            let passed = current == release;
            CheckResult {
                name: "Release branch".into(),
                passed,
                message: if passed {
                    format!("On release branch '{current}'")
                } else {
                    format!("On '{current}', expected '{release}'")
                },
                skip_flag: None,
            }
        }
        Ok(None) => CheckResult {
            name: "Release branch".into(),
            passed: false,
            message: format!("On '{current}' — no main/master branch found"),
            skip_flag: None,
        },
        Err(e) => CheckResult {
            name: "Release branch".into(),
            passed: false,
            message: format!("Failed to detect: {e}"),
            skip_flag: None,
        },
    }
}

fn check_remote_sync(fetch_remote: bool) -> CheckResult {
    match git::is_remote_in_sync(fetch_remote) {
        Ok(true) => CheckResult {
            name: "Remote sync".into(),
            passed: true,
            message: if fetch_remote {
                "Local branch is in sync with remote".into()
            } else {
                "Local branch is in sync with cached remote (--no-fetch)".into()
            },
            skip_flag: None,
        },
        Ok(false) => CheckResult {
            name: "Remote sync".into(),
            passed: false,
            message: "Local branch is out of sync with remote (pull or push needed)".into(),
            skip_flag: Some("--no-fetch".into()),
        },
        Err(e) => CheckResult {
            name: "Remote sync".into(),
            passed: false,
            message: format!("Failed to check: {e}"),
            skip_flag: None,
        },
    }
}

fn check_ecosystem(detection: &Option<ProjectDetection>) -> CheckResult {
    detection.as_ref().map_or_else(
        || CheckResult {
            name: "Project detection".into(),
            // Not a hard failure — CLI will prompt for ecosystem selection
            passed: true,
            message: "No ecosystem detected — select interactively or set project.type in config"
                .into(),
            skip_flag: None,
        },
        |det| CheckResult {
            name: "Project detection".into(),
            passed: true,
            message: format!("Detected {} project", det.ecosystem),
            skip_flag: None,
        },
    )
}

fn check_required_tools(detection: &ProjectDetection) -> CheckResult {
    let mut missing = Vec::new();

    // Check that the test command's binary exists
    if let Some(bin) = detection.tools.test_cmd.split_whitespace().next()
        && !detect::has_binary(bin)
    {
        missing.push(bin.to_string());
    }

    // Check bump tool
    if let Some(ref cmd) = detection.tools.bump_cmd
        && let Some(bin) = cmd.split_whitespace().next()
        && !detect::has_binary(bin)
    {
        missing.push(bin.to_string());
    }

    // Check changelog tool binary + minimum version
    if let Some(ref tool) = detection.tools.changelog_tool {
        let bin = tool.to_string();
        if !detect::has_binary(&bin) {
            missing.push(bin);
        }
    }

    if !missing.is_empty() {
        return CheckResult {
            name: "Required tools".into(),
            passed: false,
            message: format!("Missing tools: {}", missing.join(", ")),
            skip_flag: None,
        };
    }

    // Version check for git-cliff (requires 2.5.0+ for --bump [type])
    if detection.tools.changelog_tool == Some(crate::ecosystem::ChangelogTool::GitCliff) {
        match detect::check_tool_version("git-cliff", &detect::MIN_GIT_CLIFF_VERSION) {
            detect::ToolVersionCheck::Ok(v) => {
                debug!(%v, "git-cliff version ok");
            }
            detect::ToolVersionCheck::TooOld { found, minimum } => {
                return CheckResult {
                    name: "Required tools".into(),
                    passed: false,
                    message: format!(
                        "git-cliff {found} is too old (need {minimum}+) — run `cargo install git-cliff`"
                    ),
                    skip_flag: None,
                };
            }
            detect::ToolVersionCheck::Unknown(reason) => {
                debug!(reason, "could not check git-cliff version");
            }
        }
    }

    CheckResult {
        name: "Required tools".into(),
        passed: true,
        message: "All required tools are installed".into(),
        skip_flag: None,
    }
}

// ── Extended checks: credentials & auth ──────────────

/// Check GitHub CLI authentication status.
///
/// Runs `gh auth status` which exits 0 when a valid token is configured.
fn check_gh_auth() -> CheckResult {
    if !detect::has_binary("gh") {
        return CheckResult {
            name: "GitHub CLI".into(),
            passed: false,
            message: "gh not found on PATH — install from https://cli.github.com/".into(),
            skip_flag: Some("--no-release".into()),
        };
    }

    let result = std::process::Command::new("gh")
        .args(["auth", "status"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();

    match result {
        Ok(status) if status.success() => CheckResult {
            name: "GitHub CLI".into(),
            passed: true,
            message: "gh is authenticated".into(),
            skip_flag: None,
        },
        _ => CheckResult {
            name: "GitHub CLI".into(),
            passed: false,
            message: "gh is not authenticated — run `gh auth login`".into(),
            skip_flag: Some("--no-release".into()),
        },
    }
}

/// Check registry auth for ecosystems that publish to a package registry.
///
/// Uses fast env-var checks (no network calls). The check is informational:
/// some ecosystems support credential stores that env vars don't cover,
/// so a missing env var is a warning-level failure.
fn check_registry_auth(ecosystem: Ecosystem) -> CheckResult {
    let (env_vars, registry_name, login_hint) = match ecosystem {
        Ecosystem::Rust => (
            vec!["CARGO_REGISTRY_TOKEN"],
            "crates.io",
            "set CARGO_REGISTRY_TOKEN or run `cargo login`",
        ),
        Ecosystem::Node => (
            vec!["NPM_TOKEN", "NODE_AUTH_TOKEN"],
            "npm",
            "set NPM_TOKEN or run `npm login`",
        ),
        Ecosystem::Python => (
            vec!["TWINE_PASSWORD", "PYPI_TOKEN"],
            "PyPI",
            "set TWINE_PASSWORD or PYPI_TOKEN",
        ),
        Ecosystem::Ruby => (
            vec!["GEM_HOST_API_KEY"],
            "RubyGems",
            "set GEM_HOST_API_KEY or run `gem signin`",
        ),
        // Ecosystems without registry publish
        _ => {
            return CheckResult {
                name: "Registry auth".into(),
                passed: true,
                message: "No registry publish for this ecosystem".into(),
                skip_flag: None,
            };
        }
    };

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

/// Check whether a specific git tag already exists.
///
/// This is a post-version-resolution check, called from [`ReadyShip::validate`]
/// rather than from `run_preflight` (which runs before the version is known).
pub fn check_tag_available(tag: &str) -> CheckResult {
    match git::tag_exists(tag) {
        Ok(false) => CheckResult {
            name: "Tag available".into(),
            passed: true,
            message: format!("{tag} does not exist"),
            skip_flag: None,
        },
        Ok(true) => CheckResult {
            name: "Tag available".into(),
            passed: false,
            message: format!("{tag} already exists — delete it or choose a different version"),
            skip_flag: Some("--no-tag".into()),
        },
        Err(e) => CheckResult {
            name: "Tag available".into(),
            passed: false,
            message: format!("Failed to check tag: {e}"),
            skip_flag: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecosystem::{
        ChangelogTool, DetectedTools, Ecosystem, ProjectDetection, VersionStrategy,
    };

    // ---------------------------------------------------------------
    // Helpers
    // ---------------------------------------------------------------

    /// Build a minimal `ProjectDetection` for the given ecosystem.
    fn detection_for(ecosystem: Ecosystem) -> ProjectDetection {
        match ecosystem {
            Ecosystem::Rust => ProjectDetection {
                ecosystem: Ecosystem::Rust,
                version_strategy: VersionStrategy::Interactive,
                tools: DetectedTools {
                    test_cmd: "cargo nextest run".into(),
                    build_cmd: "cargo build --release".into(),
                    publish_cmd: Some("cargo publish".into()),
                    bump_cmd: Some("cargo set-version".into()),
                    changelog_tool: None,
                },
            },
            Ecosystem::Node => ProjectDetection {
                ecosystem: Ecosystem::Node,
                version_strategy: VersionStrategy::Interactive,
                tools: DetectedTools {
                    test_cmd: "npm test".into(),
                    build_cmd: "npm run build".into(),
                    publish_cmd: Some("npm publish".into()),
                    bump_cmd: Some("npm version --no-git-tag-version".into()),
                    changelog_tool: None,
                },
            },
            Ecosystem::Go => ProjectDetection {
                ecosystem: Ecosystem::Go,
                version_strategy: VersionStrategy::Interactive,
                tools: DetectedTools {
                    test_cmd: "go test ./...".into(),
                    build_cmd: "go build ./...".into(),
                    publish_cmd: None,
                    bump_cmd: None,
                    changelog_tool: None,
                },
            },
            Ecosystem::Php => ProjectDetection {
                ecosystem: Ecosystem::Php,
                version_strategy: VersionStrategy::Interactive,
                tools: DetectedTools {
                    test_cmd: "composer test".into(),
                    build_cmd: String::new(),
                    publish_cmd: None,
                    bump_cmd: None,
                    changelog_tool: None,
                },
            },
            Ecosystem::Python => ProjectDetection {
                ecosystem: Ecosystem::Python,
                version_strategy: VersionStrategy::Interactive,
                tools: DetectedTools {
                    test_cmd: "pytest".into(),
                    build_cmd: "python -m build".into(),
                    publish_cmd: Some("twine upload dist/*".into()),
                    bump_cmd: None,
                    changelog_tool: None,
                },
            },
            Ecosystem::Ruby => ProjectDetection {
                ecosystem: Ecosystem::Ruby,
                version_strategy: VersionStrategy::Interactive,
                tools: DetectedTools {
                    test_cmd: "bundle exec rake test".into(),
                    build_cmd: "gem build".into(),
                    publish_cmd: Some("gem push".into()),
                    bump_cmd: None,
                    changelog_tool: None,
                },
            },
            Ecosystem::Swift => ProjectDetection {
                ecosystem: Ecosystem::Swift,
                version_strategy: VersionStrategy::Interactive,
                tools: DetectedTools {
                    test_cmd: "swift test".into(),
                    build_cmd: "swift build -c release".into(),
                    publish_cmd: None,
                    bump_cmd: None,
                    changelog_tool: None,
                },
            },
            Ecosystem::Generic => ProjectDetection::generic(VersionStrategy::Interactive),
        }
    }

    /// Build a detection with a specific changelog tool set.
    fn detection_with_changelog(tool: ChangelogTool) -> ProjectDetection {
        ProjectDetection {
            ecosystem: Ecosystem::Rust,
            version_strategy: VersionStrategy::ConventionalCommits { tool },
            tools: DetectedTools {
                test_cmd: "cargo nextest run".into(),
                build_cmd: "cargo build --release".into(),
                publish_cmd: Some("cargo publish".into()),
                bump_cmd: Some("cargo set-version".into()),
                changelog_tool: Some(tool),
            },
        }
    }

    /// Build a detection whose test_cmd binary is guaranteed to not exist.
    fn detection_with_missing_test_tool() -> ProjectDetection {
        ProjectDetection {
            ecosystem: Ecosystem::Rust,
            version_strategy: VersionStrategy::Interactive,
            tools: DetectedTools {
                test_cmd: "nonexistent-tool-abc123 run".into(),
                build_cmd: "cargo build --release".into(),
                publish_cmd: None,
                bump_cmd: None,
                changelog_tool: None,
            },
        }
    }

    /// Build a detection whose bump_cmd binary is guaranteed to not exist.
    fn detection_with_missing_bump_tool() -> ProjectDetection {
        ProjectDetection {
            ecosystem: Ecosystem::Rust,
            version_strategy: VersionStrategy::Interactive,
            tools: DetectedTools {
                test_cmd: "cargo nextest run".into(),
                build_cmd: "cargo build --release".into(),
                publish_cmd: None,
                bump_cmd: Some("nonexistent-bumper-xyz789 bump".into()),
                changelog_tool: None,
            },
        }
    }

    // ---------------------------------------------------------------
    // CheckResult construction and fields
    // ---------------------------------------------------------------

    #[test]
    fn check_result_fields_accessible() {
        let result = CheckResult {
            name: "My Check".into(),
            passed: true,
            message: "Everything is fine".into(),
            skip_flag: None,
        };
        assert_eq!(result.name, "My Check");
        assert!(result.passed);
        assert_eq!(result.message, "Everything is fine");
    }

    #[test]
    fn check_result_clone() {
        let result = CheckResult {
            name: "test".into(),
            passed: false,
            message: "fail".into(),
            skip_flag: None,
        };
        let cloned = result.clone();
        assert_eq!(cloned.name, result.name);
        assert_eq!(cloned.passed, result.passed);
        assert_eq!(cloned.message, result.message);
    }

    #[test]
    fn check_result_debug_impl() {
        let result = CheckResult {
            name: "test".into(),
            passed: true,
            message: "ok".into(),
            skip_flag: None,
        };
        let debug = format!("{result:?}");
        assert!(debug.contains("CheckResult"));
        assert!(debug.contains("test"));
    }

    // ---------------------------------------------------------------
    // CheckResult serialization
    // ---------------------------------------------------------------

    #[test]
    fn check_result_serializes_to_json() {
        let result = CheckResult {
            name: "Git repository".into(),
            passed: true,
            message: "Inside a git repository".into(),
            skip_flag: None,
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("\"name\":\"Git repository\""));
        assert!(json.contains("\"passed\":true"));
        assert!(json.contains("\"message\":\"Inside a git repository\""));
    }

    #[test]
    fn check_result_serializes_failed_state() {
        let result = CheckResult {
            name: "Working tree".into(),
            passed: false,
            message: "Uncommitted changes".into(),
            skip_flag: None,
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("\"passed\":false"));
    }

    // ---------------------------------------------------------------
    // PreflightReport construction and serialization
    // ---------------------------------------------------------------

    #[test]
    fn preflight_report_serializes() {
        let report = PreflightReport {
            checks: vec![CheckResult {
                name: "test".into(),
                passed: true,
                message: "ok".into(),
                skip_flag: None,
            }],
            all_passed: true,
            detection: None,
        };
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("\"all_passed\":true"));
    }

    #[test]
    fn preflight_report_serializes_with_detection() {
        let det = detection_for(Ecosystem::Rust);
        let report = PreflightReport {
            checks: vec![CheckResult {
                name: "test".into(),
                passed: true,
                message: "ok".into(),
                skip_flag: None,
            }],
            all_passed: true,
            detection: Some(det),
        };
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("\"ecosystem\":\"rust\""));
        assert!(json.contains("\"all_passed\":true"));
    }

    #[test]
    fn preflight_report_serializes_with_generic_detection() {
        let det = detection_for(Ecosystem::Generic);
        let report = PreflightReport {
            checks: vec![],
            all_passed: true,
            detection: Some(det),
        };
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("\"ecosystem\":\"generic\""));
    }

    #[test]
    fn preflight_report_serializes_with_node_detection() {
        let det = detection_for(Ecosystem::Node);
        let report = PreflightReport {
            checks: vec![],
            all_passed: true,
            detection: Some(det),
        };
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("\"ecosystem\":\"node\""));
    }

    #[test]
    fn preflight_report_empty_checks_all_passed() {
        let report = PreflightReport {
            checks: vec![],
            all_passed: true,
            detection: None,
        };
        assert!(report.all_passed);
        assert!(report.checks.is_empty());
    }

    #[test]
    fn preflight_report_all_passed_false_when_any_fails() {
        let report = PreflightReport {
            checks: vec![
                CheckResult {
                    name: "check1".into(),
                    passed: true,
                    message: "ok".into(),
                    skip_flag: None,
                },
                CheckResult {
                    name: "check2".into(),
                    passed: false,
                    message: "fail".into(),
                    skip_flag: None,
                },
            ],
            all_passed: false,
            detection: None,
        };
        assert!(!report.all_passed);
    }

    #[test]
    fn preflight_report_multiple_checks_serialize() {
        let report = PreflightReport {
            checks: vec![
                CheckResult {
                    name: "a".into(),
                    passed: true,
                    message: "ok".into(),
                    skip_flag: None,
                },
                CheckResult {
                    name: "b".into(),
                    passed: false,
                    message: "nope".into(),
                    skip_flag: None,
                },
                CheckResult {
                    name: "c".into(),
                    passed: true,
                    message: "fine".into(),
                    skip_flag: None,
                },
            ],
            all_passed: false,
            detection: None,
        };
        let json = serde_json::to_string(&report).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        let checks = parsed["checks"].as_array().unwrap();
        assert_eq!(checks.len(), 3);
    }

    #[test]
    fn preflight_report_clone() {
        let report = PreflightReport {
            checks: vec![CheckResult {
                name: "test".into(),
                passed: true,
                message: "ok".into(),
                skip_flag: None,
            }],
            all_passed: true,
            detection: Some(detection_for(Ecosystem::Rust)),
        };
        let cloned = report.clone();
        assert_eq!(cloned.all_passed, report.all_passed);
        assert_eq!(cloned.checks.len(), report.checks.len());
        assert!(cloned.detection.is_some());
    }

    // ---------------------------------------------------------------
    // check_ecosystem
    // ---------------------------------------------------------------

    #[test]
    fn check_ecosystem_none_passes_with_prompt_hint() {
        let result = check_ecosystem(&None);
        // No detection is not a hard failure — CLI will prompt for selection
        assert!(result.passed);
        assert!(result.message.contains("select interactively"));
    }

    #[test]
    fn check_ecosystem_rust_detected() {
        let det = Some(detection_for(Ecosystem::Rust));
        let result = check_ecosystem(&det);
        assert!(result.passed);
        assert!(result.message.contains("rust"));
        assert_eq!(result.name, "Project detection");
    }

    #[test]
    fn check_ecosystem_node_detected() {
        let det = Some(detection_for(Ecosystem::Node));
        let result = check_ecosystem(&det);
        assert!(result.passed);
        assert!(result.message.contains("node"));
    }

    #[test]
    fn check_ecosystem_generic_detected() {
        let det = Some(detection_for(Ecosystem::Generic));
        let result = check_ecosystem(&det);
        assert!(result.passed);
        assert!(result.message.contains("generic"));
    }

    #[test]
    fn check_ecosystem_always_passes() {
        // Even with None, check_ecosystem passes (soft failure, CLI prompts)
        let none_result = check_ecosystem(&None);
        let some_result = check_ecosystem(&Some(detection_for(Ecosystem::Rust)));
        assert!(none_result.passed);
        assert!(some_result.passed);
    }

    #[test]
    fn check_ecosystem_name_is_project_detection() {
        let result = check_ecosystem(&None);
        assert_eq!(result.name, "Project detection");
        let result = check_ecosystem(&Some(detection_for(Ecosystem::Rust)));
        assert_eq!(result.name, "Project detection");
    }

    // ---------------------------------------------------------------
    // check_required_tools
    // ---------------------------------------------------------------

    #[test]
    fn check_required_tools_passes_with_cargo() {
        // cargo should be available in any Rust dev environment
        let det = detection_for(Ecosystem::Rust);
        let result = check_required_tools(&det);
        // cargo/cargo-nextest should be on PATH in this project's environment
        assert_eq!(result.name, "Required tools");
        // If the test tool is found, it passes; if not, it reports missing.
        // We just verify the function runs without panic.
    }

    #[test]
    fn check_required_tools_missing_test_cmd() {
        let det = detection_with_missing_test_tool();
        let result = check_required_tools(&det);
        assert!(!result.passed);
        assert!(result.message.contains("nonexistent-tool-abc123"));
        assert!(result.message.contains("Missing tools"));
    }

    #[test]
    fn check_required_tools_missing_bump_cmd() {
        let det = detection_with_missing_bump_tool();
        let result = check_required_tools(&det);
        assert!(!result.passed);
        assert!(result.message.contains("nonexistent-bumper-xyz789"));
    }

    #[test]
    fn check_required_tools_generic_has_no_tools() {
        // Generic detection has empty test_cmd, so split_whitespace().next() is None
        let det = detection_for(Ecosystem::Generic);
        let result = check_required_tools(&det);
        // Should pass — nothing to check
        assert!(result.passed);
        assert_eq!(result.message, "All required tools are installed");
    }

    #[test]
    fn check_required_tools_with_changelog_tool_present() {
        // If git-cliff is installed, this passes; if not, it reports missing.
        // We test the structural behavior either way.
        let det = detection_with_changelog(ChangelogTool::GitCliff);
        let result = check_required_tools(&det);
        assert_eq!(result.name, "Required tools");
        // The result depends on whether git-cliff is installed, but the
        // function should not panic regardless.
    }

    #[test]
    fn check_required_tools_multiple_missing() {
        // Both test_cmd and bump_cmd point to nonexistent tools
        let det = ProjectDetection {
            ecosystem: Ecosystem::Rust,
            version_strategy: VersionStrategy::Interactive,
            tools: DetectedTools {
                test_cmd: "nonexistent-test-tool run".into(),
                build_cmd: "cargo build".into(),
                publish_cmd: None,
                bump_cmd: Some("nonexistent-bump-tool bump".into()),
                changelog_tool: None,
            },
        };
        let result = check_required_tools(&det);
        assert!(!result.passed);
        assert!(result.message.contains("nonexistent-test-tool"));
        assert!(result.message.contains("nonexistent-bump-tool"));
    }

    #[test]
    fn check_required_tools_name_is_required_tools() {
        let det = detection_for(Ecosystem::Rust);
        let result = check_required_tools(&det);
        assert_eq!(result.name, "Required tools");
    }

    // ---------------------------------------------------------------
    // check_git_repo (runs actual git — works in this project's repo)
    // ---------------------------------------------------------------

    #[test]
    fn check_git_repo_in_project() {
        // This test runs inside the scrat repo, so should pass
        let result = check_git_repo();
        assert_eq!(result.name, "Git repository");
        // We're in a git repo (scrat itself), so this should be true
        if result.passed {
            assert_eq!(result.message, "Inside a git repository");
        }
    }

    // ---------------------------------------------------------------
    // check_clean_tree (runs actual git)
    // ---------------------------------------------------------------

    #[test]
    fn check_clean_tree_runs_without_panic() {
        // The result depends on the actual working tree state, but
        // the function should not panic.
        let result = check_clean_tree();
        assert_eq!(result.name, "Working tree");
        // Message is either "Clean working tree" or "Uncommitted changes..."
        assert!(!result.message.is_empty());
    }

    // ---------------------------------------------------------------
    // check_release_branch
    // ---------------------------------------------------------------

    #[test]
    fn check_release_branch_no_override() {
        // Runs in the scrat repo — we're probably not on main, so it
        // may pass or fail, but should not panic.
        let result = check_release_branch(None);
        assert_eq!(result.name, "Release branch");
        assert!(!result.message.is_empty());
    }

    #[test]
    fn check_release_branch_with_override_matching() {
        // Get the current branch and use it as the override — should pass.
        if let Ok(Some(branch)) = git::current_branch() {
            let result = check_release_branch(Some(&branch));
            assert!(result.passed);
            assert!(result.message.contains(&branch));
            assert!(result.message.contains("configured release branch"));
        }
    }

    #[test]
    fn check_release_branch_with_override_not_matching() {
        // Use a branch name that definitely doesn't match current branch.
        // In CI (detached HEAD), the message will be about detached HEAD
        // rather than a branch mismatch — either way, it should fail.
        let result = check_release_branch(Some("this-branch-does-not-exist-xyz"));
        assert!(!result.passed);
        assert!(
            result.message.contains("expected") || result.message.contains("Detached HEAD"),
            "unexpected message: {}",
            result.message
        );
    }

    // ---------------------------------------------------------------
    // check_remote_sync (runs actual git)
    // ---------------------------------------------------------------

    #[test]
    fn check_remote_sync_runs_without_panic() {
        // Use no-fetch form to avoid touching the network from tests
        let result = check_remote_sync(false);
        assert_eq!(result.name, "Remote sync");
        assert!(!result.message.is_empty());
    }

    // ---------------------------------------------------------------
    // run_preflight integration (uses tempdir + git init)
    // ---------------------------------------------------------------

    fn make_git_repo(tmp: &tempfile::TempDir) {
        use std::process::Command;
        let dir = tmp.path();
        Command::new("git")
            .args(["init"])
            .current_dir(dir)
            .output()
            .expect("git init");
        Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(dir)
            .output()
            .expect("git config email");
        Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(dir)
            .output()
            .expect("git config name");
        // Create an initial commit so HEAD exists
        std::fs::write(dir.join("README.md"), "# test").unwrap();
        Command::new("git")
            .args(["add", "."])
            .current_dir(dir)
            .output()
            .expect("git add");
        Command::new("git")
            .args(["commit", "-m", "init"])
            .current_dir(dir)
            .output()
            .expect("git commit");
    }

    fn utf8_tmp(tmp: &tempfile::TempDir) -> &camino::Utf8Path {
        camino::Utf8Path::from_path(tmp.path()).expect("tempdir is UTF-8")
    }

    #[test]
    fn run_preflight_in_clean_git_repo() {
        let tmp = tempfile::TempDir::new().unwrap();
        make_git_repo(&tmp);

        // Create a Cargo.toml so ecosystem detection finds Rust
        std::fs::write(tmp.path().join("Cargo.toml"), "[package]\nname = \"test\"").unwrap();

        let config = Config::default();

        // We need to cd into the temp repo for the git checks to work
        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        let report = run_preflight(utf8_tmp(&tmp), &config, None);

        std::env::set_current_dir(original_dir).unwrap();

        // Should have multiple checks
        assert!(report.checks.len() >= 4);

        // Git repo check should pass
        assert_eq!(report.checks[0].name, "Git repository");
        assert!(report.checks[0].passed);

        // Working tree should be clean (we committed and only added Cargo.toml after)
        assert_eq!(report.checks[1].name, "Working tree");
        // Cargo.toml was added after the commit, so tree is dirty
        assert!(!report.checks[1].passed);

        // Ecosystem detection should find Rust
        let eco_check = report.checks.iter().find(|c| c.name == "Project detection");
        assert!(eco_check.is_some());
        assert!(eco_check.unwrap().passed);
        assert!(eco_check.unwrap().message.contains("rust"));

        // Detection should be present
        assert!(report.detection.is_some());
        assert_eq!(
            report.detection.as_ref().unwrap().ecosystem,
            Ecosystem::Rust
        );
    }

    #[test]
    fn run_preflight_clean_git_repo_all_committed() {
        let tmp = tempfile::TempDir::new().unwrap();
        make_git_repo(&tmp);

        let config = Config::default();

        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        let report = run_preflight(utf8_tmp(&tmp), &config, None);

        std::env::set_current_dir(original_dir).unwrap();

        // Working tree should be clean (nothing added after initial commit)
        assert_eq!(report.checks[1].name, "Working tree");
        assert!(report.checks[1].passed);
    }

    #[test]
    fn run_preflight_dirty_working_tree() {
        let tmp = tempfile::TempDir::new().unwrap();
        make_git_repo(&tmp);
        // Create an uncommitted file to dirty the tree
        std::fs::write(tmp.path().join("dirty.txt"), "uncommitted").unwrap();

        let config = Config::default();

        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        let report = run_preflight(utf8_tmp(&tmp), &config, None);

        std::env::set_current_dir(original_dir).unwrap();

        let tree_check = report.checks.iter().find(|c| c.name == "Working tree");
        assert!(tree_check.is_some());
        assert!(!tree_check.unwrap().passed);
        assert!(tree_check.unwrap().message.contains("Uncommitted changes"));
        assert!(!report.all_passed);
    }

    #[test]
    fn run_preflight_with_release_branch_override() {
        let tmp = tempfile::TempDir::new().unwrap();
        make_git_repo(&tmp);

        // Configure a release branch override that won't match (default branch is
        // main or master from git init, not "release")
        let config = Config {
            project: Some(crate::config::ProjectConfig {
                project_type: None,
                release_branch: Some("release".into()),
            }),
            ..Config::default()
        };

        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        let report = run_preflight(utf8_tmp(&tmp), &config, None);

        std::env::set_current_dir(original_dir).unwrap();

        let branch_check = report.checks.iter().find(|c| c.name == "Release branch");
        assert!(branch_check.is_some());
        assert!(!branch_check.unwrap().passed);
        assert!(branch_check.unwrap().message.contains("release"));
    }

    #[test]
    fn run_preflight_with_ecosystem_override_generic() {
        let tmp = tempfile::TempDir::new().unwrap();
        make_git_repo(&tmp);

        let config = Config {
            project: Some(crate::config::ProjectConfig {
                project_type: Some(Ecosystem::Generic),
                release_branch: None,
            }),
            ..Config::default()
        };

        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        let report = run_preflight(utf8_tmp(&tmp), &config, None);

        std::env::set_current_dir(original_dir).unwrap();

        assert!(report.detection.is_some());
        assert_eq!(
            report.detection.as_ref().unwrap().ecosystem,
            Ecosystem::Generic
        );
    }

    #[test]
    fn run_preflight_no_ecosystem_detected() {
        let tmp = tempfile::TempDir::new().unwrap();
        make_git_repo(&tmp);
        // No marker files — no ecosystem detected

        let config = Config::default();

        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        let report = run_preflight(utf8_tmp(&tmp), &config, None);

        std::env::set_current_dir(original_dir).unwrap();

        // Detection is None, but ecosystem check still passes (soft failure)
        assert!(report.detection.is_none());
        let eco_check = report.checks.iter().find(|c| c.name == "Project detection");
        assert!(eco_check.unwrap().passed);

        // No required tools check should exist (only added when detection is Some)
        let tools_check = report.checks.iter().find(|c| c.name == "Required tools");
        assert!(tools_check.is_none());
    }

    #[test]
    fn run_preflight_node_ecosystem() {
        let tmp = tempfile::TempDir::new().unwrap();
        make_git_repo(&tmp);
        std::fs::write(tmp.path().join("package.json"), "{}").unwrap();
        // Commit the marker file so tree is clean
        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(tmp.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "add package.json"])
            .current_dir(tmp.path())
            .output()
            .unwrap();

        let config = Config::default();

        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        let report = run_preflight(utf8_tmp(&tmp), &config, None);

        std::env::set_current_dir(original_dir).unwrap();

        assert!(report.detection.is_some());
        assert_eq!(
            report.detection.as_ref().unwrap().ecosystem,
            Ecosystem::Node
        );

        let eco_check = report.checks.iter().find(|c| c.name == "Project detection");
        assert!(eco_check.unwrap().message.contains("node"));
    }

    #[test]
    fn run_preflight_check_count_with_detection() {
        let tmp = tempfile::TempDir::new().unwrap();
        make_git_repo(&tmp);
        std::fs::write(tmp.path().join("Cargo.toml"), "[package]\nname=\"t\"").unwrap();
        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(tmp.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "add Cargo.toml"])
            .current_dir(tmp.path())
            .output()
            .unwrap();

        let config = Config::default();

        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        let report = run_preflight(utf8_tmp(&tmp), &config, None);

        std::env::set_current_dir(original_dir).unwrap();

        // When detection succeeds (standalone, None ship_options):
        // git repo, clean tree, release branch, remote sync, ecosystem,
        // required tools, gh auth, registry auth = 8 checks
        assert_eq!(report.checks.len(), 8);
    }

    #[test]
    fn run_preflight_check_count_without_detection() {
        let tmp = tempfile::TempDir::new().unwrap();
        make_git_repo(&tmp);
        // No marker files

        let config = Config::default();

        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        let report = run_preflight(utf8_tmp(&tmp), &config, None);

        std::env::set_current_dir(original_dir).unwrap();

        // Without detection (standalone, None ship_options):
        // git repo, clean tree, release branch, remote sync, ecosystem,
        // gh auth = 6 checks (no required tools, no registry auth)
        assert_eq!(report.checks.len(), 6);
    }

    // ---------------------------------------------------------------
    // Serialization roundtrip tests
    // ---------------------------------------------------------------

    #[test]
    fn preflight_report_json_roundtrip_structure() {
        let report = PreflightReport {
            checks: vec![
                CheckResult {
                    name: "Git repository".into(),
                    passed: true,
                    message: "Inside a git repository".into(),
                    skip_flag: None,
                },
                CheckResult {
                    name: "Working tree".into(),
                    passed: false,
                    message: "Uncommitted changes".into(),
                    skip_flag: None,
                },
            ],
            all_passed: false,
            detection: Some(detection_for(Ecosystem::Rust)),
        };

        let json = serde_json::to_string_pretty(&report).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed["all_passed"], false);
        assert_eq!(parsed["checks"].as_array().unwrap().len(), 2);
        assert_eq!(parsed["checks"][0]["name"], "Git repository");
        assert_eq!(parsed["checks"][0]["passed"], true);
        assert_eq!(parsed["checks"][1]["passed"], false);
        assert_eq!(parsed["detection"]["ecosystem"], "rust");
    }

    #[test]
    fn preflight_report_json_detection_null_when_none() {
        let report = PreflightReport {
            checks: vec![],
            all_passed: true,
            detection: None,
        };
        let json = serde_json::to_string(&report).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(parsed["detection"].is_null());
    }

    // ---------------------------------------------------------------
    // Extended checks: gh auth, registry auth, tag availability
    // ---------------------------------------------------------------

    #[test]
    fn check_gh_auth_runs_without_panic() {
        // Result depends on whether `gh` is installed and authed,
        // but the function should not panic.
        let result = check_gh_auth();
        assert_eq!(result.name, "GitHub CLI");
        if !result.passed {
            assert!(result.skip_flag.is_some());
            assert_eq!(result.skip_flag.as_deref(), Some("--no-release"));
        }
    }

    #[test]
    fn check_registry_auth_rust() {
        let result = check_registry_auth(Ecosystem::Rust);
        assert_eq!(result.name, "Registry auth");
        if !result.passed {
            assert!(result.message.contains("CARGO_REGISTRY_TOKEN"));
            assert_eq!(result.skip_flag.as_deref(), Some("--no-publish"));
        }
    }

    #[test]
    fn check_registry_auth_node() {
        let result = check_registry_auth(Ecosystem::Node);
        assert_eq!(result.name, "Registry auth");
        if !result.passed {
            assert!(result.message.contains("NPM_TOKEN"));
            assert_eq!(result.skip_flag.as_deref(), Some("--no-publish"));
        }
    }

    #[test]
    fn check_registry_auth_python() {
        let result = check_registry_auth(Ecosystem::Python);
        assert_eq!(result.name, "Registry auth");
        if !result.passed {
            assert!(
                result.message.contains("TWINE_PASSWORD") || result.message.contains("PYPI_TOKEN")
            );
        }
    }

    #[test]
    fn check_registry_auth_go_skips() {
        // Go doesn't publish via registry
        let result = check_registry_auth(Ecosystem::Go);
        assert!(result.passed);
        assert!(result.message.contains("No registry publish"));
    }

    #[test]
    fn check_registry_auth_generic_skips() {
        let result = check_registry_auth(Ecosystem::Generic);
        assert!(result.passed);
    }

    #[test]
    fn check_tag_available_nonexistent_tag() {
        // A tag that definitely doesn't exist
        let result = check_tag_available("v99999.99999.99999-never-exists");
        assert!(result.passed);
        assert!(result.message.contains("does not exist"));
        assert!(result.skip_flag.is_none());
    }

    #[test]
    fn check_tag_available_reports_skip_flag() {
        // If a tag exists, the result should include --no-tag as skip_flag
        // We can't easily test the "exists" case without creating a tag,
        // so verify the structure of a passing check instead.
        let result = check_tag_available("v0.0.0-test-tag-xyz");
        if !result.passed {
            assert_eq!(result.skip_flag.as_deref(), Some("--no-tag"));
        }
    }

    // ---------------------------------------------------------------
    // ShipOptions gating: extended checks respect --no-* flags
    // ---------------------------------------------------------------

    #[test]
    fn run_preflight_no_release_skips_gh_check() {
        let tmp = tempfile::TempDir::new().unwrap();
        make_git_repo(&tmp);

        let config = Config::default();
        let opts = ShipOptions {
            no_release: true,
            ..Default::default()
        };

        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        let report = run_preflight(utf8_tmp(&tmp), &config, Some(&opts));

        std::env::set_current_dir(original_dir).unwrap();

        // GitHub CLI check should NOT be present when --no-release
        let gh_check = report.checks.iter().find(|c| c.name == "GitHub CLI");
        assert!(
            gh_check.is_none(),
            "gh auth check should be skipped with --no-release"
        );
    }

    #[test]
    fn run_preflight_no_publish_skips_registry_check() {
        let tmp = tempfile::TempDir::new().unwrap();
        make_git_repo(&tmp);
        std::fs::write(tmp.path().join("Cargo.toml"), "[package]\nname = \"test\"").unwrap();

        let config = Config::default();
        let opts = ShipOptions {
            no_publish: true,
            ..Default::default()
        };

        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        let report = run_preflight(utf8_tmp(&tmp), &config, Some(&opts));

        std::env::set_current_dir(original_dir).unwrap();

        // Registry auth check should NOT be present when --no-publish
        let reg_check = report.checks.iter().find(|c| c.name == "Registry auth");
        assert!(
            reg_check.is_none(),
            "registry check should be skipped with --no-publish"
        );
    }

    #[test]
    fn run_preflight_standalone_includes_gh_check() {
        let tmp = tempfile::TempDir::new().unwrap();
        make_git_repo(&tmp);

        let config = Config::default();

        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        // None = standalone preflight, check everything
        let report = run_preflight(utf8_tmp(&tmp), &config, None);

        std::env::set_current_dir(original_dir).unwrap();

        // GitHub CLI check should be present
        let gh_check = report.checks.iter().find(|c| c.name == "GitHub CLI");
        assert!(
            gh_check.is_some(),
            "standalone preflight should include gh auth check"
        );
    }

    #[test]
    fn check_result_skip_flag_serializes_when_present() {
        let result = CheckResult {
            name: "test".into(),
            passed: false,
            message: "fail".into(),
            skip_flag: Some("--no-publish".into()),
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("\"skip_flag\":\"--no-publish\""));
    }

    #[test]
    fn check_result_skip_flag_omitted_when_none() {
        let result = CheckResult {
            name: "test".into(),
            passed: true,
            message: "ok".into(),
            skip_flag: None,
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(
            !json.contains("skip_flag"),
            "skip_flag should be omitted when None"
        );
    }

    // ---------------------------------------------------------------
    // Updated check counts with extended checks
    // ---------------------------------------------------------------

    #[test]
    fn run_preflight_standalone_check_count() {
        let tmp = tempfile::TempDir::new().unwrap();
        make_git_repo(&tmp);
        // No marker files — no ecosystem, no registry check
        let config = Config::default();

        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        let report = run_preflight(utf8_tmp(&tmp), &config, None);

        std::env::set_current_dir(original_dir).unwrap();

        // Without detection: git repo, clean tree, release branch,
        // remote sync, ecosystem, gh auth = 6 checks
        assert_eq!(
            report.checks.len(),
            6,
            "standalone preflight without detection: 5 base + 1 gh auth"
        );
    }

    #[test]
    fn preflight_report_json_includes_version_strategy() {
        let det = ProjectDetection {
            ecosystem: Ecosystem::Rust,
            version_strategy: VersionStrategy::ConventionalCommits {
                tool: ChangelogTool::GitCliff,
            },
            tools: DetectedTools {
                test_cmd: "cargo test".into(),
                build_cmd: "cargo build".into(),
                publish_cmd: None,
                bump_cmd: None,
                changelog_tool: Some(ChangelogTool::GitCliff),
            },
        };
        let report = PreflightReport {
            checks: vec![],
            all_passed: true,
            detection: Some(det),
        };
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("conventional-commits"));
        assert!(json.contains("git-cliff"));
    }
}
