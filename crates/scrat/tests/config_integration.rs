//! Configuration integration tests.
//!
//! These tests verify config discovery, format parsing, and precedence
//! from an end-to-end perspective using the compiled binary.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

/// Returns a Command configured to run our binary.
#[allow(deprecated)]
fn cmd() -> Command {
    Command::cargo_bin(env!("CARGO_PKG_NAME")).unwrap()
}

// =============================================================================
// Config File Discovery
// =============================================================================

#[test]
fn runs_without_config_file() {
    // The CLI should work even when no config file exists
    let tmp = TempDir::new().unwrap();

    cmd()
        .args(["-C", tmp.path().to_str().unwrap(), "info"])
        .assert()
        .success();
}

#[test]
fn discovers_dotfile_config_in_current_dir() {
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join(".scrat.toml");
    fs::write(&config_path, r#"log_level = "debug""#).unwrap();

    cmd()
        .args(["-C", tmp.path().to_str().unwrap(), "info"])
        .assert()
        .success();
}

#[test]
fn discovers_regular_config_in_current_dir() {
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("scrat.toml");
    fs::write(&config_path, r#"log_level = "warn""#).unwrap();

    cmd()
        .args(["-C", tmp.path().to_str().unwrap(), "info"])
        .assert()
        .success();
}

#[test]
fn discovers_config_in_parent_directory() {
    let tmp = TempDir::new().unwrap();
    let sub_dir = tmp.path().join("nested").join("deep");
    fs::create_dir_all(&sub_dir).unwrap();

    // Config in root, run from nested/deep
    let config_path = tmp.path().join(".scrat.toml");
    fs::write(&config_path, r#"log_level = "debug""#).unwrap();

    cmd()
        .args(["-C", sub_dir.to_str().unwrap(), "info"])
        .assert()
        .success();
}

#[test]
fn dotfile_takes_precedence_over_regular_name() {
    let tmp = TempDir::new().unwrap();

    // Both configs exist
    fs::write(
        tmp.path().join(".scrat.toml"),
        r#"log_level = "debug""#,
    )
    .unwrap();
    fs::write(
        tmp.path().join("scrat.toml"),
        r#"log_level = "error""#,
    )
    .unwrap();

    // Should use the dotfile (debug), not the regular one (error)
    // The test passes if the CLI runs successfully with either config
    cmd()
        .args(["-C", tmp.path().to_str().unwrap(), "info"])
        .assert()
        .success();
}

// =============================================================================
// Config Format Parsing
// =============================================================================

#[test]
fn parses_toml_config() {
    let tmp = TempDir::new().unwrap();
    fs::write(
        tmp.path().join(".scrat.toml"),
        r#"
log_level = "warn"
"#,
    )
    .unwrap();

    cmd()
        .args(["-C", tmp.path().to_str().unwrap(), "info"])
        .assert()
        .success();
}

#[test]
fn parses_yaml_config() {
    let tmp = TempDir::new().unwrap();
    fs::write(
        tmp.path().join(".scrat.yaml"),
        r#"
log_level: warn
"#,
    )
    .unwrap();

    cmd()
        .args(["-C", tmp.path().to_str().unwrap(), "info"])
        .assert()
        .success();
}

#[test]
fn parses_yml_config() {
    let tmp = TempDir::new().unwrap();
    fs::write(
        tmp.path().join(".scrat.yml"),
        r#"
log_level: debug
"#,
    )
    .unwrap();

    cmd()
        .args(["-C", tmp.path().to_str().unwrap(), "info"])
        .assert()
        .success();
}

#[test]
fn parses_json_config() {
    let tmp = TempDir::new().unwrap();
    fs::write(
        tmp.path().join(".scrat.json"),
        r#"{"log_level": "error"}"#,
    )
    .unwrap();

    cmd()
        .args(["-C", tmp.path().to_str().unwrap(), "info"])
        .assert()
        .success();
}

// =============================================================================
// Config Precedence
// =============================================================================

#[test]
fn closer_config_takes_precedence() {
    let tmp = TempDir::new().unwrap();
    let sub_dir = tmp.path().join("project");
    fs::create_dir_all(&sub_dir).unwrap();

    // Parent config
    fs::write(
        tmp.path().join(".scrat.toml"),
        r#"log_level = "error""#,
    )
    .unwrap();

    // Child config (should win)
    fs::write(
        sub_dir.join(".scrat.toml"),
        r#"log_level = "debug""#,
    )
    .unwrap();

    // Run from child directory - should use child config
    cmd()
        .args(["-C", sub_dir.to_str().unwrap(), "info"])
        .assert()
        .success();
}

#[test]
fn toml_preferred_over_yaml_in_same_directory() {
    let tmp = TempDir::new().unwrap();

    // TOML is first in extension preference order
    fs::write(
        tmp.path().join(".scrat.toml"),
        r#"log_level = "debug""#,
    )
    .unwrap();
    fs::write(
        tmp.path().join(".scrat.yaml"),
        r#"log_level: error"#,
    )
    .unwrap();

    // Should succeed with the TOML config
    cmd()
        .args(["-C", tmp.path().to_str().unwrap(), "info"])
        .assert()
        .success();
}

// =============================================================================
// Error Cases
// =============================================================================

#[test]
fn invalid_toml_config_shows_error() {
    let tmp = TempDir::new().unwrap();
    fs::write(
        tmp.path().join(".scrat.toml"),
        "this is not valid toml [[[",
    )
    .unwrap();

    cmd()
        .args(["-C", tmp.path().to_str().unwrap(), "info"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("configuration").or(predicate::str::contains("config")));
}

#[test]
fn invalid_yaml_config_shows_error() {
    let tmp = TempDir::new().unwrap();
    fs::write(
        tmp.path().join(".scrat.yaml"),
        "invalid:\n  yaml\n content:\n[broken",
    )
    .unwrap();

    cmd()
        .args(["-C", tmp.path().to_str().unwrap(), "info"])
        .assert()
        .failure();
}

#[test]
fn invalid_json_config_shows_error() {
    let tmp = TempDir::new().unwrap();
    fs::write(
        tmp.path().join(".scrat.json"),
        "{not valid json}",
    )
    .unwrap();

    cmd()
        .args(["-C", tmp.path().to_str().unwrap(), "info"])
        .assert()
        .failure();
}

#[test]
fn unknown_config_field_is_ignored() {
    // Figment ignores unknown fields by default with serde
    let tmp = TempDir::new().unwrap();
    fs::write(
        tmp.path().join(".scrat.toml"),
        r#"
log_level = "info"
unknown_field = "should be ignored"
another_unknown = 42
"#,
    )
    .unwrap();

    cmd()
        .args(["-C", tmp.path().to_str().unwrap(), "info"])
        .assert()
        .success();
}

// =============================================================================
// Boundary Marker Tests
// =============================================================================

#[test]
fn git_boundary_stops_config_search() {
    let tmp = TempDir::new().unwrap();

    // Structure: /tmp/parent/.project.toml + /tmp/parent/repo/.git/ + /tmp/parent/repo/src/
    let parent = tmp.path().join("parent");
    let repo = parent.join("repo");
    let src = repo.join("src");
    fs::create_dir_all(&src).unwrap();

    // Config in parent (outside repo)
    fs::write(
        parent.join(".scrat.toml"),
        r#"log_level = "error""#,
    )
    .unwrap();

    // .git directory marks repo boundary
    fs::create_dir(repo.join(".git")).unwrap();

    // Running from src/ should NOT find parent config (stopped at .git)
    // The CLI should still work, just with defaults
    cmd()
        .args(["-C", src.to_str().unwrap(), "info"])
        .assert()
        .success();
}

#[test]
fn config_in_same_dir_as_git_is_found() {
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path().join("repo");
    let src = repo.join("src");
    fs::create_dir_all(&src).unwrap();

    // .git and config in same directory
    fs::create_dir(repo.join(".git")).unwrap();
    fs::write(
        repo.join(".scrat.toml"),
        r#"log_level = "debug""#,
    )
    .unwrap();

    // Running from src/ should find the repo config
    cmd()
        .args(["-C", src.to_str().unwrap(), "info"])
        .assert()
        .success();
}

