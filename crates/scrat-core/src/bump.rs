//! Version bump planning and execution.
//!
//! All orchestration logic lives here. The CLI is purely a display layer.
//!
//! # Two-phase workflow
//!
//! 1. **Plan** ([`plan_bump`]) — detect ecosystem, resolve version strategy,
//!    compute the next version (or gather interactive context).
//! 2. **Execute** ([`ReadyBump::execute`]) — update project files and
//!    generate changelog.
//!
//! If the plan comes back as [`BumpPlan::NeedsInteraction`], the CLI prompts
//! the user and calls [`resolve_interactive`] to get a [`ReadyBump`].

use std::process::Command;

use camino::Utf8Path;
use semver::Version;
use serde::Serialize;
use thiserror::Error;
use tracing::{debug, info, instrument};

use crate::config::Config;
use crate::ecosystem::{ChangelogTool, Ecosystem, ProjectDetection, VersionStrategy};
use crate::version::{self, conventional, explicit, interactive};

// ──────────────────────────────────────────────
// Errors
// ──────────────────────────────────────────────

/// Errors from bump operations.
#[derive(Error, Debug)]
pub enum BumpError {
    /// A shell command failed during the bump.
    #[error("{tool} failed: {message}")]
    ToolFailed {
        /// Tool name.
        tool: String,
        /// Error details.
        message: String,
    },

    /// No bump tool available for this ecosystem.
    #[error("no bump tool available (install cargo-edit for Rust)")]
    NoBumpTool,

    /// Ecosystem not supported for bump operations.
    #[error("bump not yet supported for {0} ecosystem")]
    UnsupportedEcosystem(Ecosystem),

    /// Project detection failed.
    #[error("project detection failed: {0}")]
    Detection(String),

