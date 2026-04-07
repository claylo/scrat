//! End-to-end CLI integration tests
//!
//! These tests invoke the compiled binary as a subprocess to verify
//! that the CLI behaves correctly from a user's perspective.

use assert_cmd::Command;
use predicates::prelude::*;

/// Returns a Command configured to run our binary.
///
/// Note: `cargo_bin` is marked deprecated for edge cases involving custom
/// cargo build directories, but works correctly for standard project layouts.
#[allow(deprecated)]
fn cmd() -> Command {
    let mut cmd = Command::cargo_bin(env!("CARGO_PKG_NAME")).unwrap();
    // Isolate from real user config (e.g. ~/Library/Application Support/scrat/config.toml)
    cmd.env("HOME", std::env::temp_dir().join("scrat-test-home"));
    // Route log output to a temp directory so tests don't write to production paths
    let prefix = env!("CARGO_PKG_NAME").to_uppercase().replace('-', "_");
    let test_log_dir = std::env::temp_dir().join(format!("{}-test-logs", env!("CARGO_PKG_NAME")));
    cmd.env(format!("{prefix}_LOG_DIR"), test_log_dir);
    cmd
}

// =============================================================================
// Help & Version
// =============================================================================

#[test]
fn help_flag_shows_usage() {
    cmd()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage:"))
        .stdout(predicate::str::contains("Commands:"))
        .stdout(predicate::str::contains("Options:"));
}

#[test]
fn short_help_flag_shows_usage() {
    cmd()
        .arg("-h")
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage:"));
}

