//! Configuration loading and discovery.
//!
//! This module provides configuration file discovery by:
//! 1. Walking up from the current directory to find project config
//! 2. Loading user config from XDG config directory
//! 3. Merging with sensible defaults
//!
//! # Supported formats
//!
//! The following configuration file formats are supported:
//! - TOML (`.toml`)
//! - YAML (`.yaml`, `.yml`)
//! - JSON (`.json`)
//!
//! # Config file locations (in order of precedence, highest first):
//! - `.scrat.<ext>` in current directory or any parent
//! - `scrat.<ext>` in current directory or any parent
//! - `~/.config/scrat/config.<ext>` (user config)
//!
//! Where `<ext>` is one of: `toml`, `yaml`, `yml`, `json`
//!
//! # Example
//! ```no_run
//! use camino::Utf8PathBuf;
//! use scrat_core::config::{Config, ConfigLoader};
//!
//! let cwd = std::env::current_dir().unwrap();
//! let cwd = Utf8PathBuf::try_from(cwd).expect("current directory is not valid UTF-8");
//! let config = ConfigLoader::new()
//!     .with_project_search(&cwd)
//!     .load()
//!     .unwrap();
//! ```

use camino::{Utf8Path, Utf8PathBuf};
use figment::Figment;
use figment::providers::{Format, Json, Serialized, Toml, Yaml};
use serde::{Deserialize, Serialize};

use crate::ecosystem::{ChangelogTool, Ecosystem};
use crate::error::{ConfigError, ConfigResult};

/// The configuration for scrat.
///
/// Deserialized from config files found during discovery (TOML, YAML, or JSON).
/// All section fields are optional — auto-detection fills in smart defaults,
/// and config values act as overrides.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct Config {
    /// Log level for the application (e.g., "debug", "info", "warn", "error").
    pub log_level: LogLevel,
    /// Directory for JSONL log files (falls back to platform defaults if unset).
    pub log_dir: Option<Utf8PathBuf>,
    /// Project configuration overrides.
    pub project: Option<ProjectConfig>,
    /// Version strategy overrides.
    pub version: Option<VersionConfig>,
    /// Command overrides per workflow phase.
    pub commands: Option<CommandsConfig>,
    /// Release workflow configuration.
    pub release: Option<ReleaseConfig>,
    /// Hook commands per release phase.
    pub hooks: Option<HooksConfig>,
    /// Ship command behavior.
    pub ship: Option<ShipConfig>,
}

/// Project-level configuration overrides.
///
/// Normally auto-detected from marker files in the working directory.
/// Use this section to override the detected values.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct ProjectConfig {
    /// Override the detected ecosystem (e.g., `"rust"`, `"node"`).
    #[serde(rename = "type")]
    pub project_type: Option<Ecosystem>,
    /// Override the release branch (default: auto-detect `main` or `master`).
    pub release_branch: Option<String>,
}

/// Version strategy configuration.
///
/// Normally auto-detected from the presence of `cliff.toml` / `cog.toml`.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct VersionConfig {
    /// Override the version strategy.
    ///
    /// Possible values: `"conventional-commits"`, `"interactive"`, `"explicit"`.
    pub strategy: Option<String>,
}

/// Command overrides for each phase of the release workflow.
///
/// Each ecosystem provides smart defaults. Use this section to override
/// any phase's command.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct CommandsConfig {
    /// Override the test command (e.g., `"cargo nextest run"`).
    pub test: Option<String>,
    /// Override the build command (e.g., `"cargo build --release"`).
    pub build: Option<String>,
    /// Override the publish command (e.g., `"cargo publish"`).
    pub publish: Option<String>,
    /// Override the clean command.
    pub clean: Option<String>,
}

