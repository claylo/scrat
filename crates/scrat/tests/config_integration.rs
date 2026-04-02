//! Configuration integration tests.
//!
//! These tests verify config discovery, format parsing, and precedence
//! from an end-to-end perspective using the compiled binary. Tests use
//! `info --json` to assert actual config values, not just process success.

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
use std::fs;
use tempfile::TempDir;

/// Returns a Command configured to run our binary.
#[allow(deprecated)]
fn cmd() -> Command {
    Command::cargo_bin(env!("CARGO_PKG_NAME")).unwrap()
}

/// Run `info --json` from a directory and parse the JSON output.
fn info_json(dir: &std::path::Path) -> Value {
    let output = cmd()
        .args(["-C", dir.to_str().unwrap(), "info", "--json"])
        .output()
        .expect("failed to run command");
    assert!(
        output.status.success(),
        "command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("invalid JSON output")
}

// =============================================================================
// Config File Discovery
// =============================================================================

#[test]
fn runs_without_config_file() {
    let tmp = TempDir::new().unwrap();
    let json = info_json(tmp.path());

    assert_eq!(
        json["config"]["log_level"], "info",
        "should use default log level"
    );
    assert!(
        json["config"]["config_file"].is_null(),
        "no config file should be reported"
    );
}

#[test]
fn discovers_dotfile_config_in_current_dir() {
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join(".scrat.toml");
    fs::write(&config_path, r#"log_level = "debug""#).unwrap();

    let json = info_json(tmp.path());

    assert_eq!(json["config"]["log_level"], "debug");
    let reported = json["config"]["config_file"].as_str().unwrap();
    assert!(
        reported.ends_with(".scrat.toml"),
        "should report dotfile: {reported}"
    );
}

#[test]
fn discovers_regular_config_in_current_dir() {
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("scrat.toml");
    fs::write(&config_path, r#"log_level = "warn""#).unwrap();

    let json = info_json(tmp.path());

    assert_eq!(json["config"]["log_level"], "warn");
    let reported = json["config"]["config_file"].as_str().unwrap();
    assert!(
        reported.ends_with("scrat.toml"),
        "should report regular config: {reported}"
    );
}

#[test]
fn discovers_config_in_parent_directory() {
    let tmp = TempDir::new().unwrap();
    let sub_dir = tmp.path().join("nested").join("deep");
    fs::create_dir_all(&sub_dir).unwrap();

    // Config in root, run from nested/deep
    fs::write(tmp.path().join(".scrat.toml"), r#"log_level = "debug""#).unwrap();

    let json = info_json(&sub_dir);

    assert_eq!(json["config"]["log_level"], "debug");
    assert!(
        json["config"]["config_file"].as_str().is_some(),
        "should find parent config"
    );
}

#[test]
fn discovers_dotconfig_directory_config() {
    let tmp = TempDir::new().unwrap();
    let dotconfig = tmp.path().join(".config");
    fs::create_dir_all(&dotconfig).unwrap();
    fs::write(dotconfig.join("scrat.toml"), r#"log_level = "debug""#).unwrap();

    let json = info_json(tmp.path());

    assert_eq!(json["config"]["log_level"], "debug");
    let reported = json["config"]["config_file"].as_str().unwrap();
    assert!(
        reported.contains(".config/"),
        "should report .config/ path: {reported}"
    );
}

#[test]
fn dotconfig_takes_precedence_over_dotfile() {
    let tmp = TempDir::new().unwrap();
    let dotconfig = tmp.path().join(".config");
    fs::create_dir_all(&dotconfig).unwrap();

    // .config/ gets debug, dotfile gets error — .config/ should win
    fs::write(dotconfig.join("scrat.toml"), r#"log_level = "debug""#).unwrap();
    fs::write(tmp.path().join(".scrat.toml"), r#"log_level = "error""#).unwrap();

    let json = info_json(tmp.path());

    assert_eq!(
        json["config"]["log_level"], "debug",
        ".config/ should win over dotfile"
    );
}

#[test]
fn dotfile_takes_precedence_over_regular_name() {
    let tmp = TempDir::new().unwrap();

    // Both configs exist — dotfile should win
    fs::write(tmp.path().join(".scrat.toml"), r#"log_level = "debug""#).unwrap();
    fs::write(tmp.path().join("scrat.toml"), r#"log_level = "error""#).unwrap();

    let json = info_json(tmp.path());

    assert_eq!(
        json["config"]["log_level"], "debug",
        "dotfile value should win"
    );
}

// =============================================================================
// Config Format Parsing
// =============================================================================

#[test]
fn parses_toml_config() {
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join(".scrat.toml"), r#"log_level = "warn""#).unwrap();

    let json = info_json(tmp.path());
    assert_eq!(json["config"]["log_level"], "warn");
}

#[test]
fn parses_yaml_config() {
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join(".scrat.yaml"), "log_level: warn\n").unwrap();

    let json = info_json(tmp.path());
    assert_eq!(json["config"]["log_level"], "warn");
}

#[test]
fn parses_yml_config() {
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join(".scrat.yml"), "log_level: debug\n").unwrap();

    let json = info_json(tmp.path());
    assert_eq!(json["config"]["log_level"], "debug");
}

#[test]
fn parses_json_config() {
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join(".scrat.json"), r#"{"log_level": "error"}"#).unwrap();

    let json = info_json(tmp.path());
    assert_eq!(json["config"]["log_level"], "error");
}

// =============================================================================
// Config Precedence
// =============================================================================

#[test]
fn closer_config_takes_precedence() {
    let tmp = TempDir::new().unwrap();
    let sub_dir = tmp.path().join("project");
    fs::create_dir_all(&sub_dir).unwrap();

    // Parent config (error) vs child config (debug) — child should win
    fs::write(tmp.path().join(".scrat.toml"), r#"log_level = "error""#).unwrap();
    fs::write(sub_dir.join(".scrat.toml"), r#"log_level = "debug""#).unwrap();

    let json = info_json(&sub_dir);

    assert_eq!(
        json["config"]["log_level"], "debug",
        "closer config should win"
    );
}

#[test]
fn toml_preferred_over_yaml_in_same_directory() {
    let tmp = TempDir::new().unwrap();

    // TOML is first in extension preference order
    fs::write(tmp.path().join(".scrat.toml"), r#"log_level = "debug""#).unwrap();
    fs::write(tmp.path().join(".scrat.yaml"), "log_level: error\n").unwrap();

    let json = info_json(tmp.path());
    assert_eq!(
        json["config"]["log_level"], "debug",
        "TOML should be preferred over YAML"
    );
}

#[test]
fn explicit_config_overrides_discovered() {
    let tmp = TempDir::new().unwrap();

    // Project config sets debug
    fs::write(tmp.path().join(".scrat.toml"), r#"log_level = "debug""#).unwrap();

    // Explicit config sets error
    let explicit = tmp.path().join("override.toml");
    fs::write(&explicit, r#"log_level = "error""#).unwrap();

    let output = cmd()
        .args([
            "-C",
            tmp.path().to_str().unwrap(),
            "--config",
            explicit.to_str().unwrap(),
            "info",
            "--json",
        ])
        .output()
        .expect("failed to run command");
    assert!(output.status.success());

    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        json["config"]["log_level"], "error",
        "--config should override discovered config"
    );
    let reported = json["config"]["config_file"].as_str().unwrap();
    assert!(
        reported.ends_with("override.toml"),
        "--config path should be reported: {reported}"
    );
}

// =============================================================================
// Error Cases
// =============================================================================

#[test]
fn invalid_toml_config_shows_error() {
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join(".scrat.toml"), "this is not valid toml [[[").unwrap();

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
    fs::write(tmp.path().join(".scrat.json"), "{not valid json}").unwrap();

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
        "log_level = \"info\"\nunknown_field = \"should be ignored\"\nanother_unknown = 42\n",
    )
    .unwrap();

    let json = info_json(tmp.path());
    assert_eq!(json["config"]["log_level"], "info");
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
    fs::write(parent.join(".scrat.toml"), r#"log_level = "error""#).unwrap();

    // .git directory marks repo boundary
    fs::create_dir(repo.join(".git")).unwrap();

    // Running from src/ should NOT find parent config (stopped at .git)
    let json = info_json(&src);

    assert_eq!(
        json["config"]["log_level"], "info",
        "should use default — boundary stops search"
    );
    assert!(
        json["config"]["config_file"].is_null(),
        "should not find config beyond boundary"
    );
}

#[test]
fn config_in_same_dir_as_git_is_found() {
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path().join("repo");
    let src = repo.join("src");
    fs::create_dir_all(&src).unwrap();

    // .git and config in same directory
    fs::create_dir(repo.join(".git")).unwrap();
    fs::write(repo.join(".scrat.toml"), r#"log_level = "debug""#).unwrap();

    // Running from src/ should find the repo config
    let json = info_json(&src);

    assert_eq!(
        json["config"]["log_level"], "debug",
        "config next to .git should be found"
    );
    assert!(
        json["config"]["config_file"].as_str().is_some(),
        "should report config file"
    );
}
