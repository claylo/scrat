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
    Command::cargo_bin(env!("CARGO_PKG_NAME")).unwrap()
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
    let output = cmd()
        .arg("info")
        .arg("--json")
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&output.get_output().stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout)
        .expect("info --json should output valid JSON");

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
    cmd()
        .args(["--quiet", "info"])
        .assert()
        .success();
}

#[test]
fn short_quiet_flag_accepted() {
    cmd()
        .args(["-q", "info"])
        .assert()
        .success();
}

#[test]
fn verbose_flag_accepted() {
    cmd()
        .args(["--verbose", "info"])
        .assert()
        .success();
}

#[test]
fn short_verbose_flag_accepted() {
    cmd()
        .args(["-v", "info"])
        .assert()
        .success();
}

#[test]
fn multiple_verbose_flags_accepted() {
    cmd()
        .args(["-vv", "info"])
        .assert()
        .success();
}

#[test]
fn color_auto_accepted() {
    cmd()
        .args(["--color", "auto", "info"])
        .assert()
        .success();
}

#[test]
fn color_always_accepted() {
    cmd()
        .args(["--color", "always", "info"])
        .assert()
        .success();
}

#[test]
fn color_never_accepted() {
    cmd()
        .args(["--color", "never", "info"])
        .assert()
        .success();
}

// =============================================================================
// Error Cases
// =============================================================================

#[test]
fn no_subcommand_shows_help() {
    cmd()
        .assert()
        .failure()
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
    cmd()
        .args(["-C", "/tmp", "info"])
        .assert()
        .success();
}

#[test]
fn chdir_nonexistent_fails() {
    cmd()
        .args(["-C", "/nonexistent/path/that/does/not/exist", "info"])
        .assert()
        .failure();
}