    /// Version computation failed.
    #[error(transparent)]
    Version(#[from] crate::version::VersionError),
}

/// Result alias for bump operations.
pub type BumpResult<T> = Result<T, BumpError>;

// ──────────────────────────────────────────────
// Plan types
// ──────────────────────────────────────────────

/// The result of planning a bump — either ready to execute or needs user input.
#[derive(Debug)]
pub enum BumpPlan {
    /// Version fully determined (explicit or conventional commits).
    Ready(ReadyBump),
    /// Interactive mode — the CLI must prompt the user and call [`resolve_interactive`].
    NeedsInteraction(InteractiveBump),
}

/// A bump plan that is ready to execute.
#[derive(Debug, Clone)]
pub struct ReadyBump {
    /// The previous version (from tags, or 0.0.0 for first release).
    pub previous: Version,
    /// The computed next version.
    pub next: Version,
    /// How the version was determined.
    pub strategy: VersionStrategy,
    /// Detected ecosystem and tools.
    pub detection: ProjectDetection,
    /// Additional files to update with the new version.
    pub version_files: Vec<crate::config::VersionFileConfig>,
}

/// A bump plan that requires the user to pick a version interactively.
#[derive(Debug)]
pub struct InteractiveBump {
    /// Context for the interactive picker (commits, candidates).
    pub context: interactive::InteractiveContext,
    /// Detected ecosystem and tools.
    pub detection: ProjectDetection,
    /// Additional files to update with the new version.
    pub version_files: Vec<crate::config::VersionFileConfig>,
}

// ──────────────────────────────────────────────
// Plan
// ──────────────────────────────────────────────

/// Plan a version bump: detect ecosystem, resolve strategy, compute version.
///
/// Returns [`BumpPlan::Ready`] when the version can be determined automatically
/// (explicit or conventional commits), or [`BumpPlan::NeedsInteraction`] when
/// the user must pick a version from candidates.
///
/// # Arguments
/// * `project_root` — project working directory
/// * `config` — loaded configuration (for strategy overrides)
/// * `explicit_version` — if set, overrides everything (from CLI `--version` flag)
#[instrument(skip(config), fields(%project_root))]
pub fn plan_bump(
    project_root: &Utf8Path,
    config: &Config,
    explicit_version: Option<&str>,
) -> BumpResult<BumpPlan> {
    // Step 1: Detect ecosystem (config override > auto-detect)
    let detection = crate::detect::resolve_detection(project_root, config).ok_or_else(|| {
        BumpError::Detection(
            "could not detect project type — use `project.type` in config or select interactively"
                .into(),
        )
    })?;
    plan_bump_with_detection(project_root, config, explicit_version, detection)
}

/// Plan a bump using a pre-computed [`ProjectDetection`].
///
/// Equivalent to [`plan_bump`] but skips the detection phase. Used by
/// [`crate::ship::plan_ship`] to avoid scanning marker files and
/// probing `PATH` twice in a single ship run.
#[instrument(skip(config, detection), fields(%project_root, ecosystem = %detection.ecosystem))]
pub fn plan_bump_with_detection(
    project_root: &Utf8Path,
    config: &Config,
    explicit_version: Option<&str>,
    detection: ProjectDetection,
) -> BumpResult<BumpPlan> {
    let version_files = config.version_files.clone().unwrap_or_default();

    // Step 2: Determine version strategy
    // CLI --version flag > config override > auto-detected
    let strategy = explicit_version.map_or_else(
        || resolve_strategy(config, &detection),
        |v| VersionStrategy::Explicit(v.to_owned()),
    );

    debug!(%strategy, "resolved version strategy");

    // Step 3: Compute version (or gather interactive context)
    match strategy {
        VersionStrategy::Explicit(ref v) => {
            let next = explicit::validate_explicit(v)?;
            let previous = current_or_zero()?;
            Ok(BumpPlan::Ready(ReadyBump {
                previous,
                next,
                strategy,
                detection,
                version_files,
            }))
        }
        VersionStrategy::ConventionalCommits { tool } => {
            let cliff_config_override = config
                .version
                .as_ref()
                .and_then(|v| v.cliff_config.as_deref());
            let next = conventional::compute_next_version(
                tool,
                detection.ecosystem,
                cliff_config_override,
            )?;
            let previous = current_or_zero()?;
            Ok(BumpPlan::Ready(ReadyBump {
                previous,
                next,
                strategy: VersionStrategy::ConventionalCommits { tool },
                detection,
                version_files,
            }))
        }
        VersionStrategy::Interactive => {
            let context = interactive::gather_interactive_context(20)?;
            Ok(BumpPlan::NeedsInteraction(InteractiveBump {
                context,
                detection,
                version_files,
            }))
        }
    }
}

/// Finalize an interactive plan with the user's chosen version.
pub fn resolve_interactive(plan: InteractiveBump, chosen_version: Version) -> ReadyBump {
    let previous = plan
        .context
        .current_version
        .clone()
        .unwrap_or_else(|| Version::new(0, 0, 0));
    ReadyBump {
        previous,
        next: chosen_version,
        strategy: VersionStrategy::Interactive,
        detection: plan.detection,
        version_files: plan.version_files,
    }
}

/// Determine the version strategy from config overrides or auto-detection.
fn resolve_strategy(config: &Config, detection: &ProjectDetection) -> VersionStrategy {
    // Config strategy override
    if let Some(ref vc) = config.version
        && let Some(ref s) = vc.strategy
    {
        match s.as_str() {
            "conventional-commits" => {
                // Use the detected changelog tool, or default to git-cliff
                let tool = detection
                    .tools
                    .changelog_tool
                    .unwrap_or(ChangelogTool::GitCliff);
                return VersionStrategy::ConventionalCommits { tool };
            }
            "interactive" => return VersionStrategy::Interactive,
            // Anything else: fall through to detection
            _ => {}
        }
    }
    detection.version_strategy.clone()
}

/// Get the current version from tags, defaulting to 0.0.0 for first releases.
fn current_or_zero() -> BumpResult<Version> {
    let current = version::current_version_from_tags()?;
    Ok(current.unwrap_or_else(|| Version::new(0, 0, 0)))
}

// ──────────────────────────────────────────────
// Execute
// ──────────────────────────────────────────────

/// Result of a successful bump operation.
#[derive(Debug, Clone, Serialize)]
pub struct BumpOutcome {
    /// The previous version.
    pub previous: Version,
    /// The new version.
    pub new: Version,
    /// Whether the changelog was updated.
    pub changelog_updated: bool,
    /// Files that were modified.
    pub modified_files: Vec<String>,
}

impl ReadyBump {
    /// Execute the bump: update project files and optionally generate changelog.
    #[instrument(skip(self), fields(ecosystem = %self.detection.ecosystem, next = %self.next))]
    pub fn execute(
        &self,
        project_root: &Utf8Path,
        update_changelog: bool,
    ) -> BumpResult<BumpOutcome> {
        let mut modified_files = Vec::new();

        // Update version in project files (Generic has no project files to update)
        match self.detection.ecosystem {
            Ecosystem::Rust => {
                bump_rust_version(project_root, &self.next, &self.detection)?;
                modified_files.push("Cargo.toml".into());
            }
            Ecosystem::Node => {
                if bump_node_version(project_root, &self.next)? {
                    modified_files.push("package.json".into());
                }
            }
            Ecosystem::Go | Ecosystem::Swift => {
                debug!(%self.detection.ecosystem, "version lives in git tags, no file to bump");
            }
            Ecosystem::Php => {
                if bump_composer_version(project_root, &self.next)? {
                    modified_files.push("composer.json".into());
                } else {
                    debug!("composer.json has no version field, skipping");
                }
            }
            Ecosystem::Python => {
                if bump_pyproject_version(project_root, &self.next)? {
                    modified_files.push("pyproject.toml".into());
                } else {
                    debug!("pyproject.toml has no version field, skipping");
                }
            }
            Ecosystem::Ruby => {
                let files = bump_ruby_version(project_root, &self.next)?;
                if files.is_empty() && self.version_files.is_empty() {
                    return Err(BumpError::ToolFailed {
                        tool: "ruby".into(),
                        message: "no lib/**/version.rb or gemspec with a literal version \
                                  was found, and no `[[version_files]]` entries are \
                                  configured — the release would be tagged without \
                                  updating any file"
                            .into(),
                    });
                }
                modified_files.extend(files);
            }
            Ecosystem::Generic => {
                debug!("generic ecosystem — no project files to bump");
            }
        }

        // Update configured version files
        if !self.version_files.is_empty() {
            let vf_modified = crate::version_files::bump_version_files(
                project_root,
                &self.version_files,
                &self.next.to_string(),
            )?;
            modified_files.extend(vf_modified);
        }

        // Generate/update changelog (if requested and tool available)
        let changelog_updated = if update_changelog {
            if let Some(tool) = self.detection.tools.changelog_tool {
                generate_changelog(project_root, &self.next, tool)?;
                modified_files.push("CHANGELOG.md".into());
                true
            } else {
                debug!("no changelog tool configured, skipping");
                false
            }
        } else {
            false
        };

        info!(
            previous = %self.previous,
            new = %self.next,
            changelog_updated,
            "bump complete"
        );

        Ok(BumpOutcome {
            previous: self.previous.clone(),
            new: self.next.clone(),
            changelog_updated,
            modified_files,
        })
    }
}

// ──────────────────────────────────────────────
// Internal helpers
// ──────────────────────────────────────────────

/// Bump the version in Cargo.toml using `cargo set-version`.
fn bump_rust_version(
    project_root: &Utf8Path,
    version: &Version,
    detection: &ProjectDetection,
) -> BumpResult<()> {
    let Some(ref bump_cmd) = detection.tools.bump_cmd else {
        return Err(BumpError::NoBumpTool);
    };

    debug!(%bump_cmd, %version, "bumping Rust version");

    let parts: Vec<&str> = bump_cmd.split_whitespace().collect();
    let (bin, args) = parts.split_first().unwrap_or((&"cargo", &[]));

    let output = Command::new(bin)
        .args(args)
        .arg(version.to_string())
        .current_dir(project_root.as_std_path())
        .output()
        .map_err(|e| BumpError::ToolFailed {
            tool: bump_cmd.clone(),
            message: format!("failed to execute: {e}"),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(BumpError::ToolFailed {
            tool: bump_cmd.clone(),
            message: stderr,
        });
    }

    Ok(())
}

/// Bump the version in `package.json` directly.
///
/// scrat edits only `package.json` — it is intentionally *not* a
/// lockfile manager. If the user needs `package-lock.json` (or
/// `yarn.lock`, `pnpm-lock.yaml`) synced after the bump, that's their
/// package manager's job (e.g. a pre-commit scrat hook running
/// `npm install --package-lock-only`).
///
/// Returns `true` if the file was modified.
fn bump_node_version(project_root: &Utf8Path, version: &Version) -> BumpResult<bool> {
    let package_path = project_root.join("package.json");
    let content = std::fs::read_to_string(&package_path).map_err(|e| BumpError::ToolFailed {
        tool: "package.json".into(),
        message: format!("failed to read: {e}"),
    })?;

    let mut parsed: serde_json::Value =
        serde_json::from_str(&content).map_err(|e| BumpError::ToolFailed {
            tool: "package.json".into(),
            message: format!("failed to parse: {e}"),
        })?;

    if parsed.get("version").and_then(|v| v.as_str()).is_none() {
        return Err(BumpError::ToolFailed {
            tool: "package.json".into(),
            message: "no `version` field found — cannot bump".into(),
        });
    }

    parsed["version"] = serde_json::Value::String(version.to_string());

    // npm convention: 2-space indent, trailing newline
    let output = serde_json::to_string_pretty(&parsed).map_err(|e| BumpError::ToolFailed {
        tool: "package.json".into(),
        message: format!("failed to serialize: {e}"),
    })?;

    std::fs::write(&package_path, format!("{output}\n")).map_err(|e| BumpError::ToolFailed {
        tool: "package.json".into(),
        message: format!("failed to write: {e}"),
    })?;

    debug!(%version, "bumped package.json version");
    Ok(true)
}

/// Bump the version in `composer.json` if it has a `"version"` field.
///
/// Returns `true` if the file was modified, `false` if no version field exists.
fn bump_composer_version(project_root: &Utf8Path, version: &Version) -> BumpResult<bool> {
    let composer_path = project_root.join("composer.json");
    let content = match std::fs::read_to_string(&composer_path) {
        Ok(c) => c,
        Err(_) => return Ok(false),
    };

    let mut parsed: serde_json::Value =
        serde_json::from_str(&content).map_err(|e| BumpError::ToolFailed {
            tool: "composer.json".into(),
            message: format!("failed to parse: {e}"),
        })?;

    // Only write if the field already exists — don't add it if absent
    if parsed.get("version").and_then(|v| v.as_str()).is_none() {
        return Ok(false);
    }

    parsed["version"] = serde_json::Value::String(version.to_string());

    let output = serde_json::to_string_pretty(&parsed).map_err(|e| BumpError::ToolFailed {
        tool: "composer.json".into(),
        message: format!("failed to serialize: {e}"),
    })?;

    // Composer convention: trailing newline
    std::fs::write(&composer_path, format!("{output}\n")).map_err(|e| BumpError::ToolFailed {
        tool: "composer.json".into(),
        message: format!("failed to write: {e}"),
    })?;

    debug!(%version, "bumped composer.json version");
    Ok(true)
}

/// Bump Ruby project versions. Updates every `lib/**/version.rb` file that
/// has a `VERSION = "..."` assignment, plus any top-level `*.gemspec` that
/// contains a literal `<spec>.version = "..."` line.
///
/// Returns the paths (relative to `project_root`) of files that were
/// actually modified. Returns an empty `Vec` if no standard Ruby version
/// files were found — callers may fall back to `[[version_files]]`.
fn bump_ruby_version(project_root: &Utf8Path, version: &Version) -> BumpResult<Vec<String>> {
    let new_version = version.to_string();
    let mut modified = Vec::new();

    // 1. lib/**/version.rb — the canonical location for gem versions.
    let lib_dir = project_root.join("lib");
    if lib_dir.is_dir() {
        let pattern = format!("{lib_dir}/**/version.rb");
        let paths = glob::glob(&pattern).map_err(|e| BumpError::ToolFailed {
            tool: "ruby".into(),
            message: format!("glob pattern error: {e}"),
        })?;
        for entry in paths {
            let path = entry.map_err(|e| BumpError::ToolFailed {
                tool: "ruby".into(),
                message: format!("glob entry error: {e}"),
            })?;
            let path =
                camino::Utf8PathBuf::from_path_buf(path).map_err(|p| BumpError::ToolFailed {
                    tool: "ruby".into(),
                    message: format!("non-UTF-8 path: {}", p.display()),
                })?;
            if update_ruby_version_file(&path, &new_version)? {
                let rel = path
                    .strip_prefix(project_root)
                    .map(camino::Utf8Path::to_path_buf)
                    .unwrap_or_else(|_| path.clone());
                modified.push(rel.to_string());
            }
        }
    }

    // 2. *.gemspec — only update literal `<x>.version = "..."` assignments;
    //    skip `spec.version = MyGem::VERSION` constant references.
    let read_dir =
        std::fs::read_dir(project_root.as_std_path()).map_err(|e| BumpError::ToolFailed {
            tool: "ruby".into(),
            message: format!("failed to read project root: {e}"),
        })?;
    for entry in read_dir {
        let entry = entry.map_err(|e| BumpError::ToolFailed {
            tool: "ruby".into(),
            message: format!("read_dir entry error: {e}"),
        })?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("gemspec") {
            continue;
        }
        let path = camino::Utf8PathBuf::from_path_buf(path).map_err(|p| BumpError::ToolFailed {
            tool: "ruby".into(),
            message: format!("non-UTF-8 gemspec path: {}", p.display()),
        })?;
        if update_gemspec_version_file(&path, &new_version)? {
            let rel = path
                .strip_prefix(project_root)
                .map(camino::Utf8Path::to_path_buf)
                .unwrap_or_else(|_| path.clone());
            modified.push(rel.to_string());
        }
    }

    debug!(files = ?modified, "ruby version bump complete");
    Ok(modified)
}

/// Rewrite a Ruby `VERSION = "..."` assignment in-place.
/// Returns `true` if the file was modified.
fn update_ruby_version_file(path: &Utf8Path, new_version: &str) -> BumpResult<bool> {
    let content =
        std::fs::read_to_string(path.as_std_path()).map_err(|e| BumpError::ToolFailed {
            tool: "ruby".into(),
            message: format!("failed to read {path}: {e}"),
        })?;

    let mut changed = false;
    let mut out_lines: Vec<String> = Vec::with_capacity(content.lines().count());
    for line in content.lines() {
        if let Some(replaced) = replace_ruby_version_line(line, new_version) {
            if replaced != line {
                changed = true;
            }
            out_lines.push(replaced);
        } else {
            out_lines.push(line.to_string());
        }
    }

    if !changed {
        return Ok(false);
    }

    let mut out = out_lines.join("\n");
    if content.ends_with('\n') {
        out.push('\n');
    }
    std::fs::write(path.as_std_path(), out).map_err(|e| BumpError::ToolFailed {
        tool: "ruby".into(),
        message: format!("failed to write {path}: {e}"),
    })?;
    Ok(true)
}

/// Rewrite `<x>.version = "..."` lines in a gemspec.
///
/// Only touches literal string assignments — leaves constant references
/// like `spec.version = MyGem::VERSION` alone so the version.rb update
/// remains the source of truth.
fn update_gemspec_version_file(path: &Utf8Path, new_version: &str) -> BumpResult<bool> {
    let content =
        std::fs::read_to_string(path.as_std_path()).map_err(|e| BumpError::ToolFailed {
            tool: "ruby".into(),
            message: format!("failed to read {path}: {e}"),
        })?;

    let mut changed = false;
    let mut out_lines: Vec<String> = Vec::with_capacity(content.lines().count());
    for line in content.lines() {
        if let Some(replaced) = replace_gemspec_version_line(line, new_version) {
            if replaced != line {
                changed = true;
            }
            out_lines.push(replaced);
        } else {
            out_lines.push(line.to_string());
        }
    }

    if !changed {
        return Ok(false);
    }

    let mut out = out_lines.join("\n");
    if content.ends_with('\n') {
        out.push('\n');
    }
    std::fs::write(path.as_std_path(), out).map_err(|e| BumpError::ToolFailed {
        tool: "ruby".into(),
        message: format!("failed to write {path}: {e}"),
    })?;
    Ok(true)
}

/// Replace the literal in a `VERSION = "x.y.z"` (or `'x.y.z'`) line.
///
/// Preserves indentation, the receiver (bare `VERSION`, or `self::VERSION`),
/// quote style, and anything trailing (e.g. `.freeze`, comments).
/// Returns `None` if the line isn't a VERSION assignment.
fn replace_ruby_version_line(line: &str, new_version: &str) -> Option<String> {
    let bytes = line.as_bytes();
    let mut i = 0;

    // Skip leading whitespace
    while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
        i += 1;
    }
    // Skip comment lines
    if i < bytes.len() && bytes[i] == b'#' {
        return None;
    }
    // Must start with `VERSION` as a standalone token.
    if !line[i..].starts_with("VERSION") {
        return None;
    }
    // Ensure `VERSION` isn't a suffix of another identifier (e.g. `FOO_VERSION`).
    if i > 0 {
        let prev = bytes[i - 1];
        if prev.is_ascii_alphanumeric() || prev == b'_' {
            return None;
        }
    }
    i += "VERSION".len();
    // Next char must be whitespace or '='
    if i >= bytes.len() {
        return None;
    }
    let next_byte = bytes[i];
    if next_byte != b' ' && next_byte != b'\t' && next_byte != b'=' {
        return None;
    }
    // Find '='
    while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
        i += 1;
    }
    if i >= bytes.len() || bytes[i] != b'=' {
        return None;
    }
    i += 1;
    // Reject '=='
    if i < bytes.len() && bytes[i] == b'=' {
        return None;
    }
    // Skip whitespace
    while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
        i += 1;
    }
    // Expect an opening quote
    if i >= bytes.len() {
        return None;
    }
    let quote = bytes[i];
    if quote != b'"' && quote != b'\'' {
        return None;
    }
    i += 1;
    let content_start = i;
    while i < bytes.len() && bytes[i] != quote {
        // Reject embedded backslash escapes — version strings don't use them
        if bytes[i] == b'\\' {
            return None;
        }
        i += 1;
    }
    if i >= bytes.len() {
        return None;
    }
    let content_end = i;

    let mut result = String::with_capacity(line.len() + new_version.len());
    result.push_str(&line[..content_start]);
    result.push_str(new_version);
    result.push_str(&line[content_end..]);
    Some(result)
}

