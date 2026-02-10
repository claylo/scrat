//! Pipeline context — the accumulator for the ship workflow.
//!
//! Every phase in `scrat ship` contributes structured data to a
//! [`PipelineContext`]. This context serves three purposes:
//!
//! 1. **`filter:` hooks** receive it as JSON on stdin and return mutated JSON
//! 2. **Release notes templates** consume it as a Tera context
//! 3. **CLI output** includes it in the `ShipOutcome` for machine-readable results
//!
//! Version fields are `String` (not `semver::Version`) so that JSON
//! round-trips through external processes don't require semver-aware parsing.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::hooks::HookContext;

// ──────────────────────────────────────────────
// Types
// ──────────────────────────────────────────────

/// The main pipeline accumulator.
///
/// Constructed at the start of `ReadyShip::execute()` from the resolved
/// bump plan and project detection. Phase methods (`record_bump`,
/// `record_git`, `record_release`) fill in results as the pipeline runs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineContext {
    // ── Version ──
    /// The new version being released (e.g., `"1.2.3"`).
    pub version: String,
    /// The previous version (e.g., `"1.1.0"`).
    pub previous_version: String,
    /// The git tag for this release (e.g., `"v1.2.3"`).
    pub tag: String,
    /// The git tag for the previous release (e.g., `"v1.1.0"`).
    pub previous_tag: String,
    /// Release date in `YYYY-MM-DD` format.
    pub date: String,

    // ── Repository ──
    /// Repository owner (from git remote).
    pub owner: String,
    /// Repository name (from git remote).
    pub repo: String,
    /// Full repository URL, if available.
    pub repo_url: Option<String>,
    /// Current git branch.
    pub branch: Option<String>,

    // ── Project ──
    /// Detected ecosystem (e.g., `"rust"`, `"node"`).
    pub ecosystem: String,

    // ── Stats (populated by M4 #3) ──
    /// Release statistics (files changed, insertions, deletions, etc.).
    pub stats: Option<ReleaseStats>,

    // ── Deps (populated by M4 #2) ──
    /// Dependency changes between previous and current version.
    pub dependencies: Vec<DepChange>,

    // ── Bump results ──
    /// Whether the changelog was updated during the bump phase.
    pub changelog_updated: bool,
    /// Path to the changelog file.
    pub changelog_path: String,
    /// Files modified during the bump phase.
    pub modified_files: Vec<String>,

    // ── Git results ──
    /// The commit hash created by the git phase.
    pub commit_hash: Option<String>,

    // ── Release results ──
    /// URL of the created GitHub release.
    pub release_url: Option<String>,
    /// Asset paths attached to the release.
    pub assets: Vec<String>,

    // ── Extensible ──
    /// Arbitrary metadata for hooks and templates.
    pub metadata: HashMap<String, serde_json::Value>,

    // ── Control ──
    /// Whether this is a dry-run execution.
    pub dry_run: bool,
}

/// Release statistics gathered from git between two refs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseStats {
    /// Number of commits in the release.
    pub commit_count: usize,
    /// Number of files changed.
    pub files_changed: usize,
    /// Total lines inserted.
    pub insertions: usize,
    /// Total lines deleted.
    pub deletions: usize,
    /// Contributors and their commit counts.
    pub contributors: Vec<Contributor>,
}

/// A contributor to the release.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contributor {
    /// Contributor name (from git shortlog).
    pub name: String,
    /// Number of commits by this contributor.
    pub count: usize,
}

/// A dependency change between two versions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DepChange {
    /// Dependency name.
    pub name: String,
    /// Previous version (None if newly added).
    pub from: Option<String>,
    /// New version (None if removed).
    pub to: Option<String>,
}

// ──────────────────────────────────────────────
// Constructor
// ──────────────────────────────────────────────

/// Arguments for constructing a [`PipelineContext`].
///
/// Keeps the constructor call site readable without a 15-argument function.
pub struct PipelineContextInit {
    /// New version string.
    pub version: String,
    /// Previous version string.
    pub previous_version: String,
    /// Git tag for this release.
    pub tag: String,
    /// Git tag for the previous release.
    pub previous_tag: String,
    /// Repository owner.
    pub owner: String,
    /// Repository name.
    pub repo: String,
    /// Full repository URL.
    pub repo_url: Option<String>,
    /// Current git branch.
    pub branch: Option<String>,
    /// Detected ecosystem name.
    pub ecosystem: String,
    /// Path to the changelog file.
    pub changelog_path: String,
    /// Whether this is a dry run.
    pub dry_run: bool,
}

