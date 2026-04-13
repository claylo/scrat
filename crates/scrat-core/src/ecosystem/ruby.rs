//! Ruby ecosystem driver (`Gemfile` / `Gemfile.lock`).
//!
//! `parse_lockfile_diff` walks `Gemfile.lock` as a collect-and-merge
//! pass on 4-space-indented gem lines. `bump_version_files` walks
//! `lib/**/version.rb` files and gemspec literal assignments.
//!
//! The byte-level line parsers preserve indentation, quote style, and
//! trailing content (e.g. `.freeze`, comments) so the rewrite is
//! minimally invasive. Constant references like
//! `spec.version = MyGem::VERSION` are intentionally skipped so the
//! `version.rb` file remains the source of truth.
//!
//! When `bump_version_files` returns an empty `Vec`, that is a valid
//! state — the caller (`bump.rs::ReadyBump::execute`) enforces the
//! release-correctness rule that there must be either a recognized
//! Ruby version file or a `[[version_files]]` config entry. The
//! driver does not have visibility into the `[[version_files]]`
//! configuration.

use std::collections::HashMap;

use camino::{Utf8Path, Utf8PathBuf};
use semver::Version;
use tracing::debug;

use super::EcosystemDriver;
use crate::bump::{BumpError, BumpResult};
use crate::detect::has_binary;
use crate::ecosystem::{DetectedTools, Ecosystem, ProjectDetection, VersionStrategy};
use crate::pipeline::DepChange;
use crate::preflight::CheckResult;

/// Ruby ecosystem driver.
pub struct RubyDriver;

impl EcosystemDriver for RubyDriver {
    /// Bump Ruby project versions. Updates every `lib/**/version.rb`
    /// file that has a `VERSION = "..."` assignment, plus any top-level
    /// `*.gemspec` that contains a literal `<spec>.version = "..."` line.
    ///
    /// Returns the paths (relative to `project_root`) of files that
    /// were actually modified. Returns an empty `Vec` if no standard
    /// Ruby version files were found — the caller
    /// (`bump.rs::ReadyBump::execute`) decides whether that's an error
    /// based on the user's `[[version_files]]` configuration, which
    /// the driver cannot see.
    fn bump_version_files(
        &self,
        project_root: &Utf8Path,
        version: &Version,
        _detection: &ProjectDetection,
    ) -> BumpResult<Vec<String>> {
        let new_version = version.to_string();
        let mut modified = Vec::new();

        // 1. lib/**/version.rb — the canonical location for gem versions.
        let lib_dir = project_root.join("lib");
        if lib_dir.is_dir() {
            let pattern = format!("{lib_dir}/**/version.rb");
            let paths = glob::glob(&pattern).map_err(|e| BumpError::ToolParse {
                tool: pattern.clone(),
                source: Box::new(e),
            })?;
            for entry in paths {
                let path = entry.map_err(|e| BumpError::ToolParse {
                    tool: pattern.clone(),
                    source: Box::new(e),
                })?;
                let path = Utf8PathBuf::from_path_buf(path).map_err(|p| BumpError::ToolFailed {
                    tool: "ruby".into(),
                    message: format!("non-UTF-8 path: {}", p.display()),
                })?;
                if update_ruby_version_file(&path, &new_version)? {
                    let rel = path
                        .strip_prefix(project_root)
                        .map(Utf8Path::to_path_buf)
                        .unwrap_or_else(|_| path.clone());
                    modified.push(rel.to_string());
                }
            }
        }

        // 2. *.gemspec — only update literal `<x>.version = "..."` assignments;
        //    skip `spec.version = MyGem::VERSION` constant references.
        let read_dir =
            std::fs::read_dir(project_root.as_std_path()).map_err(|e| BumpError::ToolIo {
                tool: project_root.to_string(),
                source: e,
            })?;
        for entry in read_dir {
            let entry = entry.map_err(|e| BumpError::ToolIo {
                tool: project_root.to_string(),
                source: e,
            })?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("gemspec") {
                continue;
            }
            let path = Utf8PathBuf::from_path_buf(path).map_err(|p| BumpError::ToolFailed {
                tool: "ruby".into(),
                message: format!("non-UTF-8 gemspec path: {}", p.display()),
            })?;
            if update_gemspec_version_file(&path, &new_version)? {
                let rel = path
                    .strip_prefix(project_root)
                    .map(Utf8Path::to_path_buf)
                    .unwrap_or_else(|_| path.clone());
                modified.push(rel.to_string());
            }
        }

        debug!(files = ?modified, "ruby version bump complete");
        Ok(modified)
    }