/// Replace the literal in `<x>.version = "y.z"` lines in a gemspec.
///
/// Matches `<receiver>.version` where `<receiver>` is an identifier
/// (typically `spec`, `s`, `gem`, `Gem::Specification.new do |spec|` →
/// `spec`). Returns `None` for constant references like
/// `spec.version = MyGem::VERSION`, so the version.rb update remains the
/// source of truth.
fn replace_gemspec_version_line(line: &str, new_version: &str) -> Option<String> {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
        i += 1;
    }
    if i < bytes.len() && bytes[i] == b'#' {
        return None;
    }
    // Parse receiver — an identifier starting with letter or underscore.
    let receiver_start = i;
    if i >= bytes.len() || !(bytes[i].is_ascii_alphabetic() || bytes[i] == b'_') {
        return None;
    }
    while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
        i += 1;
    }
    if receiver_start == i {
        return None;
    }
    // Expect `.version`
    if !line[i..].starts_with(".version") {
        return None;
    }
    i += ".version".len();
    // `.version` must be a complete token (not e.g. `.versioned`).
    if i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
        return None;
    }
    // Skip whitespace
    while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
        i += 1;
    }
    // Expect '='
    if i >= bytes.len() || bytes[i] != b'=' {
        return None;
    }
    i += 1;
    if i < bytes.len() && bytes[i] == b'=' {
        return None;
    }
    while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
        i += 1;
    }
    // Expect a quote — if the next char isn't a quote, it's a constant
    // reference (MyGem::VERSION). Skip.
    if i >= bytes.len() {
        return None;
    }
    let quote = bytes[i];
    if quote != b'"' && quote != b'\'' {
        return None;
    }
    i += 1;
    let content_start = i;
    while i < bytes.len() && bytes[i] != quote {
        if bytes[i] == b'\\' {
            return None;
        }
        i += 1;
    }
    if i >= bytes.len() {
        return None;
    }
    let content_end = i;

    let mut result = String::with_capacity(line.len() + new_version.len());
    result.push_str(&line[..content_start]);
    result.push_str(new_version);
    result.push_str(&line[content_end..]);
    Some(result)
}