impl PipelineContext {
    /// Create a new pipeline context from resolved plan data.
    ///
    /// Phase-specific fields (`stats`, `dependencies`, `commit_hash`,
    /// `release_url`, etc.) start empty and are populated as phases run.
    pub fn new(init: PipelineContextInit) -> Self {
        Self {
            version: init.version,
            previous_version: init.previous_version,
            tag: init.tag,
            previous_tag: init.previous_tag,
            date: iso_date_today(),
            owner: init.owner,
            repo: init.repo,
            repo_url: init.repo_url,
            branch: init.branch,
            ecosystem: init.ecosystem,
            stats: None,
            dependencies: Vec::new(),
            changelog_updated: false,
            changelog_path: init.changelog_path,
            modified_files: Vec::new(),
            commit_hash: None,
            release_url: None,
            assets: Vec::new(),
            metadata: HashMap::new(),
            dry_run: init.dry_run,
        }
    }

    /// Derive a [`HookContext`] for variable interpolation in hook commands.
    pub fn hook_context(&self) -> HookContext {
        HookContext {
            version: self.version.clone(),
            prev_version: self.previous_version.clone(),
            tag: self.tag.clone(),
            changelog_path: self.changelog_path.clone(),
            owner: self.owner.clone(),
            repo: self.repo.clone(),
        }
    }

    /// Record results from the bump phase.
    pub fn record_bump(&mut self, changelog_updated: bool, modified_files: Vec<String>) {
        self.changelog_updated = changelog_updated;
        self.modified_files = modified_files;
    }

    /// Record results from the git phase.
    pub fn record_git(&mut self, commit_hash: Option<String>, branch: Option<String>) {
        self.commit_hash = commit_hash;
        if branch.is_some() {
            self.branch = branch;
        }
    }

    /// Record results from the release phase.
    pub fn record_release(&mut self, url: Option<String>) {
        self.release_url = url;
    }

    /// Set release assets from configuration.
    pub fn set_assets(&mut self, assets: Vec<String>) {
        self.assets = assets;
    }
}

// ──────────────────────────────────────────────
// Helpers
// ──────────────────────────────────────────────