/// Release workflow configuration.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct ReleaseConfig {
    /// Override the changelog tool (`"git-cliff"` or `"cog"`).
    pub changelog_tool: Option<ChangelogTool>,
    /// Whether to create a GitHub release (default: `true`).
    pub github_release: Option<bool>,
    /// File paths to attach to the GitHub release as assets.
    ///
    /// Hook commands produce these files; scrat attaches them.
    /// Paths are relative to the project root.
    pub assets: Option<Vec<String>>,
    /// Path to a custom git-cliff template for release notes.
    ///
    /// If unset, uses the built-in template. The template is rendered by
    /// git-cliff (Tera syntax) with scrat's extra data injected into context.
    pub notes_template: Option<String>,
    /// Create the GitHub release as a draft (default at usage site: `true`).
    ///
    /// When `true`, `gh release create --draft` is used. Review and publish
    /// with `gh release edit <tag> --draft=false`.
    pub draft: Option<bool>,
    /// Title format for the GitHub release.
    ///
    /// Supports `{var}` interpolation: `{version}`, `{prev_version}`,
    /// `{tag}`, `{owner}`, `{repo}`, `{changelog_path}`.
    /// Default (when `None`): uses the tag as title (e.g., `v1.2.3`).
    pub title: Option<String>,
    /// GitHub Discussions category to associate with the release.
    ///
    /// When set, passes `--discussion-category <value>` to `gh release create`.
    /// Only applies to newly created releases (not edits).
    pub discussion_category: Option<String>,
}

/// Hook commands to run at each phase of the release workflow.
///
/// Each hook is a list of shell commands executed in order. Commands
/// support variable interpolation:
/// - `{version}` — the new version (e.g., `1.2.3`)
/// - `{prev_version}` — the previous version
/// - `{tag}` — the git tag (e.g., `v1.2.3`)
/// - `{changelog_path}` — path to the generated CHANGELOG
/// - `{owner}` — the repository owner (from git remote)
/// - `{repo}` — the repository name (from git remote)
///
/// Commands run in parallel by default. Prefix a command with `sync:`
/// to create a barrier — all prior commands must finish, the sync
/// command runs alone, then subsequent commands resume in parallel.
///
/// # Example
///
/// ```toml
/// [hooks]
/// post_bump = [
///     "ll-graphics generate --version {version} --output release-card.png",
/// ]
/// ```
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct HooksConfig {
    /// Commands to run before the entire ship workflow.
    pub pre_ship: Option<Vec<String>>,
    /// Commands to run after the entire ship workflow completes.
    pub post_ship: Option<Vec<String>>,
    /// Commands to run before the test phase.
    pub pre_test: Option<Vec<String>>,
    /// Commands to run after tests pass.
    pub post_test: Option<Vec<String>>,
    /// Commands to run before bumping the version.
    pub pre_bump: Option<Vec<String>>,
    /// Commands to run after bumping the version and generating the changelog.
    pub post_bump: Option<Vec<String>>,
    /// Commands to run before publishing to a registry.
    pub pre_publish: Option<Vec<String>>,
    /// Commands to run after publishing.
    pub post_publish: Option<Vec<String>>,
    /// Commands to run before creating the git tag.
    pub pre_tag: Option<Vec<String>>,
    /// Commands to run after pushing tags.
    pub post_tag: Option<Vec<String>>,
    /// Commands to run before creating a GitHub release.
    pub pre_release: Option<Vec<String>>,
    /// Commands to run after the GitHub release is created.
    pub post_release: Option<Vec<String>>,
}

/// Ship command behavior.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct ShipConfig {
    /// Prompt for confirmation before executing (default: true).
    ///
    /// When `None` or `Some(true)`, `scrat ship` shows the plan and asks
    /// for confirmation before executing. Set to `false` for CI/scripted use.
    /// The `--yes`/`-y` CLI flag overrides this at runtime.
    pub confirm: Option<bool>,
}

/// Log level configuration.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    /// Verbose output for debugging and development.
    Debug,
    /// Standard operational information (default).
    #[default]
    Info,
    /// Warnings about potential issues.
    Warn,
    /// Errors that indicate failures.
    Error,
}

impl LogLevel {
    /// Returns the log level as a lowercase string slice.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }
}

/// Supported configuration file extensions (in order of preference).
const CONFIG_EXTENSIONS: &[&str] = &["toml", "yaml", "yml", "json"];

/// Application name for XDG directory lookup and config file names.
const APP_NAME: &str = "scrat";

/// Builder for loading configuration from multiple sources.
#[derive(Debug, Default)]
pub struct ConfigLoader {
    /// Starting directory for project config search.
    project_search_root: Option<Utf8PathBuf>,
    /// Whether to include user config from XDG directory.
    include_user_config: bool,
    /// Stop searching when we hit a directory containing this file/dir.
    boundary_marker: Option<String>,
    /// Explicit config files to load (for testing or programmatic use).
    explicit_files: Vec<Utf8PathBuf>,
}