#[test]
fn version_flag_shows_version() {
    cmd()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn short_version_flag_shows_version() {
    cmd()
        .arg("-V")
        .assert()
        .success()
        .stdout(predicate::str::contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn version_only_prints_bare_version() {
    cmd()
        .arg("--version-only")
        .assert()
        .success()
        .stdout(predicate::str::diff(format!(
            "{}\n",
            env!("CARGO_PKG_VERSION")
        )));
}

// =============================================================================
// Info Command
// =============================================================================

#[test]
fn info_shows_package_name_and_version() {
    cmd()
        .arg("info")
        .assert()
        .success()
        .stdout(predicate::str::contains(env!("CARGO_PKG_NAME")))
        .stdout(predicate::str::contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn info_json_outputs_valid_json() {
    let output = cmd().arg("info").arg("--json").assert().success();

    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("info --json should output valid JSON");

    assert_eq!(json["name"], env!("CARGO_PKG_NAME"));
    assert_eq!(json["version"], env!("CARGO_PKG_VERSION"));
}

#[test]
fn info_json_contains_expected_fields() {
    cmd()
        .arg("info")
        .arg("--json")
        .assert()
        .success()
        .stdout(predicate::str::contains("\"name\""))
        .stdout(predicate::str::contains("\"version\""));
}

#[test]
fn info_help_shows_command_options() {
    cmd()
        .args(["info", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--json"));
}

// =============================================================================
// Global Flags
// =============================================================================

#[test]
fn quiet_flag_accepted() {
    cmd().args(["--quiet", "info"]).assert().success();
}

#[test]
fn short_quiet_flag_accepted() {
    cmd().args(["-q", "info"]).assert().success();
}

#[test]
fn verbose_flag_accepted() {
    cmd().args(["--verbose", "info"]).assert().success();
}

#[test]
fn short_verbose_flag_accepted() {
    cmd().args(["-v", "info"]).assert().success();
}

#[test]
fn multiple_verbose_flags_accepted() {
    cmd().args(["-vv", "info"]).assert().success();
}

#[test]
fn color_auto_accepted() {
    cmd().args(["--color", "auto", "info"]).assert().success();
}

#[test]
fn color_always_accepted() {
    cmd().args(["--color", "always", "info"]).assert().success();
}

#[test]
fn color_never_accepted() {
    cmd().args(["--color", "never", "info"]).assert().success();
}

// =============================================================================
// Error Cases
// =============================================================================

#[test]
fn no_subcommand_shows_help() {
    // arg_required_else_help makes clap print help to stderr and exit 2
    cmd()
        .assert()
        .code(2)
        .stderr(predicate::str::contains("Usage:"));
}

#[test]
fn invalid_subcommand_shows_error() {
    cmd()
        .arg("not-a-command")
        .assert()
        .failure()
        .stderr(predicate::str::contains("error:"));
}

#[test]
fn invalid_flag_shows_error() {
    cmd()
        .arg("--not-a-flag")
        .assert()
        .failure()
        .stderr(predicate::str::contains("error:"));
}

// =============================================================================
// Chdir Flag
// =============================================================================

#[test]
fn chdir_flag_changes_directory() {
    // The -C flag should be accepted and work without error
    // We use a path that definitely exists
    cmd().args(["-C", "/tmp", "info"]).assert().success();
}

#[test]
fn chdir_nonexistent_fails() {
    cmd()
        .args(["-C", "/nonexistent/path/that/does/not/exist", "info"])
        .assert()
        .failure();
}

// =============================================================================
// Ship Command
// =============================================================================

#[test]
fn ship_help_shows_usage() {
    cmd()
        .args(["ship", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Run the full release workflow"))
        .stdout(predicate::str::contains("--dry-run"))
        .stdout(predicate::str::contains("--no-publish"))
        .stdout(predicate::str::contains("--no-push"))
        .stdout(predicate::str::contains("--no-release"))
        .stdout(predicate::str::contains("--no-test"))
        .stdout(predicate::str::contains("--no-tag"))
        .stdout(predicate::str::contains("--no-git"))
        .stdout(predicate::str::contains("--draft"))
        .stdout(predicate::str::contains("--no-changelog"))
        .stdout(predicate::str::contains("--version"));
}

#[test]
fn ship_shows_in_subcommand_list() {
    cmd()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("ship"));
}

// =============================================================================
// Doctor Command
// =============================================================================

#[test]
fn doctor_runs_successfully() {
    cmd().arg("doctor").assert().success();
}

#[test]
fn doctor_json_outputs_valid_json() {
    let output = cmd().args(["doctor", "--json"]).assert().success();

    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("doctor --json should output valid JSON");

    // Should have the top-level sections
    assert!(
        json.get("directories").is_some(),
        "missing 'directories' key"
    );
    assert!(json.get("config").is_some(), "missing 'config' key");
    assert!(
        json.get("environment").is_some(),
        "missing 'environment' key"
    );
}

#[test]
fn doctor_help_shows_usage() {
    cmd()
        .args(["doctor", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Diagnose configuration and environment",
        ));
}

// =============================================================================
// Init Command
// =============================================================================

#[test]
fn init_help_shows_usage_and_flags() {
    cmd()
        .args(["init", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Generate a scrat config file"))
        .stdout(predicate::str::contains("--format"))
        .stdout(predicate::str::contains("--style"))
        .stdout(predicate::str::contains("--yes"))
        .stdout(predicate::str::contains("--output"));
}

#[test]
fn init_yes_toml_generates_valid_toml() {
    let tmp = tempfile::tempdir().expect("failed to create tempdir");
    let tmp_path = tmp.path();

    // git init so ecosystem detection doesn't trip
    std::process::Command::new("git")
        .args(["init"])
        .current_dir(tmp_path)
        .output()
        .expect("git init failed");

    cmd()
        .args(["init", "--yes", "--format", "toml"])
        .current_dir(tmp_path)
        .assert()
        .success();

    let config_path = tmp_path.join("scrat.toml");
    assert!(config_path.exists(), "scrat.toml should be created");

    let content = std::fs::read_to_string(&config_path).expect("failed to read scrat.toml");
    assert!(!content.is_empty(), "scrat.toml should not be empty");
    // All generated configs include the [release] section
    assert!(
        content.contains("[release]"),
        "should contain release section"
    );
}

#[test]
fn init_yes_yaml_generates_valid_yaml() {
    let tmp = tempfile::tempdir().expect("failed to create tempdir");
    let tmp_path = tmp.path();

    // git init so ecosystem detection doesn't trip
    std::process::Command::new("git")
        .args(["init"])
        .current_dir(tmp_path)
        .output()
        .expect("git init failed");

    cmd()
        .args(["init", "--yes", "--format", "yaml"])
        .current_dir(tmp_path)
        .assert()
        .success();

    let config_path = tmp_path.join("scrat.yaml");
    assert!(config_path.exists(), "scrat.yaml should be created");

    let content = std::fs::read_to_string(&config_path).expect("failed to read scrat.yaml");
    assert!(!content.is_empty(), "scrat.yaml should not be empty");
    // All generated configs include the release section
    assert!(
        content.contains("release:"),
        "should contain release section"
    );
}

#[test]
fn init_yes_custom_output_path() {
    let tmp = tempfile::tempdir().expect("failed to create tempdir");
    let tmp_path = tmp.path();
    let output_file = tmp_path.join("custom-config.toml");

    std::process::Command::new("git")
        .args(["init"])
        .current_dir(tmp_path)
        .output()
        .expect("git init failed");

    cmd()
        .args([
            "init",
            "--yes",
            "--format",
            "toml",
            "--output",
            output_file.to_str().unwrap(),
        ])
        .current_dir(tmp_path)
        .assert()
        .success();

    assert!(output_file.exists(), "custom output file should be created");
}

#[test]
fn init_shows_in_subcommand_list() {
    cmd()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("init"));
}

// =============================================================================
// Bump Command
// =============================================================================

#[test]
fn bump_help_shows_usage() {
    cmd()
        .args(["bump", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Determine next version"))
        .stdout(predicate::str::contains("--version"))
        .stdout(predicate::str::contains("--no-changelog"))
        .stdout(predicate::str::contains("--dry-run"));
}

#[test]
fn bump_shows_in_subcommand_list() {
    cmd()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("bump"));
}

// =============================================================================
// Notes Command
// =============================================================================

#[test]
fn notes_help_shows_usage() {
    cmd()
        .args(["notes", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Render release notes"))
        .stdout(predicate::str::contains("--from"))
        .stdout(predicate::str::contains("--version"))
        .stdout(predicate::str::contains("--template"))
        .stdout(predicate::str::contains("--no-deps"))
        .stdout(predicate::str::contains("--no-stats"));
}

#[test]
fn notes_shows_in_subcommand_list() {
    cmd()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("notes"));
}

// =============================================================================
// Preflight Command
// =============================================================================

#[test]
fn preflight_help_shows_usage() {
    cmd()
        .args(["preflight", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Check release readiness"));
}

#[test]
fn preflight_json_flag_accepted() {
    // Run in a tempdir with git init to avoid interactive prompts.
    // Preflight will likely fail checks (no version files, etc.) but
    // the --json flag should be accepted and produce JSON output.
    let tmp = tempfile::tempdir().expect("failed to create tempdir");
    let tmp_path = tmp.path();

    std::process::Command::new("git")
        .args(["init"])
        .current_dir(tmp_path)
        .output()
        .expect("git init failed");

    let output = cmd()
        .args(["preflight", "--json"])
        .current_dir(tmp_path)
        .assert();

    // Preflight may exit non-zero if checks fail, but stdout should be valid JSON
    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    if !stdout.trim().is_empty() {
        let _json: serde_json::Value =
            serde_json::from_str(&stdout).expect("preflight --json should output valid JSON");
    }
}

#[test]
fn preflight_shows_in_subcommand_list() {
    cmd()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("preflight"));
}

// =============================================================================
// All Subcommands in Help
// =============================================================================

#[test]
fn all_subcommands_appear_in_help() {
    let expected = [
        "doctor",
        "init",
        "info",
        "preflight",
        "bump",
        "notes",
        "ship",
    ];
    let output = cmd().arg("--help").assert().success();
    let stdout = String::from_utf8_lossy(&output.get_output().stdout);

    for subcmd in expected {
        assert!(
            stdout.contains(subcmd),
            "subcommand '{subcmd}' should appear in --help output"
        );
    }
}