/// Bump the version in `pyproject.toml` if it has a `version` field under `[project]`.
///
/// Returns `true` if the file was modified, `false` if no version field exists.
fn bump_pyproject_version(project_root: &Utf8Path, version: &Version) -> BumpResult<bool> {
    let pyproject_path = project_root.join("pyproject.toml");
    let content = match std::fs::read_to_string(&pyproject_path) {
        Ok(c) => c,
        Err(_) => return Ok(false),
    };

    // Look for `version = "..."` under `[project]` section
    let mut in_project = false;
    let mut found = false;
    let mut lines: Vec<String> = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_project = trimmed == "[project]";
        }
        if in_project
            && trimmed.starts_with("version")
            && let Some((key, _)) = trimmed.split_once('=')
            && key.trim() == "version"
        {
            lines.push(format!("version = \"{version}\""));
            found = true;
        } else {
            lines.push(line.to_string());
        }
    }

    if !found {
        return Ok(false);
    }

    std::fs::write(&pyproject_path, lines.join("\n") + "\n").map_err(|e| {
        BumpError::ToolFailed {
            tool: "pyproject.toml".into(),
            message: format!("failed to write: {e}"),
        }
    })?;

    debug!(%version, "bumped pyproject.toml version");
    Ok(true)
}

/// Generate or update the changelog.
fn generate_changelog(
    project_root: &Utf8Path,
    version: &Version,
    tool: ChangelogTool,
) -> BumpResult<()> {
    match tool {
        ChangelogTool::GitCliff => {
            debug!("generating changelog via git-cliff");
            let output = Command::new("git-cliff")
                .args(["--output", "CHANGELOG.md", "--tag"])
                .arg(format!("v{version}"))
                .current_dir(project_root.as_std_path())
                .output()
                .map_err(|e| BumpError::ToolFailed {
                    tool: "git-cliff".into(),
                    message: format!("failed to execute: {e}"),
                })?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                return Err(BumpError::ToolFailed {
                    tool: "git-cliff".into(),
                    message: stderr,
                });
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecosystem::{ChangelogTool, DetectedTools};

    /// Build a minimal Rust ProjectDetection for testing.
    fn rust_detection() -> ProjectDetection {
        ProjectDetection {
            ecosystem: Ecosystem::Rust,
            version_strategy: VersionStrategy::Interactive,
            tools: DetectedTools {
                test_cmd: "cargo test".into(),
                build_cmd: "cargo build --release".into(),
                publish_cmd: Some("cargo publish".into()),
                bump_cmd: Some("cargo set-version".into()),
                changelog_tool: None,
            },
        }
    }

    /// Build a minimal Generic ProjectDetection for testing.
    fn generic_detection() -> ProjectDetection {
        ProjectDetection::generic(VersionStrategy::Interactive)
    }

    // ── resolve_interactive ─────────────────────────────────

    #[test]
    fn resolve_interactive_with_current_version() {
        let context = interactive::InteractiveContext {
            current_version: Some(Version::new(1, 2, 3)),
            recent_commits: vec![("abc1234".into(), "feat: stuff".into())],
            candidates: vec![],
        };
        let plan = InteractiveBump {
            context,
            detection: generic_detection(),
            version_files: vec![],
        };

        let ready = resolve_interactive(plan, Version::new(2, 0, 0));
        assert_eq!(ready.previous, Version::new(1, 2, 3));
        assert_eq!(ready.next, Version::new(2, 0, 0));
        assert_eq!(ready.strategy, VersionStrategy::Interactive);
        assert_eq!(ready.detection.ecosystem, Ecosystem::Generic);
    }

    #[test]
    fn resolve_interactive_without_current_version_defaults_to_zero() {
        let context = interactive::InteractiveContext {
            current_version: None,
            recent_commits: vec![],
            candidates: vec![],
        };
        let plan = InteractiveBump {
            context,
            detection: generic_detection(),
            version_files: vec![],
        };

        let ready = resolve_interactive(plan, Version::new(0, 1, 0));
        assert_eq!(ready.previous, Version::new(0, 0, 0));
        assert_eq!(ready.next, Version::new(0, 1, 0));
    }

    // ── resolve_strategy ────────────────────────────────────

    #[test]
    fn resolve_strategy_uses_config_conventional_commits() {
        let config = Config {
            version: Some(crate::config::VersionConfig {
                strategy: Some("conventional-commits".into()),
                cliff_config: None,
            }),
            ..Config::default()
        };
        let detection = ProjectDetection {
            ecosystem: Ecosystem::Rust,
            version_strategy: VersionStrategy::Interactive,
            tools: DetectedTools {
                test_cmd: String::new(),
                build_cmd: String::new(),
                publish_cmd: None,
                bump_cmd: None,
                changelog_tool: Some(ChangelogTool::GitCliff),
            },
        };

        let strategy = resolve_strategy(&config, &detection);
        // Should use detected tool (GitCliff)
        assert_eq!(
            strategy,
            VersionStrategy::ConventionalCommits {
                tool: ChangelogTool::GitCliff
            }
        );
    }

    #[test]
    fn resolve_strategy_cc_defaults_to_git_cliff_when_no_tool_detected() {
        let config = Config {
            version: Some(crate::config::VersionConfig {
                strategy: Some("conventional-commits".into()),
                cliff_config: None,
            }),
            ..Config::default()
        };
        let detection = ProjectDetection {
            ecosystem: Ecosystem::Rust,
            version_strategy: VersionStrategy::Interactive,
            tools: DetectedTools {
                test_cmd: String::new(),
                build_cmd: String::new(),
                publish_cmd: None,
                bump_cmd: None,
                changelog_tool: None,
            },
        };

        let strategy = resolve_strategy(&config, &detection);
        assert_eq!(
            strategy,
            VersionStrategy::ConventionalCommits {
                tool: ChangelogTool::GitCliff
            }
        );
    }

    #[test]
    fn resolve_strategy_uses_config_interactive() {
        let config = Config {
            version: Some(crate::config::VersionConfig {
                strategy: Some("interactive".into()),
                cliff_config: None,
            }),
            ..Config::default()
        };
        let detection = rust_detection();

        let strategy = resolve_strategy(&config, &detection);
        assert_eq!(strategy, VersionStrategy::Interactive);
    }

    #[test]
    fn resolve_strategy_falls_through_on_unknown_string() {
        let config = Config {
            version: Some(crate::config::VersionConfig {
                strategy: Some("unknown-thing".into()),
                cliff_config: None,
            }),
            ..Config::default()
        };
        let detection = rust_detection();

        let strategy = resolve_strategy(&config, &detection);
        // Should fall through to the detection's strategy
        assert_eq!(strategy, detection.version_strategy);
    }

    #[test]
    fn resolve_strategy_no_config_version_uses_detection() {
        let config = Config::default();
        let detection = ProjectDetection {
            ecosystem: Ecosystem::Rust,
            version_strategy: VersionStrategy::ConventionalCommits {
                tool: ChangelogTool::GitCliff,
            },
            tools: DetectedTools {
                test_cmd: String::new(),
                build_cmd: String::new(),
                publish_cmd: None,
                bump_cmd: None,
                changelog_tool: Some(ChangelogTool::GitCliff),
            },
        };

        let strategy = resolve_strategy(&config, &detection);
        assert_eq!(
            strategy,
            VersionStrategy::ConventionalCommits {
                tool: ChangelogTool::GitCliff
            }
        );
    }

    #[test]
    fn resolve_strategy_config_version_without_strategy_uses_detection() {
        let config = Config {
            version: Some(crate::config::VersionConfig {
                strategy: None,
                cliff_config: None,
            }),
            ..Config::default()
        };
        let detection = rust_detection();

        let strategy = resolve_strategy(&config, &detection);
        assert_eq!(strategy, detection.version_strategy);
    }

    // ── ReadyBump::execute ──────────────────────────────────

    #[test]
    fn execute_generic_no_changelog_tool() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = camino::Utf8Path::from_path(tmp.path()).unwrap();

        let ready = ReadyBump {
            previous: Version::new(0, 0, 0),
            next: Version::new(0, 1, 0),
            strategy: VersionStrategy::Interactive,
            detection: generic_detection(),
            version_files: vec![],
        };

        let outcome = ready.execute(root, false).unwrap();
        assert_eq!(outcome.previous, Version::new(0, 0, 0));
        assert_eq!(outcome.new, Version::new(0, 1, 0));
        assert!(!outcome.changelog_updated);
        assert!(outcome.modified_files.is_empty());
    }

    #[test]
    fn execute_generic_changelog_requested_but_no_tool() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = camino::Utf8Path::from_path(tmp.path()).unwrap();

        let ready = ReadyBump {
            previous: Version::new(1, 0, 0),
            next: Version::new(1, 1, 0),
            strategy: VersionStrategy::Interactive,
            detection: generic_detection(),
            version_files: vec![],
        };

        // Changelog requested but no tool available — should succeed with no changelog
        let outcome = ready.execute(root, true).unwrap();
        assert!(!outcome.changelog_updated);
        assert!(outcome.modified_files.is_empty());
    }

    #[test]
    fn execute_node_updates_package_json_only() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = camino::Utf8Path::from_path(tmp.path()).unwrap();

        // Seed a package.json and a pre-existing package-lock.json.
        std::fs::write(
            root.join("package.json").as_std_path(),
            r#"{
  "name": "test-pkg",
  "version": "1.0.0"
}
"#,
        )
        .unwrap();
        std::fs::write(
            root.join("package-lock.json").as_std_path(),
            r#"{
  "name": "test-pkg",
  "version": "1.0.0",
  "lockfileVersion": 3,
  "packages": {
    "": { "name": "test-pkg", "version": "1.0.0" }
  }
}
"#,
        )
        .unwrap();

        let detection = ProjectDetection {
            ecosystem: Ecosystem::Node,
            version_strategy: VersionStrategy::Interactive,
            tools: DetectedTools {
                test_cmd: "npm test".into(),
                build_cmd: "npm run build".into(),
                publish_cmd: Some("npm publish".into()),
                bump_cmd: None, // direct JSON edit, no shell-out
                changelog_tool: None,
            },
        };

        let ready = ReadyBump {
            previous: Version::new(1, 0, 0),
            next: Version::new(1, 0, 1),
            strategy: VersionStrategy::Interactive,
            detection,
            version_files: vec![],
        };

        let outcome = ready.execute(root, false).unwrap();
        assert_eq!(outcome.modified_files, vec!["package.json".to_string()]);

        // package.json was bumped
        let pkg: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(root.join("package.json")).unwrap())
                .unwrap();
        assert_eq!(pkg["version"], "1.0.1");

        // package-lock.json was NOT touched — that's npm's job, not scrat's
        let lock: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(root.join("package-lock.json")).unwrap())
                .unwrap();
        assert_eq!(lock["version"], "1.0.0");
        assert_eq!(lock["packages"][""]["version"], "1.0.0");
    }

    #[test]
    fn execute_node_errors_without_version_field() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = camino::Utf8Path::from_path(tmp.path()).unwrap();
        std::fs::write(
            root.join("package.json").as_std_path(),
            r#"{"name": "no-version"}"#,
        )
        .unwrap();

        let detection = ProjectDetection {
            ecosystem: Ecosystem::Node,
            version_strategy: VersionStrategy::Interactive,
            tools: DetectedTools {
                test_cmd: "npm test".into(),
                build_cmd: "npm run build".into(),
                publish_cmd: None,
                bump_cmd: None,
                changelog_tool: None,
            },
        };
        let ready = ReadyBump {
            previous: Version::new(1, 0, 0),
            next: Version::new(1, 0, 1),
            strategy: VersionStrategy::Interactive,
            detection,
            version_files: vec![],
        };
        let err = ready.execute(root, false).unwrap_err();
        assert!(
            matches!(err, BumpError::ToolFailed { ref tool, .. } if tool == "package.json"),
            "expected package.json ToolFailed, got: {err:?}"
        );
    }

    #[test]
    fn execute_rust_no_bump_tool_returns_error() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = camino::Utf8Path::from_path(tmp.path()).unwrap();

        let detection = ProjectDetection {
            ecosystem: Ecosystem::Rust,
            version_strategy: VersionStrategy::Interactive,
            tools: DetectedTools {
                test_cmd: "cargo test".into(),
                build_cmd: "cargo build --release".into(),
                publish_cmd: None,
                bump_cmd: None, // No bump tool!
                changelog_tool: None,
            },
        };

        let ready = ReadyBump {
            previous: Version::new(0, 1, 0),
            next: Version::new(0, 2, 0),
            strategy: VersionStrategy::Interactive,
            detection,
            version_files: vec![],
        };

        let err = ready.execute(root, false).unwrap_err();
        assert!(matches!(err, BumpError::NoBumpTool));
    }

    // ── BumpOutcome serialization ───────────────────────────

    #[test]
    fn bump_outcome_serializes() {
        let outcome = BumpOutcome {
            previous: Version::new(1, 0, 0),
            new: Version::new(1, 1, 0),
            changelog_updated: true,
            modified_files: vec!["Cargo.toml".into(), "CHANGELOG.md".into()],
        };

        let json = serde_json::to_string(&outcome).unwrap();
        assert!(json.contains("\"previous\":\"1.0.0\""));
        assert!(json.contains("\"new\":\"1.1.0\""));
        assert!(json.contains("\"changelog_updated\":true"));
        assert!(json.contains("Cargo.toml"));
    }

    // ── Error display ───────────────────────────────────────

    #[test]
    fn bump_error_tool_failed_display() {
        let err = BumpError::ToolFailed {
            tool: "cargo set-version".into(),
            message: "not found".into(),
        };
        let msg = err.to_string();
        assert!(msg.contains("cargo set-version"));
        assert!(msg.contains("not found"));
    }

    #[test]
    fn bump_error_no_bump_tool_display() {
        let err = BumpError::NoBumpTool;
        assert!(err.to_string().contains("no bump tool available"));
    }

    #[test]
    fn bump_error_unsupported_ecosystem_display() {
        let err = BumpError::UnsupportedEcosystem(Ecosystem::Node);
        assert!(err.to_string().contains("node"));
    }

    #[test]
    fn bump_error_detection_display() {
        let err = BumpError::Detection("could not detect".into());
        assert!(err.to_string().contains("could not detect"));
    }

    // ── ReadyBump fields ────────────────────────────────────

    #[test]
    fn ready_bump_clone() {
        let ready = ReadyBump {
            previous: Version::new(1, 2, 3),
            next: Version::new(1, 3, 0),
            strategy: VersionStrategy::Explicit("1.3.0".into()),
            detection: generic_detection(),
            version_files: vec![],
        };

        let cloned = ready.clone();
        assert_eq!(cloned.previous, ready.previous);
        assert_eq!(cloned.next, ready.next);
        assert_eq!(cloned.strategy, ready.strategy);
    }

    // ── plan_bump ───────────────────────────────────────────

    #[test]
    fn plan_bump_explicit_version_in_detected_project() {
        // Use a tempdir with config override so detection succeeds
        let tmp = tempfile::TempDir::new().unwrap();
        let root = camino::Utf8Path::from_path(tmp.path()).unwrap();

        let config = Config {
            project: Some(crate::config::ProjectConfig {
                project_type: Some(Ecosystem::Generic),
                release_branch: None,
            }),
            ..Config::default()
        };

        let result = plan_bump(root, &config, Some("2.0.0"));
        // This calls current_or_zero which calls git. It may fail
        // outside a git repo. Inside scrat's repo it should succeed.
        if let Ok(BumpPlan::Ready(ready)) = result {
            assert_eq!(ready.next, Version::new(2, 0, 0));
            assert!(matches!(ready.strategy, VersionStrategy::Explicit(_)));
        }
    }

    #[test]
    fn plan_bump_fails_when_detection_fails() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = camino::Utf8Path::from_path(tmp.path()).unwrap();
        // No marker files, no config override -> detection fails
        let config = Config::default();

        let result = plan_bump(root, &config, None);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, BumpError::Detection(_)));
    }

    #[test]
    fn plan_bump_explicit_overrides_config_strategy() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = camino::Utf8Path::from_path(tmp.path()).unwrap();

        let config = Config {
            project: Some(crate::config::ProjectConfig {
                project_type: Some(Ecosystem::Generic),
                release_branch: None,
            }),
            version: Some(crate::config::VersionConfig {
                strategy: Some("interactive".into()),
                cliff_config: None,
            }),
            ..Config::default()
        };

        let result = plan_bump(root, &config, Some("3.0.0"));
        if let Ok(BumpPlan::Ready(ready)) = result {
            // Strategy should be Explicit, not Interactive
            assert!(matches!(ready.strategy, VersionStrategy::Explicit(_)));
            assert_eq!(ready.next, Version::new(3, 0, 0));
        }
    }

    #[test]
    fn plan_bump_explicit_with_v_prefix() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = camino::Utf8Path::from_path(tmp.path()).unwrap();

        let config = Config {
            project: Some(crate::config::ProjectConfig {
                project_type: Some(Ecosystem::Generic),
                release_branch: None,
            }),
            ..Config::default()
        };

        let result = plan_bump(root, &config, Some("v1.5.0"));
        if let Ok(BumpPlan::Ready(ready)) = result {
            assert_eq!(ready.next, Version::new(1, 5, 0));
        }
    }

    #[test]
    fn plan_bump_explicit_invalid_version() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = camino::Utf8Path::from_path(tmp.path()).unwrap();

        let config = Config {
            project: Some(crate::config::ProjectConfig {
                project_type: Some(Ecosystem::Generic),
                release_branch: None,
            }),
            ..Config::default()
        };

        let result = plan_bump(root, &config, Some("not-semver"));
        assert!(result.is_err());
    }

    // ── version_files on ReadyBump ─────────────────────────

    #[test]
    fn ready_bump_carries_version_files() {
        use crate::config::{VersionFields, VersionFileConfig, VersionFileFormat};

        let vf = vec![VersionFileConfig {
            path: "plugin.json".into(),
            format: VersionFileFormat::Json,
            fields: Some(VersionFields::Single("version".into())),
        }];

        let bump = ReadyBump {
            previous: Version::new(1, 0, 0),
            next: Version::new(2, 0, 0),
            strategy: VersionStrategy::Interactive,
            detection: generic_detection(),
            version_files: vf,
        };

        assert_eq!(bump.version_files.len(), 1);
        assert_eq!(bump.version_files[0].path, "plugin.json");
    }

    // ── ruby version line replacement ────────────────────────

    #[test]
    fn ruby_version_double_quoted() {
        let line = r#"  VERSION = "1.2.3""#;
        assert_eq!(
            replace_ruby_version_line(line, "2.0.0"),
            Some(r#"  VERSION = "2.0.0""#.to_string())
        );
    }

    #[test]
    fn ruby_version_single_quoted() {
        let line = "  VERSION = '1.2.3'";
        assert_eq!(
            replace_ruby_version_line(line, "2.0.0"),
            Some("  VERSION = '2.0.0'".to_string())
        );
    }

    #[test]
    fn ruby_version_with_freeze_suffix() {
        let line = r#"  VERSION = "1.2.3".freeze"#;
        assert_eq!(
            replace_ruby_version_line(line, "2.0.0"),
            Some(r#"  VERSION = "2.0.0".freeze"#.to_string())
        );
    }

    #[test]
    fn ruby_version_no_indent() {
        let line = r#"VERSION = "1.2.3""#;
        assert_eq!(
            replace_ruby_version_line(line, "2.0.0"),
            Some(r#"VERSION = "2.0.0""#.to_string())
        );
    }

    #[test]
    fn ruby_version_extra_whitespace() {
        let line = r#"    VERSION   =   "1.2.3""#;
        assert_eq!(
            replace_ruby_version_line(line, "2.0.0"),
            Some(r#"    VERSION   =   "2.0.0""#.to_string())
        );
    }

    #[test]
    fn ruby_version_equality_check_rejected() {
        // VERSION == "1.2.3" is a comparison, not an assignment.
        let line = r#"if VERSION == "1.2.3""#;
        assert_eq!(replace_ruby_version_line(line, "2.0.0"), None);
    }

    #[test]
    fn ruby_version_comment_rejected() {
        let line = r#"# VERSION = "1.2.3""#;
        assert_eq!(replace_ruby_version_line(line, "2.0.0"), None);
    }

    #[test]
    fn ruby_version_suffix_identifier_rejected() {
        // FOO_VERSION is a different identifier.
        let line = r#"FOO_VERSION = "1.2.3""#;
        assert_eq!(replace_ruby_version_line(line, "2.0.0"), None);
    }

    #[test]
    fn ruby_version_unrelated_line_rejected() {
        assert_eq!(replace_ruby_version_line("puts 'hello'", "2.0.0"), None);
        assert_eq!(replace_ruby_version_line("", "2.0.0"), None);
    }

    // ── gemspec version line replacement ─────────────────────

    #[test]
    fn gemspec_spec_version_literal() {
        let line = r#"  spec.version = "1.2.3""#;
        assert_eq!(
            replace_gemspec_version_line(line, "2.0.0"),
            Some(r#"  spec.version = "2.0.0""#.to_string())
        );
    }

    #[test]
    fn gemspec_short_receiver() {
        let line = r#"  s.version = "1.2.3""#;
        assert_eq!(
            replace_gemspec_version_line(line, "2.0.0"),
            Some(r#"  s.version = "2.0.0""#.to_string())
        );
    }

    #[test]
    fn gemspec_constant_reference_rejected() {
        // Don't touch constant references — version.rb is the source of truth.
        let line = r#"  spec.version = MyGem::VERSION"#;
        assert_eq!(replace_gemspec_version_line(line, "2.0.0"), None);
    }

    #[test]
    fn gemspec_other_attribute_rejected() {
        let line = r#"  spec.name = "my_gem""#;
        assert_eq!(replace_gemspec_version_line(line, "2.0.0"), None);
    }

    #[test]
    fn gemspec_versioned_attribute_rejected() {
        // `.versioned` is not `.version`.
        let line = r#"  spec.versioned = "true""#;
        assert_eq!(replace_gemspec_version_line(line, "2.0.0"), None);
    }

    // ── ruby version file integration ────────────────────────

    #[test]
    fn bump_ruby_updates_version_rb_under_lib() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = camino::Utf8Path::from_path(tmp.path()).unwrap();
        let lib_dir = root.join("lib/my_gem");
        std::fs::create_dir_all(lib_dir.as_std_path()).unwrap();
        let version_rb = lib_dir.join("version.rb");
        std::fs::write(
            version_rb.as_std_path(),
            "module MyGem\n  VERSION = \"0.1.0\"\nend\n",
        )
        .unwrap();

        let modified = bump_ruby_version(root, &Version::new(0, 2, 0)).unwrap();
        assert_eq!(modified.len(), 1);
        assert!(modified[0].ends_with("version.rb"));

        let new_content = std::fs::read_to_string(version_rb.as_std_path()).unwrap();
        assert!(new_content.contains("VERSION = \"0.2.0\""));
        assert!(!new_content.contains("0.1.0"));
    }

    #[test]
    fn bump_ruby_updates_gemspec_literal() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = camino::Utf8Path::from_path(tmp.path()).unwrap();
        let gemspec = root.join("my_gem.gemspec");
        std::fs::write(
            gemspec.as_std_path(),
            "Gem::Specification.new do |spec|\n  \
             spec.name = \"my_gem\"\n  \
             spec.version = \"0.1.0\"\nend\n",
        )
        .unwrap();

        let modified = bump_ruby_version(root, &Version::new(0, 2, 0)).unwrap();
        assert_eq!(modified.len(), 1);
        assert!(modified[0].ends_with(".gemspec"));

        let new_content = std::fs::read_to_string(gemspec.as_std_path()).unwrap();
        assert!(new_content.contains(r#"spec.version = "0.2.0""#));
    }

    #[test]
    fn bump_ruby_skips_gemspec_constant_reference() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = camino::Utf8Path::from_path(tmp.path()).unwrap();
        // version.rb is the real source of truth
        let lib_dir = root.join("lib/my_gem");
        std::fs::create_dir_all(lib_dir.as_std_path()).unwrap();
        std::fs::write(
            lib_dir.join("version.rb").as_std_path(),
            "module MyGem\n  VERSION = \"0.1.0\"\nend\n",
        )
        .unwrap();
        // gemspec uses constant reference
        std::fs::write(
            root.join("my_gem.gemspec").as_std_path(),
            "Gem::Specification.new do |spec|\n  spec.version = MyGem::VERSION\nend\n",
        )
        .unwrap();

        let modified = bump_ruby_version(root, &Version::new(0, 2, 0)).unwrap();
        assert_eq!(modified.len(), 1, "only version.rb should be modified");
        assert!(modified[0].ends_with("version.rb"));

        // Gemspec untouched
        let gemspec_content =
            std::fs::read_to_string(root.join("my_gem.gemspec").as_std_path()).unwrap();
        assert!(gemspec_content.contains("spec.version = MyGem::VERSION"));
    }

    #[test]
    fn bump_ruby_returns_empty_when_nothing_found() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = camino::Utf8Path::from_path(tmp.path()).unwrap();
        let modified = bump_ruby_version(root, &Version::new(0, 2, 0)).unwrap();
        assert!(modified.is_empty());
    }

    #[test]
    fn bump_ruby_finds_nested_version_rb() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = camino::Utf8Path::from_path(tmp.path()).unwrap();
        let lib_dir = root.join("lib/my_gem/core");
        std::fs::create_dir_all(lib_dir.as_std_path()).unwrap();
        std::fs::write(
            lib_dir.join("version.rb").as_std_path(),
            "module MyGem\n  module Core\n    VERSION = \"1.0.0\".freeze\n  end\nend\n",
        )
        .unwrap();

        let modified = bump_ruby_version(root, &Version::new(1, 1, 0)).unwrap();
        assert_eq!(modified.len(), 1);
        let content = std::fs::read_to_string(lib_dir.join("version.rb").as_std_path()).unwrap();
        assert!(content.contains(r#"VERSION = "1.1.0".freeze"#));
    }
}