impl ConfigLoader {
    /// Create a new config loader with default settings.
    pub fn new() -> Self {
        Self {
            project_search_root: None,
            include_user_config: true,
            boundary_marker: Some(".git".to_string()),
            explicit_files: Vec::new(),
        }
    }

    /// Set the starting directory for project config search.
    ///
    /// The loader will walk up from this directory looking for config files.
    pub fn with_project_search<P: AsRef<Utf8Path>>(mut self, path: P) -> Self {
        self.project_search_root = Some(path.as_ref().to_path_buf());
        self
    }

    /// Set whether to include user config from `~/.config/scrat/`.
    pub const fn with_user_config(mut self, include: bool) -> Self {
        self.include_user_config = include;
        self
    }

    /// Set a boundary marker to stop directory traversal.
    ///
    /// When walking up directories, stop if we find a directory containing
    /// this file or directory name. Default is `.git`.
    pub fn with_boundary_marker<S: Into<String>>(mut self, marker: S) -> Self {
        self.boundary_marker = Some(marker.into());
        self
    }

    /// Disable boundary marker (search all the way to filesystem root).
    pub fn without_boundary_marker(mut self) -> Self {
        self.boundary_marker = None;
        self
    }

    /// Add an explicit config file to load.
    ///
    /// Files are loaded in order, with later files taking precedence.
    /// Explicit files are loaded after discovered files.
    pub fn with_file<P: AsRef<Utf8Path>>(mut self, path: P) -> Self {
        self.explicit_files.push(path.as_ref().to_path_buf());
        self
    }

    /// Load configuration, merging all discovered sources.
    ///
    /// Precedence (highest to lowest):
    /// 1. Explicit files (in order added via `with_file`)
    /// 2. Project config (closest to search root)
    /// 3. User config (`~/.config/scrat/config.<ext>`)
    /// 4. Default values
    #[tracing::instrument(skip(self), fields(search_root = ?self.project_search_root))]
    pub fn load(self) -> ConfigResult<Config> {
        tracing::debug!("loading configuration");
        let mut figment = Figment::new().merge(Serialized::defaults(Config::default()));

        // Start with user config (lowest precedence of file sources)
        if self.include_user_config
            && let Some(user_config) = self.find_user_config()
        {
            figment = Self::merge_file(figment, &user_config);
        }

        // Add project config
        if let Some(ref root) = self.project_search_root
            && let Some(project_config) = self.find_project_config(root)
        {
            figment = Self::merge_file(figment, &project_config);
        }

        // Add explicit files (highest precedence)
        for file in &self.explicit_files {
            figment = Self::merge_file(figment, file);
        }

        let config: Config = figment
            .extract()
            .map_err(|e| ConfigError::Deserialize(Box::new(e)))?;
        tracing::info!(
            log_level = config.log_level.as_str(),
            "configuration loaded"
        );
        Ok(config)
    }

    /// Load configuration, returning an error if no config file is found.
    pub fn load_or_error(self) -> ConfigResult<Config> {
        let has_user = self.include_user_config && self.find_user_config().is_some();
        let has_project = self
            .project_search_root
            .as_ref()
            .and_then(|root| self.find_project_config(root))
            .is_some();
        let has_explicit = !self.explicit_files.is_empty();

        if !has_user && !has_project && !has_explicit {
            return Err(ConfigError::NotFound);
        }

        self.load()
    }

    /// Find project config by walking up from the given directory.
    fn find_project_config(&self, start: &Utf8Path) -> Option<Utf8PathBuf> {
        let mut current = Some(start.to_path_buf());

        while let Some(dir) = current {
            // Check for boundary marker
            if let Some(ref marker) = self.boundary_marker {
                let marker_path = dir.join(marker);
                if marker_path.exists() && dir != start {
                    // Found boundary in a parent dir, stop searching
                    break;
                }
            }

            // Check for config files in this directory (try each extension)
            for ext in CONFIG_EXTENSIONS {
                // Try dotfile first (.scrat.toml)
                let dotfile = dir.join(format!(".{APP_NAME}.{ext}"));
                if dotfile.is_file() {
                    return Some(dotfile);
                }

                // Then try regular name (scrat.toml)
                let regular = dir.join(format!("{APP_NAME}.{ext}"));
                if regular.is_file() {
                    return Some(regular);
                }
            }

            current = dir.parent().map(Utf8Path::to_path_buf);
        }

        None
    }