/// Return today's date as `YYYY-MM-DD` without external date crates.
///
/// Uses the civil-days-from-epoch algorithm (Howard Hinnant) to convert
/// `SystemTime::now()` into a calendar date in local-ish UTC.
pub fn iso_date_today() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    // Days since epoch (UTC)
    let days = (secs / 86400) as i64;

    // Hinnant civil_from_days algorithm
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097); // day of era [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // year of era [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // day of year [0, 365]
    let mp = (5 * doy + 2) / 153; // month index [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // day [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // month [1, 12]
    let y = if m <= 2 { y + 1 } else { y };

    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_init() -> PipelineContextInit {
        PipelineContextInit {
            version: "1.2.3".into(),
            previous_version: "1.1.0".into(),
            tag: "v1.2.3".into(),
            previous_tag: "v1.1.0".into(),
            owner: "claylo".into(),
            repo: "scrat".into(),
            repo_url: Some("https://github.com/claylo/scrat".into()),
            branch: Some("main".into()),
            ecosystem: "rust".into(),
            changelog_path: "CHANGELOG.md".into(),
            dry_run: false,
        }
    }

    #[test]
    fn new_sets_version_fields() {
        let ctx = PipelineContext::new(test_init());
        assert_eq!(ctx.version, "1.2.3");
        assert_eq!(ctx.previous_version, "1.1.0");
        assert_eq!(ctx.tag, "v1.2.3");
        assert_eq!(ctx.previous_tag, "v1.1.0");
    }

    #[test]
    fn new_computes_date() {
        let ctx = PipelineContext::new(test_init());
        // YYYY-MM-DD format: 10 chars, dashes at positions 4 and 7
        assert_eq!(ctx.date.len(), 10);
        assert_eq!(ctx.date.as_bytes()[4], b'-');
        assert_eq!(ctx.date.as_bytes()[7], b'-');
    }

    #[test]
    fn new_starts_with_empty_phase_results() {
        let ctx = PipelineContext::new(test_init());
        assert!(ctx.stats.is_none());
        assert!(ctx.dependencies.is_empty());
        assert!(!ctx.changelog_updated);
        assert!(ctx.modified_files.is_empty());
        assert!(ctx.commit_hash.is_none());
        assert!(ctx.release_url.is_none());
        assert!(ctx.assets.is_empty());
        assert!(ctx.metadata.is_empty());
    }

    #[test]
    fn hook_context_derives_correctly() {
        let ctx = PipelineContext::new(test_init());
        let hc = ctx.hook_context();
        assert_eq!(hc.version, "1.2.3");
        assert_eq!(hc.prev_version, "1.1.0");
        assert_eq!(hc.tag, "v1.2.3");
        assert_eq!(hc.changelog_path, "CHANGELOG.md");
        assert_eq!(hc.owner, "claylo");
        assert_eq!(hc.repo, "scrat");
    }

    #[test]
    fn record_bump_updates_fields() {
        let mut ctx = PipelineContext::new(test_init());
        ctx.record_bump(true, vec!["Cargo.toml".into(), "CHANGELOG.md".into()]);
        assert!(ctx.changelog_updated);
        assert_eq!(ctx.modified_files, vec!["Cargo.toml", "CHANGELOG.md"]);
    }

    #[test]
    fn record_git_updates_fields() {
        let mut ctx = PipelineContext::new(test_init());
        ctx.record_git(Some("abc1234".into()), Some("main".into()));
        assert_eq!(ctx.commit_hash.as_deref(), Some("abc1234"));
        assert_eq!(ctx.branch.as_deref(), Some("main"));
    }

    #[test]
    fn record_git_preserves_branch_when_none() {
        let mut ctx = PipelineContext::new(test_init());
        assert_eq!(ctx.branch.as_deref(), Some("main")); // from init
        ctx.record_git(Some("abc1234".into()), None);
        assert_eq!(ctx.branch.as_deref(), Some("main")); // preserved
    }

    #[test]
    fn record_release_updates_url() {
        let mut ctx = PipelineContext::new(test_init());
        ctx.record_release(Some(
            "https://github.com/claylo/scrat/releases/tag/v1.2.3".into(),
        ));
        assert_eq!(
            ctx.release_url.as_deref(),
            Some("https://github.com/claylo/scrat/releases/tag/v1.2.3")
        );
    }

    #[test]
    fn set_assets() {
        let mut ctx = PipelineContext::new(test_init());
        ctx.set_assets(vec!["dist/app.tar.gz".into(), "dist/app.deb".into()]);
        assert_eq!(ctx.assets, vec!["dist/app.tar.gz", "dist/app.deb"]);
    }

    #[test]
    fn json_round_trip() {
        let mut ctx = PipelineContext::new(test_init());
        ctx.record_bump(true, vec!["Cargo.toml".into()]);
        ctx.record_git(Some("abc1234".into()), Some("main".into()));
        ctx.metadata
            .insert("custom_key".into(), serde_json::json!("custom_value"));

        let json = serde_json::to_string(&ctx).unwrap();
        let back: PipelineContext = serde_json::from_str(&json).unwrap();

        assert_eq!(back.version, ctx.version);
        assert_eq!(back.previous_version, ctx.previous_version);
        assert_eq!(back.tag, ctx.tag);
        assert_eq!(back.previous_tag, ctx.previous_tag);
        assert_eq!(back.date, ctx.date);
        assert_eq!(back.owner, ctx.owner);
        assert_eq!(back.repo, ctx.repo);
        assert_eq!(back.ecosystem, ctx.ecosystem);
        assert!(back.changelog_updated);
        assert_eq!(back.modified_files, vec!["Cargo.toml"]);
        assert_eq!(back.commit_hash.as_deref(), Some("abc1234"));
        assert_eq!(
            back.metadata.get("custom_key").and_then(|v| v.as_str()),
            Some("custom_value")
        );
    }

    #[test]
    fn json_round_trip_with_stats() {
        let mut ctx = PipelineContext::new(test_init());
        ctx.stats = Some(ReleaseStats {
            commit_count: 42,
            files_changed: 10,
            insertions: 500,
            deletions: 200,
            contributors: vec![
                Contributor {
                    name: "Alice".into(),
                    count: 30,
                },
                Contributor {
                    name: "Bob".into(),
                    count: 12,
                },
            ],
        });

        let json = serde_json::to_string(&ctx).unwrap();
        let back: PipelineContext = serde_json::from_str(&json).unwrap();

        let stats = back.stats.unwrap();
        assert_eq!(stats.commit_count, 42);
        assert_eq!(stats.files_changed, 10);
        assert_eq!(stats.insertions, 500);
        assert_eq!(stats.deletions, 200);
        assert_eq!(stats.contributors.len(), 2);
        assert_eq!(stats.contributors[0].name, "Alice");
        assert_eq!(stats.contributors[0].count, 30);
    }

    #[test]
    fn dep_change_serializes() {
        let dep = DepChange {
            name: "serde".into(),
            from: Some("1.0.0".into()),
            to: Some("1.0.1".into()),
        };
        let json = serde_json::to_string(&dep).unwrap();
        assert!(json.contains("\"name\":\"serde\""));
        assert!(json.contains("\"from\":\"1.0.0\""));
        assert!(json.contains("\"to\":\"1.0.1\""));
    }

    #[test]
    fn iso_date_today_format() {
        let date = iso_date_today();
        assert_eq!(date.len(), 10);
        assert_eq!(date.as_bytes()[4], b'-');
        assert_eq!(date.as_bytes()[7], b'-');
        // Year should be reasonable (2020-2099)
        let year: u32 = date[..4].parse().unwrap();
        assert!(year >= 2020);
        assert!(year < 2100);
    }
}
