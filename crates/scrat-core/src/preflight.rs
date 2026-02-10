//! Preflight checks for release readiness.
//!
//! Validates the git state, branch, remote sync, and tool availability
//! before a release. Returns structured results that the CLI formats.

use serde::Serialize;
use tracing::{debug, instrument};

use crate::config::Config;
use crate::detect;
use crate::ecosystem::ProjectDetection;
use crate::git;

/// A single preflight check result.
#[derive(Debug, Clone, Serialize)]
pub struct CheckResult {
    /// Human-readable name of the check.
    pub name: String,
    /// Whether the check passed.
    pub passed: bool,
    /// Description of the result (reason for failure, or confirmation).
    pub message: String,
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
#[instrument(skip(config), fields(root = %project_root))]
pub fn run_preflight(project_root: &camino::Utf8Path, config: &Config) -> PreflightReport {
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

    // Check 4: Remote in sync
    checks.push(check_remote_sync());

    // Check 5: Ecosystem detection
    let detection = detect::detect_project(project_root);
    checks.push(check_ecosystem(&detection));

    // Check 6: Required tools
    if let Some(ref det) = detection {
        checks.push(check_required_tools(det));
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
        },
        Ok(false) => CheckResult {
            name: "Git repository".into(),
            passed: false,
            message: "Not inside a git repository".into(),
        },
        Err(e) => CheckResult {
            name: "Git repository".into(),
            passed: false,
            message: format!("Failed to check: {e}"),
        },
    }
}

fn check_clean_tree() -> CheckResult {
    match git::is_clean() {
        Ok(true) => CheckResult {
            name: "Working tree".into(),
            passed: true,
            message: "Clean working tree".into(),
        },
        Ok(false) => CheckResult {
            name: "Working tree".into(),
            passed: false,
            message: "Uncommitted changes in working tree".into(),
        },
        Err(e) => CheckResult {
            name: "Working tree".into(),
            passed: false,
            message: format!("Failed to check: {e}"),
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
            };
        }
        Err(e) => {
            return CheckResult {
                name: "Release branch".into(),
                passed: false,
                message: format!("Failed to check: {e}"),
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
            }
        }
        Ok(None) => CheckResult {
            name: "Release branch".into(),
            passed: false,
            message: format!("On '{current}' — no main/master branch found"),
        },
        Err(e) => CheckResult {
            name: "Release branch".into(),
            passed: false,
            message: format!("Failed to detect: {e}"),
        },
    }
}

fn check_remote_sync() -> CheckResult {
    match git::is_remote_in_sync() {
        Ok(true) => CheckResult {
            name: "Remote sync".into(),
            passed: true,
            message: "Local branch is in sync with remote".into(),
        },
        Ok(false) => CheckResult {
            name: "Remote sync".into(),
            passed: false,
            message: "Local branch is out of sync with remote (pull or push needed)".into(),
        },
        Err(e) => CheckResult {
            name: "Remote sync".into(),
            passed: false,
            message: format!("Failed to check: {e}"),
        },
    }
}

fn check_ecosystem(detection: &Option<ProjectDetection>) -> CheckResult {
    detection.as_ref().map_or_else(
        || CheckResult {
            name: "Project detection".into(),
            passed: false,
            message: "No recognized project type (missing Cargo.toml, package.json, etc.)".into(),
        },
        |det| CheckResult {
            name: "Project detection".into(),
            passed: true,
            message: format!("Detected {} project", det.ecosystem),
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preflight_report_serializes() {
        let report = PreflightReport {
            checks: vec![CheckResult {
                name: "test".into(),
                passed: true,
                message: "ok".into(),
            }],
            all_passed: true,
            detection: None,
        };
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("\"all_passed\":true"));
    }

    #[test]
    fn check_ecosystem_none() {
        let result = check_ecosystem(&None);
        assert!(!result.passed);
    }
}