    /// Find user config in XDG config directory.
    fn find_user_config(&self) -> Option<Utf8PathBuf> {
        let proj_dirs = directories::ProjectDirs::from("", "", APP_NAME)?;
        let config_dir = proj_dirs.config_dir();

        // Try each supported extension
        for ext in CONFIG_EXTENSIONS {
            let config_path = config_dir.join(format!("config.{ext}"));
            if config_path.is_file() {
                return Utf8PathBuf::from_path_buf(config_path).ok();
            }
        }

        None
    }

    /// Merge a config file into the figment, detecting format from extension.
    fn merge_file(figment: Figment, path: &Utf8Path) -> Figment {
        match path.extension() {
            Some("toml") => figment.merge(Toml::file_exact(path.as_str())),
            Some("yaml" | "yml") => figment.merge(Yaml::file_exact(path.as_str())),
            Some("json") => figment.merge(Json::file_exact(path.as_str())),
            _ => figment.merge(Toml::file_exact(path.as_str())),
        }
    }
}

/// Find the project config file path without loading it.
///
/// Useful for commands that need to know where config is located.
pub fn find_project_config<P: AsRef<Utf8Path>>(start: P) -> Option<Utf8PathBuf> {
    ConfigLoader::new()
        .with_project_search(start.as_ref())
        .without_boundary_marker()
        .find_project_config(start.as_ref())
}

/// Get the project directories for XDG-compliant path resolution.
///
/// Returns `None` if the home directory cannot be determined.
fn project_dirs() -> Option<directories::ProjectDirs> {
    directories::ProjectDirs::from("", "", APP_NAME)
}

/// Get the user config directory path.
///
/// Returns `~/.config/scrat/` on Linux, `~/Library/Application Support/scrat/`
/// on macOS, and equivalent on other platforms.
pub fn user_config_dir() -> Option<Utf8PathBuf> {
    let proj_dirs = project_dirs()?;
    Utf8PathBuf::from_path_buf(proj_dirs.config_dir().to_path_buf()).ok()
}

/// Get the user cache directory path.
///
/// Returns `~/.cache/scrat/` on Linux, `~/Library/Caches/scrat/`
/// on macOS, and equivalent on other platforms.
pub fn user_cache_dir() -> Option<Utf8PathBuf> {
    let proj_dirs = project_dirs()?;
    Utf8PathBuf::from_path_buf(proj_dirs.cache_dir().to_path_buf()).ok()
}

/// Get the user data directory path.
///
/// Returns `~/.local/share/scrat/` on Linux, `~/Library/Application Support/scrat/`
/// on macOS, and equivalent on other platforms.
pub fn user_data_dir() -> Option<Utf8PathBuf> {
    let proj_dirs = project_dirs()?;
    Utf8PathBuf::from_path_buf(proj_dirs.data_dir().to_path_buf()).ok()
}