    fn detect(
        &self,
        _project_root: &Utf8Path,
        version_strategy: VersionStrategy,
    ) -> ProjectDetection {
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

    fn check_registry_auth(&self) -> CheckResult {
        let env_vars = ["GEM_HOST_API_KEY"];
        super::check_registry_auth_impl(
            &env_vars,
            "RubyGems",
            "set GEM_HOST_API_KEY or run `gem signin`",
        )
    }

    fn parse_lockfile_diff(&self, diff: &str) -> Vec<DepChange> {
        let mut removed: HashMap<String, String> = HashMap::new();
        let mut added: HashMap<String, String> = HashMap::new();

        for line in diff.lines() {
            // `strip_prefix` gives us the content with the marker removed
            // and naturally skips context / hunk-header lines. Ruby's
            // collect-and-merge still needs an `is_remove` flag to route
            // entries into the right map, so we can't use the exact
            // state-machine pattern from `ecosystem/{rust,php,swift}.rs`.
            let (is_remove, content) = if let Some(s) = line.strip_prefix('-') {
                (true, s)
            } else if let Some(s) = line.strip_prefix('+') {
                (false, s)
            } else {
                continue;
            };

            // Skip diff headers
            if content.starts_with("++") || content.starts_with("--") {
                continue;
            }

            // Must be exactly 4 spaces indent (top-level gem, not a sub-dep at 6+)
            if !content.starts_with("    ") || content.starts_with("      ") {
                continue;
            }

            let trimmed = content.trim();

            // Parse "gem-name (1.2.3)" or "gem-name (1.2.3.alpha)"
            if let Some((name, rest)) = trimmed.split_once(" (")
                && let Some(version) = rest.strip_suffix(')')
            {
                if is_remove {
                    removed.insert(name.to_string(), version.to_string());
                } else {
                    added.insert(name.to_string(), version.to_string());
                }
            }
        }

        let mut changes: Vec<DepChange> = Vec::new();

        for (name, old_ver) in &removed {
            if let Some(new_ver) = added.get(name) {
                if old_ver != new_ver {
                    changes.push(DepChange {
                        name: name.clone(),
                        from: Some(old_ver.clone()),
                        to: Some(new_ver.clone()),
                    });
                }
            } else {
                changes.push(DepChange {
                    name: name.clone(),
                    from: Some(old_ver.clone()),
                    to: None,
                });
            }
        }

        for (name, new_ver) in &added {
            if !removed.contains_key(name) {
                changes.push(DepChange {
                    name: name.clone(),
                    from: None,
                    to: Some(new_ver.clone()),
                });
            }
        }

        changes.sort_by(|a, b| a.name.cmp(&b.name));
        changes
    }
}

// ─── Ruby version-file helpers ───────────────────────────────────
//
// Private to this module — Ruby-exclusive line parsers and
// file-rewrite helpers used by `RubyDriver::bump_version_files`.

/// Rewrite a Ruby `VERSION = "..."` assignment in-place.
/// Returns `true` if the file was modified.
fn update_ruby_version_file(path: &Utf8Path, new_version: &str) -> BumpResult<bool> {
    let content = std::fs::read_to_string(path.as_std_path()).map_err(|e| BumpError::ToolIo {
        tool: path.to_string(),
        source: e,
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
    std::fs::write(path.as_std_path(), out).map_err(|e| BumpError::ToolIo {
        tool: path.to_string(),
        source: e,
    })?;
    Ok(true)
}

/// Rewrite `<x>.version = "..."` lines in a gemspec.
///
/// Only touches literal string assignments — leaves constant references
/// like `spec.version = MyGem::VERSION` alone so the version.rb update
/// remains the source of truth.
fn update_gemspec_version_file(path: &Utf8Path, new_version: &str) -> BumpResult<bool> {
    let content = std::fs::read_to_string(path.as_std_path()).map_err(|e| BumpError::ToolIo {
        tool: path.to_string(),
        source: e,
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
    std::fs::write(path.as_std_path(), out).map_err(|e| BumpError::ToolIo {
        tool: path.to_string(),
        source: e,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_gemfile_lock_diff_update() {
        let diff = "\
-    rails (7.1.2)\n\
+    rails (7.1.3)";
        let changes = RubyDriver.parse_lockfile_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "rails");
        assert_eq!(changes[0].from.as_deref(), Some("7.1.2"));
        assert_eq!(changes[0].to.as_deref(), Some("7.1.3"));
    }

    #[test]
    fn parse_gemfile_lock_diff_added() {
        let diff = "+    new-gem (1.0.0)";
        let changes = RubyDriver.parse_lockfile_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "new-gem");
        assert_eq!(changes[0].from, None);
        assert_eq!(changes[0].to.as_deref(), Some("1.0.0"));
    }

    #[test]
    fn parse_gemfile_lock_diff_removed() {
        let diff = "-    old-gem (2.0.0)";
        let changes = RubyDriver.parse_lockfile_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].from.as_deref(), Some("2.0.0"));
        assert_eq!(changes[0].to, None);
    }

    #[test]
    fn parse_gemfile_lock_diff_ignores_subdeps() {
        // Sub-deps have 6+ spaces indent — must be ignored
        let diff = "\
-    rails (7.1.2)\n\
+    rails (7.1.3)\n\
-      actionpack (= 7.1.2)\n\
+      actionpack (= 7.1.3)";
        let changes = RubyDriver.parse_lockfile_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "rails");
    }

    #[test]
    fn parse_gemfile_lock_diff_mixed() {
        let diff = "\
-    rails (7.1.2)\n\
+    rails (7.1.3)\n\
+    new-gem (1.0.0)\n\
-    old-gem (2.0.0)";
        let changes = RubyDriver.parse_lockfile_diff(diff);
        assert_eq!(changes.len(), 3);
        assert_eq!(changes[0].name, "new-gem");
        assert_eq!(changes[1].name, "old-gem");
        assert_eq!(changes[2].name, "rails");
    }

    #[test]
    fn parse_gemfile_lock_diff_empty() {
        assert!(RubyDriver.parse_lockfile_diff("").is_empty());
    }

    #[test]
    fn parse_gemfile_lock_diff_prerelease() {
        let diff = "\
-    nokogiri (1.16.0.rc1)\n\
+    nokogiri (1.16.0)";
        let changes = RubyDriver.parse_lockfile_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].from.as_deref(), Some("1.16.0.rc1"));
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

    fn ruby_detection() -> ProjectDetection {
        ProjectDetection {
            ecosystem: Ecosystem::Ruby,
            version_strategy: VersionStrategy::Interactive,
            tools: DetectedTools {
                test_cmd: String::new(),
                build_cmd: String::new(),
                publish_cmd: None,
                bump_cmd: None,
                changelog_tool: None,
            },
        }
    }

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

        let modified = RubyDriver
            .bump_version_files(root, &Version::new(0, 2, 0), &ruby_detection())
            .unwrap();
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

        let modified = RubyDriver
            .bump_version_files(root, &Version::new(0, 2, 0), &ruby_detection())
            .unwrap();
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

        let modified = RubyDriver
            .bump_version_files(root, &Version::new(0, 2, 0), &ruby_detection())
            .unwrap();
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
        let modified = RubyDriver
            .bump_version_files(root, &Version::new(0, 2, 0), &ruby_detection())
            .unwrap();
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

        let modified = RubyDriver
            .bump_version_files(root, &Version::new(1, 1, 0), &ruby_detection())
            .unwrap();
        assert_eq!(modified.len(), 1);
        let content = std::fs::read_to_string(lib_dir.join("version.rb").as_std_path()).unwrap();
        assert!(content.contains(r#"VERSION = "1.1.0".freeze"#));
    }
}