/// Get the local data directory path (machine-specific, not synced).
///
/// Returns `~/.local/share/scrat/` on Linux, `~/Library/Application Support/scrat/`
/// on macOS, and equivalent on other platforms.
pub fn user_data_local_dir() -> Option<Utf8PathBuf> {
    let proj_dirs = project_dirs()?;
    Utf8PathBuf::from_path_buf(proj_dirs.data_local_dir().to_path_buf()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.log_level, LogLevel::Info);
        assert!(config.log_dir.is_none());
    }

    #[test]
    fn test_loader_builds_with_defaults() {
        let loader = ConfigLoader::new()
            .with_user_config(false)
            .without_boundary_marker();

        // Should succeed with defaults even if no files found
        let config = loader.load().unwrap();
        assert_eq!(config.log_level, LogLevel::Info);
    }

    #[test]
    fn test_single_file_overrides_default() {
        let tmp = TempDir::new().unwrap();
        let config_path = tmp.path().join("config.toml");
        fs::write(
            &config_path,
            r#"log_level = "debug"
log_dir = "/tmp/scrat"
"#,
        )
        .unwrap();

        // Convert to Utf8PathBuf for API call
        let config_path = Utf8PathBuf::try_from(config_path).unwrap();

        let config = ConfigLoader::new()
            .with_user_config(false)
            .with_file(&config_path)
            .load()
            .unwrap();

        assert_eq!(config.log_level, LogLevel::Debug);
        assert_eq!(
            config.log_dir.as_ref().map(|dir| dir.as_str()),
            Some("/tmp/scrat")
        );
    }

    #[test]
    fn test_later_file_overrides_earlier() {
        let tmp = TempDir::new().unwrap();

        let base_config = tmp.path().join("base.toml");
        fs::write(&base_config, r#"log_level = "warn""#).unwrap();

        let override_config = tmp.path().join("override.toml");
        fs::write(&override_config, r#"log_level = "error""#).unwrap();

        // Convert to Utf8PathBuf for API calls
        let base_config = Utf8PathBuf::try_from(base_config).unwrap();
        let override_config = Utf8PathBuf::try_from(override_config).unwrap();

        let config = ConfigLoader::new()
            .with_user_config(false)
            .with_file(&base_config)
            .with_file(&override_config)
            .load()
            .unwrap();

        // Later file wins
        assert_eq!(config.log_level, LogLevel::Error);
    }

    #[test]
    fn test_project_config_discovery() {
        let tmp = TempDir::new().unwrap();
        let project_dir = tmp.path().join("project");
        let sub_dir = project_dir.join("src").join("deep");
        fs::create_dir_all(&sub_dir).unwrap();

        // Create config in project root
        let config_path = project_dir.join(".scrat.toml");
        fs::write(&config_path, r#"log_level = "debug""#).unwrap();

        // Convert to Utf8PathBuf for API call
        let sub_dir = Utf8PathBuf::try_from(sub_dir).unwrap();

        // Search from deep subdirectory
        let config = ConfigLoader::new()
            .with_user_config(false)
            .without_boundary_marker()
            .with_project_search(&sub_dir)
            .load()
            .unwrap();

        assert_eq!(config.log_level, LogLevel::Debug);
    }

    #[test]
    fn test_boundary_marker_stops_search() {
        let tmp = TempDir::new().unwrap();

        // Create structure: /parent/config.toml, /parent/child/.git/, /parent/child/work/
        let parent = tmp.path().join("parent");
        let child = parent.join("child");
        let work = child.join("work");
        fs::create_dir_all(&work).unwrap();

        // Config in parent (should NOT be found due to .git boundary)
        fs::write(parent.join(".scrat.toml"), r#"log_level = "warn""#).unwrap();

        // .git marker in child
        fs::create_dir(child.join(".git")).unwrap();

        // Convert to Utf8PathBuf for API call
        let work = Utf8PathBuf::try_from(work).unwrap();

        // Search from work directory - should not find parent config
        let config = ConfigLoader::new()
            .with_user_config(false)
            .with_boundary_marker(".git")
            .with_project_search(&work)
            .load()
            .unwrap();

        // Should get default since config is beyond boundary
        assert_eq!(config.log_level, LogLevel::Info);
    }

    #[test]
    fn test_explicit_file_overrides_project_config() {
        let tmp = TempDir::new().unwrap();

        // Project config
        let project_config = tmp.path().join(".scrat.toml");
        fs::write(&project_config, r#"log_level = "warn""#).unwrap();

        // Explicit override
        let override_config = tmp.path().join("override.toml");
        fs::write(&override_config, r#"log_level = "error""#).unwrap();

        // Convert to Utf8PathBuf for API calls
        let tmp_path = Utf8PathBuf::try_from(tmp.path().to_path_buf()).unwrap();
        let override_config = Utf8PathBuf::try_from(override_config).unwrap();

        let config = ConfigLoader::new()
            .with_user_config(false)
            .without_boundary_marker()
            .with_project_search(&tmp_path)
            .with_file(&override_config)
            .load()
            .unwrap();

        // Explicit file wins over project config
        assert_eq!(config.log_level, LogLevel::Error);
    }

    #[test]
    fn test_load_or_error_fails_when_no_config() {
        let result = ConfigLoader::new()
            .with_user_config(false)
            .without_boundary_marker()
            .load_or_error();

        assert!(matches!(result, Err(ConfigError::NotFound)));
    }

    #[test]
    fn test_load_or_error_succeeds_with_explicit_file() {
        let tmp = TempDir::new().unwrap();
        let config_path = tmp.path().join("config.toml");
        fs::write(&config_path, r#"log_level = "debug""#).unwrap();

        // Convert to Utf8PathBuf for API call
        let config_path = Utf8PathBuf::try_from(config_path).unwrap();

        let config = ConfigLoader::new()
            .with_user_config(false)
            .with_file(&config_path)
            .load_or_error()
            .unwrap();

        assert_eq!(config.log_level, LogLevel::Debug);
    }

    #[test]
    fn test_user_config_dir() {
        // Should return Some on most systems
        let dir = user_config_dir();
        if let Some(path) = dir {
            assert!(path.as_str().contains("scrat"));
        }
    }

    #[test]
    fn test_default_config_has_no_sections() {
        let config = Config::default();
        assert!(config.project.is_none());
        assert!(config.version.is_none());
        assert!(config.commands.is_none());
        assert!(config.release.is_none());
    }

    #[test]
    fn test_config_with_project_section() {
        let tmp = TempDir::new().unwrap();
        let config_path = tmp.path().join("config.toml");
        fs::write(
            &config_path,
            r#"
[project]
type = "rust"
release_branch = "main"
"#,
        )
        .unwrap();

        let config_path = Utf8PathBuf::try_from(config_path).unwrap();
        let config = ConfigLoader::new()
            .with_user_config(false)
            .with_file(&config_path)
            .load()
            .unwrap();

        let project = config.project.unwrap();
        assert_eq!(
            project.project_type,
            Some(crate::ecosystem::Ecosystem::Rust)
        );
        assert_eq!(project.release_branch.as_deref(), Some("main"));
    }

    #[test]
    fn test_config_with_commands_section() {
        let tmp = TempDir::new().unwrap();
        let config_path = tmp.path().join("config.toml");
        fs::write(
            &config_path,
            r#"
[commands]
test = "cargo nextest run"
build = "cargo build --release"
"#,
        )
        .unwrap();

        let config_path = Utf8PathBuf::try_from(config_path).unwrap();
        let config = ConfigLoader::new()
            .with_user_config(false)
            .with_file(&config_path)
            .load()
            .unwrap();

        let commands = config.commands.unwrap();
        assert_eq!(commands.test.as_deref(), Some("cargo nextest run"));
        assert_eq!(commands.build.as_deref(), Some("cargo build --release"));
        assert!(commands.publish.is_none());
    }

    #[test]
    fn test_config_with_release_section() {
        let tmp = TempDir::new().unwrap();
        let config_path = tmp.path().join("config.toml");
        fs::write(
            &config_path,
            r#"
[release]
changelog_tool = "git-cliff"
github_release = true
assets = ["release-card.png", "checksums.txt"]
"#,
        )
        .unwrap();

        let config_path = Utf8PathBuf::try_from(config_path).unwrap();
        let config = ConfigLoader::new()
            .with_user_config(false)
            .with_file(&config_path)
            .load()
            .unwrap();

        let release = config.release.unwrap();
        assert_eq!(
            release.changelog_tool,
            Some(crate::ecosystem::ChangelogTool::GitCliff)
        );
        assert_eq!(release.github_release, Some(true));
        assert_eq!(
            release.assets,
            Some(vec![
                "release-card.png".to_string(),
                "checksums.txt".to_string()
            ])
        );
    }

    #[test]
    fn test_config_with_release_draft_and_title() {
        let tmp = TempDir::new().unwrap();
        let config_path = tmp.path().join("config.toml");
        fs::write(
            &config_path,
            r#"
[release]
draft = true
title = "{repo} {tag}"
discussion_category = "releases"
"#,
        )
        .unwrap();

        let config_path = Utf8PathBuf::try_from(config_path).unwrap();
        let config = ConfigLoader::new()
            .with_user_config(false)
            .with_file(&config_path)
            .load()
            .unwrap();

        let release = config.release.unwrap();
        assert_eq!(release.draft, Some(true));
        assert_eq!(release.title.as_deref(), Some("{repo} {tag}"));
        assert_eq!(release.discussion_category.as_deref(), Some("releases"));
    }

    #[test]
    fn test_release_notes_template_config() {
        let tmp = TempDir::new().unwrap();
        let config_path = tmp.path().join("config.toml");
        fs::write(
            &config_path,
            r#"
[release]
changelog_tool = "git-cliff"
notes_template = "templates/my-notes.tera"
"#,
        )
        .unwrap();

        let config_path = Utf8PathBuf::try_from(config_path).unwrap();
        let config = ConfigLoader::new()
            .with_user_config(false)
            .with_file(&config_path)
            .load()
            .unwrap();

        let release = config.release.unwrap();
        assert_eq!(
            release.notes_template.as_deref(),
            Some("templates/my-notes.tera")
        );
    }

    #[test]
    fn test_config_with_hooks_section() {
        let tmp = TempDir::new().unwrap();
        let config_path = tmp.path().join("config.toml");
        fs::write(
            &config_path,
            r#"
[hooks]
post_bump = ["echo {version}", "ll-graphics --version {version}"]
pre_publish = ["cargo build --release"]
"#,
        )
        .unwrap();

        let config_path = Utf8PathBuf::try_from(config_path).unwrap();
        let config = ConfigLoader::new()
            .with_user_config(false)
            .with_file(&config_path)
            .load()
            .unwrap();

        let hooks = config.hooks.unwrap();
        assert_eq!(hooks.post_bump.as_ref().unwrap().len(), 2);
        assert_eq!(
            hooks.pre_publish.as_ref().unwrap(),
            &["cargo build --release"]
        );
        assert!(hooks.pre_bump.is_none());
    }

    #[test]
    fn test_config_with_all_hook_phases() {
        let tmp = TempDir::new().unwrap();
        let config_path = tmp.path().join("config.toml");
        fs::write(
            &config_path,
            r#"
[hooks]
pre_ship = ["echo starting"]
post_ship = ["echo done"]
pre_test = ["echo pre-test"]
post_test = ["echo post-test"]
pre_bump = ["echo pre-bump"]
post_bump = ["echo post-bump"]
pre_publish = ["echo pre-publish"]
post_publish = ["echo post-publish"]
pre_tag = ["echo pre-tag"]
post_tag = ["echo post-tag"]
pre_release = ["echo pre-release"]
post_release = ["echo post-release"]
"#,
        )
        .unwrap();

        let config_path = Utf8PathBuf::try_from(config_path).unwrap();
        let config = ConfigLoader::new()
            .with_user_config(false)
            .with_file(&config_path)
            .load()
            .unwrap();

        let hooks = config.hooks.unwrap();
        assert!(hooks.pre_ship.is_some());
        assert!(hooks.post_ship.is_some());
        assert!(hooks.pre_test.is_some());
        assert!(hooks.post_test.is_some());
        assert!(hooks.pre_bump.is_some());
        assert!(hooks.post_bump.is_some());
        assert!(hooks.pre_publish.is_some());
        assert!(hooks.post_publish.is_some());
        assert!(hooks.pre_tag.is_some());
        assert!(hooks.post_tag.is_some());
        assert!(hooks.pre_release.is_some());
        assert!(hooks.post_release.is_some());
    }

    #[test]
    fn test_config_with_ship_section() {
        let tmp = TempDir::new().unwrap();
        let config_path = tmp.path().join("config.toml");
        fs::write(
            &config_path,
            r#"
[ship]
confirm = false
"#,
        )
        .unwrap();

        let config_path = Utf8PathBuf::try_from(config_path).unwrap();
        let config = ConfigLoader::new()
            .with_user_config(false)
            .with_file(&config_path)
            .load()
            .unwrap();

        let ship = config.ship.unwrap();
        assert_eq!(ship.confirm, Some(false));
    }

    #[test]
    fn test_config_ship_defaults_to_none() {
        let config = Config::default();
        assert!(config.ship.is_none());
    }

    #[test]
    fn test_config_ignores_unknown_sections() {
        let tmp = TempDir::new().unwrap();
        let config_path = tmp.path().join("config.toml");
        fs::write(
            &config_path,
            r#"
log_level = "warn"
"#,
        )
        .unwrap();

        let config_path = Utf8PathBuf::try_from(config_path).unwrap();
        let config = ConfigLoader::new()
            .with_user_config(false)
            .with_file(&config_path)
            .load()
            .unwrap();

        assert_eq!(config.log_level, LogLevel::Warn);
        assert!(config.project.is_none());
    }
}
