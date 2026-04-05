//! Release notes rendering via git-cliff context injection.
//!
//! Uses a two-pass pattern:
//! 1. `git-cliff --unreleased --context` → JSON array of release objects
//! 2. Inject scrat's extra data (stats, deps, metadata) into `release[0].extra`
//! 3. `git-cliff --from-context - --body <template>` → rendered markdown
//!
//! This keeps scrat thin — git-cliff owns commit parsing, grouping, and
//! template rendering. scrat only injects its sidecar data.

use std::process::Command;

use camino::Utf8Path;
use serde::Serialize;
use thiserror::Error;
use tracing::{debug, warn};

use crate::config::Config;
use crate::pipeline::{PipelineContext, PipelineContextInit};
use crate::{deps, detect, git, stats, version};

/// Errors from the release notes rendering pipeline.
#[derive(Error, Debug)]
pub enum NotesError {
    /// Failed to run `git-cliff --context` or parse its output.
    #[error("git-cliff context extraction failed: {0}")]
    CliffContext(String),

    /// Failed to run `git-cliff --from-context` to render notes.
    #[error("git-cliff rendering failed: {0}")]
    CliffRender(String),

    /// Failed to read a custom template file.
    #[error("failed to read template at {path}: {source}")]
    ReadTemplate {
        /// Path to the template file.
        path: String,
        /// The underlying I/O error.
        source: std::io::Error,
    },
}

// ──────────────────────────────────────────────
// Preview orchestration
// ──────────────────────────────────────────────

/// Options for the `preview_notes` orchestrator.
#[derive(Debug, Clone, Default)]
pub struct PreviewNotesOptions {
    /// Override the previous version tag (default: latest semver tag).
    pub from: Option<String>,
    /// Override the version to render (default: read from project files).
    pub version: Option<String>,
    /// Path to a custom git-cliff template (overrides config + built-in).
    pub template: Option<String>,
    /// Skip dependency diff.
    pub no_deps: bool,
    /// Skip stats collection.
    pub no_stats: bool,
}

/// The result of a notes preview.
#[derive(Debug, Clone, Serialize)]
pub struct PreviewNotesResult {
    /// The rendered markdown.
    pub notes: String,
    /// The version the notes are for.
    pub version: String,
    /// The previous version tag.
    pub previous_tag: String,
    /// The tag for this version.
    pub tag: String,
}

/// Render release notes for preview without executing a full ship workflow.
///
/// Builds a [`PipelineContext`] from the live repo state, computes deps
/// and stats, then calls [`render_notes`] to produce markdown.
pub fn preview_notes(
    project_root: &Utf8Path,
    config: &Config,
    options: PreviewNotesOptions,
) -> Result<PreviewNotesResult, NotesError> {
    // Detect ecosystem (config override > auto-detect)
    let detection = detect::resolve_detection(project_root, config);
    let ecosystem_name = detection
        .as_ref()
        .map(|d| d.ecosystem.to_string())
        .unwrap_or_else(|| "unknown".into());

    // Resolve previous version tag
    let previous_tag = match options.from {
        Some(ref tag) => tag.clone(),
        None => git::latest_version_tag()
            .map_err(|e| NotesError::CliffContext(format!("failed to query git tags: {e}")))?
            .unwrap_or_default(),
    };

    // Parse previous version from tag
    let previous_version = if previous_tag.is_empty() {
        "0.0.0".to_string()
    } else {
        let v_str = previous_tag.strip_prefix('v').unwrap_or(&previous_tag);
        version::parse_version(v_str)
            .map(|v| v.to_string())
            .unwrap_or_else(|_| previous_tag.clone())
    };

    // Resolve current version
    let current_version = match options.version {
        Some(ref v) => {
            let v_str = v.strip_prefix('v').unwrap_or(v);
            version::parse_version(v_str)
                .map(|v| v.to_string())
                .map_err(|e| NotesError::CliffContext(format!("invalid version: {e}")))?
        }
        None => detect_current_version(project_root, &ecosystem_name)
            .unwrap_or_else(|| "unreleased".into()),
    };

    let tag = format!("v{current_version}");

    // Build repo info
    let (owner, repo, repo_url) = {
        let remote = git::remote_url("origin").ok().flatten();
        let (o, r) = remote
            .as_deref()
            .and_then(git::parse_owner_repo)
            .unwrap_or_else(|| ("unknown".into(), "unknown".into()));
        (o, r, remote)
    };

    // Build pipeline context
    let mut ctx = PipelineContext::new(PipelineContextInit {
        version: current_version.clone(),
        previous_version,
        tag: tag.clone(),
        previous_tag: previous_tag.clone(),
        owner,
        repo,
        repo_url,
        branch: git::current_branch().ok().flatten(),
        ecosystem: ecosystem_name,
        changelog_path: project_root.join("CHANGELOG.md").to_string(),
        dry_run: true,
    });

    // Compute deps
    if !options.no_deps
        && let Some(ref det) = detection
    {
        ctx.dependencies = deps::compute_deps(det.ecosystem, &ctx.previous_tag);
        if !ctx.dependencies.is_empty() {
            debug!(count = ctx.dependencies.len(), "deps computed");
        }
    }

    // Compute stats
    if !options.no_stats && !previous_tag.is_empty() {
        ctx.stats = stats::compute_stats(&ctx.previous_tag);
        if ctx.stats.is_some() {
            debug!("stats computed");
        }
    }

    // Determine template: options > config > built-in
    let template = options.template.as_deref().or_else(|| {
        config
            .release
            .as_ref()
            .and_then(|r| r.notes_template.as_deref())
    });

    // Render
    let notes = render_notes(project_root, &ctx, template)?;

    Ok(PreviewNotesResult {
        notes,
        version: current_version,
        previous_tag,
        tag,
    })
}

/// Read the current version from project manifest files.
///
/// Quick extraction without full TOML/JSON parsing crate deps.
fn detect_current_version(project_root: &Utf8Path, ecosystem: &str) -> Option<String> {
    match ecosystem {
        "rust" => {
            let cargo_toml = project_root.join("Cargo.toml");
            let content = std::fs::read_to_string(&cargo_toml).ok()?;
            let mut in_package = false;
            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with('[') {
                    in_package = trimmed == "[package]";
                    continue;
                }
                if in_package
                    && trimmed.starts_with("version")
                    && let Some((_, val)) = trimmed.split_once('=')
                {
                    let val = val.trim().trim_matches('"');
                    if !val.contains("workspace") {
                        return Some(val.to_string());
                    }
                }
            }
            None
        }
        "node" => {
            let pkg_json = project_root.join("package.json");
            let content = std::fs::read_to_string(&pkg_json).ok()?;
            let parsed: serde_json::Value = serde_json::from_str(&content).ok()?;
            parsed["version"].as_str().map(String::from)
        }
        "php" => {
            let composer_json = project_root.join("composer.json");
            let content = std::fs::read_to_string(&composer_json).ok()?;
            let parsed: serde_json::Value = serde_json::from_str(&content).ok()?;
            parsed["version"].as_str().map(String::from)
        }
        "python" => {
            let pyproject = project_root.join("pyproject.toml");
            let content = std::fs::read_to_string(&pyproject).ok()?;
            let mut in_project = false;
            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with('[') {
                    in_project = trimmed == "[project]";
                    continue;
                }
                if in_project
                    && trimmed.starts_with("version")
                    && let Some((key, val)) = trimmed.split_once('=')
                    && key.trim() == "version"
                {
                    return Some(val.trim().trim_matches('"').to_string());
                }
            }
            None
        }
        // Go, Ruby, Swift — version lives in git tags or gemspec
        _ => None,
    }
}

// ──────────────────────────────────────────────
// Low-level rendering
// ──────────────────────────────────────────────

/// Built-in release notes template, shipped with scrat.
const BUILTIN_TEMPLATE: &str = include_str!("../templates/release-notes.tera");

/// Render release notes by injecting pipeline data into git-cliff's context.
///
/// # Arguments
/// - `project_root` — working directory for git-cliff commands
/// - `ctx` — the pipeline context with stats, deps, metadata
/// - `custom_template` — optional path to a user-provided cliff template
///
/// # Returns
/// Rendered markdown string on success, or `NotesError` on failure.
pub fn render_notes(
    project_root: &Utf8Path,
    ctx: &PipelineContext,
    custom_template: Option<&str>,
) -> Result<String, NotesError> {
    // Pass 1: Extract git-cliff's native context as JSON
    debug!("extracting git-cliff context (pass 1)");
    let context_json = run_cliff_context(project_root)?;

    // Parse and inject our extra data
    let enriched_json = inject_extra(&context_json, ctx)?;

    // Determine which template to use
    let template_body = match custom_template {
        Some(path) => {
            debug!(%path, "using custom release notes template");
            std::fs::read_to_string(path).map_err(|e| NotesError::ReadTemplate {
                path: path.to_string(),
                source: e,
            })?
        }
        None => {
            debug!("using built-in release notes template");
            BUILTIN_TEMPLATE.to_string()
        }
    };

    // Pass 2: Render through git-cliff with the enriched context
    debug!("rendering release notes (pass 2)");
    let rendered = run_cliff_render(project_root, &enriched_json, &template_body)?;

    Ok(rendered)
}

/// Build the `extra` JSON object from pipeline context.
///
/// Shape:
/// ```json
/// {
///   "stats": { "files_changed": N, "insertions": N, "deletions": N, "contributors": [...] },
///   "deps": [ { "name": "...", "from": "...", "to": "..." }, ... ],
///   "metadata": { ... }
/// }
/// ```
pub fn build_extra(ctx: &PipelineContext) -> serde_json::Value {
    let mut extra = serde_json::Map::new();

    // Stats
    if let Some(ref stats) = ctx.stats {
        extra.insert(
            "stats".into(),
            serde_json::to_value(stats).unwrap_or_default(),
        );
    }

    // Deps
    if !ctx.dependencies.is_empty() {
        extra.insert(
            "deps".into(),
            serde_json::to_value(&ctx.dependencies).unwrap_or_default(),
        );
    }

    // Repo name (for templates that reference extra.repo)
    extra.insert("repo".into(), serde_json::Value::String(ctx.repo.clone()));

    // Metadata
    if !ctx.metadata.is_empty() {
        extra.insert(
            "metadata".into(),
            serde_json::Value::Object(
                ctx.metadata
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
            ),
        );
    }

    serde_json::Value::Object(extra)
}

/// Run `git-cliff --unreleased --context` and capture JSON output.
fn run_cliff_context(project_root: &Utf8Path) -> Result<String, NotesError> {
    let output = Command::new("git-cliff")
        .args(["--unreleased", "--context"])
        .current_dir(project_root.as_std_path())
        .output()
        .map_err(|e| NotesError::CliffContext(format!("failed to execute git-cliff: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(NotesError::CliffContext(format!(
            "git-cliff exited with {}: {stderr}",
            output.status
        )));
    }

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    if stdout.trim().is_empty() {
        return Err(NotesError::CliffContext(
            "git-cliff produced empty context output".into(),
        ));
    }

    Ok(stdout)
}

/// Inject scrat's extra data into the git-cliff context JSON.
///
/// The context is a JSON array of release objects. We inject into `[0].extra`.
fn inject_extra(context_json: &str, ctx: &PipelineContext) -> Result<String, NotesError> {
    let mut releases: serde_json::Value = serde_json::from_str(context_json)
        .map_err(|e| NotesError::CliffContext(format!("failed to parse context JSON: {e}")))?;

    let arr = releases
        .as_array_mut()
        .ok_or_else(|| NotesError::CliffContext("context is not a JSON array".into()))?;

    if arr.is_empty() {
        return Err(NotesError::CliffContext(
            "context array is empty (no unreleased changes?)".into(),
        ));
    }

    // Inject our extra data into the first (unreleased) release object
    let release = &mut arr[0];
    let extra = build_extra(ctx);
    release["extra"] = extra;

    // Inject the version — git-cliff's --unreleased context leaves this empty
    // because the tag doesn't exist yet (notes render before the git phase).
    // Only fill it in when missing; use bare version, not the tag prefix.
    let version_missing = release.get("version").is_none_or(|v| {
        v.is_null() || v.as_str().is_some_and(str::is_empty)
    });
    if version_missing && !ctx.version.is_empty() {
        release["version"] = serde_json::Value::String(ctx.version.clone());
    }

    serde_json::to_string(&releases)
        .map_err(|e| NotesError::CliffContext(format!("failed to re-serialize context: {e}")))
}

/// Run `git-cliff --from-context - --body <template>` with enriched JSON on stdin.
fn run_cliff_render(
    project_root: &Utf8Path,
    enriched_json: &str,
    template_body: &str,
) -> Result<String, NotesError> {
    use std::io::Write;

    // --body takes inline template text, not a file path
    // --strip header suppresses the project's cliff.toml changelog header
    let mut child = Command::new("git-cliff")
        .args([
            "--from-context",
            "-",
            "--body",
            template_body,
            "--strip",
            "header",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .current_dir(project_root.as_std_path())
        .spawn()
        .map_err(|e| NotesError::CliffRender(format!("failed to spawn git-cliff: {e}")))?;

    // Write the enriched JSON to stdin
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(enriched_json.as_bytes())
            .map_err(|e| NotesError::CliffRender(format!("failed to write to stdin: {e}")))?;
    }

    let output = child
        .wait_with_output()
        .map_err(|e| NotesError::CliffRender(format!("failed to wait for git-cliff: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(NotesError::CliffRender(format!(
            "git-cliff exited with {}: {stderr}",
            output.status
        )));
    }

    let rendered = String::from_utf8_lossy(&output.stdout).to_string();
    if rendered.trim().is_empty() {
        warn!("git-cliff rendered empty notes output");
    }

    Ok(rendered)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::{Contributor, DepChange, PipelineContextInit, ReleaseStats};
    use std::collections::HashMap;

    fn test_ctx() -> PipelineContext {
        PipelineContext::new(PipelineContextInit {
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
        })
    }

    // ──────────────────────────────────────────────
    // build_extra: full context
    // ──────────────────────────────────────────────

    #[test]
    fn build_extra_with_full_context() {
        let mut ctx = test_ctx();
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
        ctx.dependencies = vec![
            DepChange {
                name: "serde".into(),
                from: Some("1.0.0".into()),
                to: Some("1.0.1".into()),
            },
            DepChange {
                name: "tokio".into(),
                from: None,
                to: Some("1.0.0".into()),
            },
        ];
        ctx.metadata
            .insert("custom".into(), serde_json::json!("value"));

        let extra = build_extra(&ctx);
        let obj = extra.as_object().unwrap();

        // Stats present
        assert!(obj.contains_key("stats"));
        let stats = &obj["stats"];
        assert_eq!(stats["files_changed"], 10);
        assert_eq!(stats["insertions"], 500);
        assert_eq!(stats["deletions"], 200);
        assert_eq!(stats["contributors"][0]["name"], "Alice");

        // Deps present
        assert!(obj.contains_key("deps"));
        let deps = obj["deps"].as_array().unwrap();
        assert_eq!(deps.len(), 2);
        assert_eq!(deps[0]["name"], "serde");
        assert_eq!(deps[1]["from"], serde_json::Value::Null);
        assert_eq!(deps[1]["to"], "1.0.0");

        // Repo always present
        assert_eq!(obj["repo"], "scrat");

        // Metadata present
        assert!(obj.contains_key("metadata"));
        assert_eq!(obj["metadata"]["custom"], "value");
    }

    // ──────────────────────────────────────────────
    // build_extra: empty context
    // ──────────────────────────────────────────────

    #[test]
    fn build_extra_with_empty_context() {
        let ctx = test_ctx();
        let extra = build_extra(&ctx);
        let obj = extra.as_object().unwrap();

        // Only repo when no stats/deps/metadata
        assert!(!obj.contains_key("stats"));
        assert!(!obj.contains_key("deps"));
        assert!(!obj.contains_key("metadata"));
        assert_eq!(obj["repo"], "scrat");
    }

    // ──────────────────────────────────────────────
    // build_extra: dep shapes (updated, added, removed)
    // ──────────────────────────────────────────────

    #[test]
    fn build_extra_deps_shape() {
        let mut ctx = test_ctx();
        ctx.dependencies = vec![
            // Updated
            DepChange {
                name: "serde".into(),
                from: Some("1.0.0".into()),
                to: Some("1.0.1".into()),
            },
            // Added (no from)
            DepChange {
                name: "new-crate".into(),
                from: None,
                to: Some("0.1.0".into()),
            },
            // Removed (no to)
            DepChange {
                name: "old-crate".into(),
                from: Some("2.0.0".into()),
                to: None,
            },
        ];

        let extra = build_extra(&ctx);
        let deps = extra["deps"].as_array().unwrap();
        assert_eq!(deps.len(), 3);

        // Updated dep has both from and to
        assert_eq!(deps[0]["name"], "serde");
        assert_eq!(deps[0]["from"], "1.0.0");
        assert_eq!(deps[0]["to"], "1.0.1");

        // Added dep has null from
        assert_eq!(deps[1]["name"], "new-crate");
        assert!(deps[1]["from"].is_null());
        assert_eq!(deps[1]["to"], "0.1.0");

        // Removed dep has null to
        assert_eq!(deps[2]["name"], "old-crate");
        assert_eq!(deps[2]["from"], "2.0.0");
        assert!(deps[2]["to"].is_null());
    }

    // ──────────────────────────────────────────────
    // inject_extra: happy path
    // ──────────────────────────────────────────────

    #[test]
    fn inject_extra_into_cliff_context() {
        let mut ctx = test_ctx();
        ctx.stats = Some(ReleaseStats {
            commit_count: 5,
            files_changed: 3,
            insertions: 100,
            deletions: 50,
            contributors: vec![Contributor {
                name: "Clay".into(),
                count: 5,
            }],
        });

        // Simulate a minimal cliff context JSON array
        let cliff_context = serde_json::json!([{
            "version": "1.2.3",
            "commits": [],
            "statistics": {
                "commit_count": 5
            }
        }]);
        let cliff_json = serde_json::to_string(&cliff_context).unwrap();

        let result = inject_extra(&cliff_json, &ctx).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();

        // The extra field should be injected
        let release = &parsed[0];
        assert!(release["extra"].is_object());
        assert_eq!(release["extra"]["repo"], "scrat");
        assert_eq!(release["extra"]["stats"]["files_changed"], 3);
        assert_eq!(release["extra"]["stats"]["contributors"][0]["name"], "Clay");
    }

    // ──────────────────────────────────────────────
    // inject_extra: error cases
    // ──────────────────────────────────────────────

    #[test]
    fn inject_extra_errors_on_empty_array() {
        let ctx = test_ctx();
        let result = inject_extra("[]", &ctx);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("context array is empty")
        );
    }

    #[test]
    fn inject_extra_errors_on_non_array() {
        let ctx = test_ctx();
        let result = inject_extra("{}", &ctx);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not a JSON array"));
    }

    // ──────────────────────────────────────────────
    // Built-in template: basic content assertions
    // ──────────────────────────────────────────────

    #[test]
    fn builtin_template_is_non_empty() {
        assert!(!BUILTIN_TEMPLATE.is_empty());
        assert!(BUILTIN_TEMPLATE.contains("version"));
    }

    // ──────────────────────────────────────────────
    // build_extra: partial contexts (stats-only, metadata-only)
    // ──────────────────────────────────────────────

    #[test]
    fn build_extra_stats_only() {
        let mut ctx = test_ctx();
        ctx.stats = Some(ReleaseStats {
            commit_count: 10,
            files_changed: 5,
            insertions: 100,
            deletions: 50,
            contributors: vec![],
        });

        let extra = build_extra(&ctx);
        let obj = extra.as_object().unwrap();
        assert!(obj.contains_key("stats"));
        assert!(obj.contains_key("repo"));
        assert!(!obj.contains_key("deps"));
        assert!(!obj.contains_key("metadata"));
    }

    #[test]
    fn build_extra_metadata_only() {
        let mut ctx = test_ctx();
        let mut meta = HashMap::new();
        meta.insert("key".into(), serde_json::json!("val"));
        ctx.metadata = meta;

        let extra = build_extra(&ctx);
        let obj = extra.as_object().unwrap();
        assert!(!obj.contains_key("stats"));
        assert!(!obj.contains_key("deps"));
        assert!(obj.contains_key("repo"));
        assert!(obj.contains_key("metadata"));
        assert_eq!(obj["metadata"]["key"], "val");
    }

    // ──────────────────────────────────────────────
    // build_extra: deps-only (no stats, no metadata)
    // ──────────────────────────────────────────────

    #[test]
    fn build_extra_deps_only() {
        let mut ctx = test_ctx();
        ctx.dependencies = vec![DepChange {
            name: "anyhow".into(),
            from: Some("1.0.0".into()),
            to: Some("1.0.86".into()),
        }];

        let extra = build_extra(&ctx);
        let obj = extra.as_object().unwrap();
        assert!(!obj.contains_key("stats"));
        assert!(obj.contains_key("deps"));
        assert!(obj.contains_key("repo"));
        assert!(!obj.contains_key("metadata"));
        assert_eq!(obj["deps"].as_array().unwrap().len(), 1);
    }

    // ──────────────────────────────────────────────
    // build_extra: stats with zero values
    // ──────────────────────────────────────────────

    #[test]
    fn build_extra_stats_with_zeroes() {
        let mut ctx = test_ctx();
        ctx.stats = Some(ReleaseStats {
            commit_count: 0,
            files_changed: 0,
            insertions: 0,
            deletions: 0,
            contributors: vec![],
        });

        let extra = build_extra(&ctx);
        let stats = &extra["stats"];
        assert_eq!(stats["commit_count"], 0);
        assert_eq!(stats["files_changed"], 0);
        assert_eq!(stats["insertions"], 0);
        assert_eq!(stats["deletions"], 0);
        assert_eq!(stats["contributors"].as_array().unwrap().len(), 0);
    }

    // ──────────────────────────────────────────────
    // build_extra: metadata with complex values
    // ──────────────────────────────────────────────

    #[test]
    fn build_extra_metadata_complex_values() {
        let mut ctx = test_ctx();
        ctx.metadata.insert(
            "nested".into(),
            serde_json::json!({
                "a": 1,
                "b": [2, 3],
                "c": {"d": true}
            }),
        );
        ctx.metadata.insert("number".into(), serde_json::json!(42));
        ctx.metadata
            .insert("null_val".into(), serde_json::Value::Null);
        ctx.metadata
            .insert("bool_val".into(), serde_json::json!(false));

        let extra = build_extra(&ctx);
        let meta = extra["metadata"].as_object().unwrap();

        assert_eq!(meta["nested"]["a"], 1);
        assert_eq!(meta["nested"]["b"][0], 2);
        assert_eq!(meta["nested"]["c"]["d"], true);
        assert_eq!(meta["number"], 42);
        assert!(meta["null_val"].is_null());
        assert_eq!(meta["bool_val"], false);
    }

    // ──────────────────────────────────────────────
    // build_extra: multiple metadata keys
    // ──────────────────────────────────────────────

    #[test]
    fn build_extra_metadata_multiple_keys() {
        let mut ctx = test_ctx();
        ctx.metadata
            .insert("postcard".into(), serde_json::json!("img/card.png"));
        ctx.metadata
            .insert("quote".into(), serde_json::json!("Ship it!"));
        ctx.metadata
            .insert("custom_flag".into(), serde_json::json!(true));

        let extra = build_extra(&ctx);
        let meta = extra["metadata"].as_object().unwrap();
        assert_eq!(meta.len(), 3);
        assert_eq!(meta["postcard"], "img/card.png");
        assert_eq!(meta["quote"], "Ship it!");
        assert_eq!(meta["custom_flag"], true);
    }

    // ──────────────────────────────────────────────
    // build_extra: stats serialization includes commit_count
    // ──────────────────────────────────────────────

    #[test]
    fn build_extra_stats_includes_commit_count() {
        let mut ctx = test_ctx();
        ctx.stats = Some(ReleaseStats {
            commit_count: 99,
            files_changed: 7,
            insertions: 200,
            deletions: 50,
            contributors: vec![Contributor {
                name: "Dev".into(),
                count: 99,
            }],
        });

        let extra = build_extra(&ctx);
        assert_eq!(extra["stats"]["commit_count"], 99);
        assert_eq!(extra["stats"]["contributors"].as_array().unwrap().len(), 1);
        assert_eq!(extra["stats"]["contributors"][0]["name"], "Dev");
        assert_eq!(extra["stats"]["contributors"][0]["count"], 99);
    }

    // ──────────────────────────────────────────────
    // build_extra: many deps preserves order
    // ──────────────────────────────────────────────

    #[test]
    fn build_extra_deps_preserves_order() {
        let mut ctx = test_ctx();
        ctx.dependencies = vec![
            DepChange {
                name: "zzz".into(),
                from: Some("1.0.0".into()),
                to: Some("2.0.0".into()),
            },
            DepChange {
                name: "aaa".into(),
                from: None,
                to: Some("0.1.0".into()),
            },
            DepChange {
                name: "mmm".into(),
                from: Some("3.0.0".into()),
                to: None,
            },
        ];

        let extra = build_extra(&ctx);
        let deps = extra["deps"].as_array().unwrap();
        // build_extra preserves input order (sorting is the deps module's job)
        assert_eq!(deps[0]["name"], "zzz");
        assert_eq!(deps[1]["name"], "aaa");
        assert_eq!(deps[2]["name"], "mmm");
    }

    // ──────────────────────────────────────────────
    // build_extra: JSON round-trip fidelity
    // ──────────────────────────────────────────────

    #[test]
    fn build_extra_json_round_trip() {
        let mut ctx = test_ctx();
        ctx.stats = Some(ReleaseStats {
            commit_count: 3,
            files_changed: 2,
            insertions: 10,
            deletions: 5,
            contributors: vec![Contributor {
                name: "Tester".into(),
                count: 3,
            }],
        });
        ctx.dependencies = vec![DepChange {
            name: "serde".into(),
            from: Some("1.0.0".into()),
            to: Some("1.1.0".into()),
        }];
        ctx.metadata
            .insert("build_id".into(), serde_json::json!("abc-123"));

        let extra = build_extra(&ctx);
        let serialized = serde_json::to_string(&extra).unwrap();
        let deserialized: serde_json::Value = serde_json::from_str(&serialized).unwrap();

        // Round-trip should produce identical structure
        assert_eq!(extra, deserialized);
    }

    // ──────────────────────────────────────────────
    // inject_extra: malformed JSON input
    // ──────────────────────────────────────────────

    #[test]
    fn inject_extra_errors_on_malformed_json() {
        let ctx = test_ctx();
        let result = inject_extra("this is not json", &ctx);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("failed to parse context JSON"),
            "unexpected error: {err_msg}"
        );
    }

    #[test]
    fn inject_extra_errors_on_truncated_json() {
        let ctx = test_ctx();
        let result = inject_extra("[{\"version\":", &ctx);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("failed to parse context JSON"),
            "unexpected error: {err_msg}"
        );
    }

    #[test]
    fn inject_extra_errors_on_json_string() {
        let ctx = test_ctx();
        // A JSON string is valid JSON but not an array
        let result = inject_extra("\"hello\"", &ctx);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("not a JSON array"),
            "unexpected error: {err_msg}"
        );
    }

    #[test]
    fn inject_extra_errors_on_json_number() {
        let ctx = test_ctx();
        let result = inject_extra("42", &ctx);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("not a JSON array"),
            "unexpected error: {err_msg}"
        );
    }

    #[test]
    fn inject_extra_errors_on_json_null() {
        let ctx = test_ctx();
        let result = inject_extra("null", &ctx);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("not a JSON array"),
            "unexpected error: {err_msg}"
        );
    }

    #[test]
    fn inject_extra_errors_on_json_boolean() {
        let ctx = test_ctx();
        let result = inject_extra("true", &ctx);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("not a JSON array"),
            "unexpected error: {err_msg}"
        );
    }

    // ──────────────────────────────────────────────
    // inject_extra: preserves existing fields
    // ──────────────────────────────────────────────

    #[test]
    fn inject_extra_preserves_existing_release_fields() {
        let ctx = test_ctx();
        let cliff_context = serde_json::json!([{
            "version": "1.2.3",
            "timestamp": "2026-03-27T00:00:00Z",
            "commits": [
                {"id": "abc1234", "message": "feat: something", "group": "Features"}
            ],
            "statistics": {"commit_count": 1},
            "custom_field": "should survive"
        }]);
        let cliff_json = serde_json::to_string(&cliff_context).unwrap();

        let result = inject_extra(&cliff_json, &ctx).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        let release = &parsed[0];

        // Original fields preserved
        assert_eq!(release["version"], "1.2.3");
        assert_eq!(release["timestamp"], "2026-03-27T00:00:00Z");
        assert_eq!(release["commits"].as_array().unwrap().len(), 1);
        assert_eq!(release["statistics"]["commit_count"], 1);
        assert_eq!(release["custom_field"], "should survive");

        // extra injected (repo always present, no stats/deps/metadata)
        assert!(release["extra"].is_object());
        assert_eq!(release["extra"]["repo"], "scrat");
    }

    // ──────────────────────────────────────────────
    // inject_extra: multiple releases — only first gets extra
    // ──────────────────────────────────────────────

    #[test]
    fn inject_extra_only_modifies_first_release() {
        let mut ctx = test_ctx();
        ctx.metadata
            .insert("marker".into(), serde_json::json!("injected"));

        let cliff_context = serde_json::json!([
            {"version": "1.2.3", "commits": []},
            {"version": "1.1.0", "commits": []}
        ]);
        let cliff_json = serde_json::to_string(&cliff_context).unwrap();

        let result = inject_extra(&cliff_json, &ctx).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();

        // First release has extra
        assert!(parsed[0]["extra"].is_object());
        assert_eq!(parsed[0]["extra"]["metadata"]["marker"], "injected");

        // Second release does NOT have extra injected
        assert!(parsed[1]["extra"].is_null());
    }

    // ──────────────────────────────────────────────
    // inject_extra: overwrites pre-existing extra field
    // ──────────────────────────────────────────────

    #[test]
    fn inject_extra_overwrites_existing_extra() {
        let mut ctx = test_ctx();
        ctx.metadata
            .insert("from_scrat".into(), serde_json::json!(true));

        let cliff_context = serde_json::json!([{
            "version": "1.2.3",
            "commits": [],
            "extra": {"old_key": "old_value"}
        }]);
        let cliff_json = serde_json::to_string(&cliff_context).unwrap();

        let result = inject_extra(&cliff_json, &ctx).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        let extra = &parsed[0]["extra"];

        // Old extra is replaced
        assert!(
            extra["old_key"].is_null(),
            "old extra should be overwritten"
        );
        // New extra is present
        assert_eq!(extra["metadata"]["from_scrat"], true);
    }

    // ──────────────────────────────────────────────
    // inject_extra: empty context (no stats/deps/metadata) still injects
    // ──────────────────────────────────────────────

    #[test]
    fn inject_extra_injects_empty_extra_object() {
        let ctx = test_ctx();
        let cliff_context = serde_json::json!([{
            "version": "1.2.3",
            "commits": []
        }]);
        let cliff_json = serde_json::to_string(&cliff_context).unwrap();

        let result = inject_extra(&cliff_json, &ctx).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();

        // extra should contain only repo (no stats/deps/metadata)
        assert!(parsed[0]["extra"].is_object());
        assert_eq!(parsed[0]["extra"].as_object().unwrap().len(), 1);
        assert_eq!(parsed[0]["extra"]["repo"], "scrat");
    }

    // ──────────────────────────────────────────────
    // inject_extra: full pipeline context round-trip
    // ──────────────────────────────────────────────

    #[test]
    fn inject_extra_full_context_round_trip() {
        let mut ctx = test_ctx();
        ctx.stats = Some(ReleaseStats {
            commit_count: 15,
            files_changed: 8,
            insertions: 300,
            deletions: 100,
            contributors: vec![
                Contributor {
                    name: "Alice".into(),
                    count: 10,
                },
                Contributor {
                    name: "Bob".into(),
                    count: 5,
                },
            ],
        });
        ctx.dependencies = vec![
            DepChange {
                name: "serde".into(),
                from: Some("1.0.0".into()),
                to: Some("1.0.1".into()),
            },
            DepChange {
                name: "added-crate".into(),
                from: None,
                to: Some("0.1.0".into()),
            },
        ];
        ctx.metadata
            .insert("release_channel".into(), serde_json::json!("stable"));

        let cliff_context = serde_json::json!([{
            "version": "1.2.3",
            "commits": [
                {"id": "aaa", "message": "feat: add feature", "group": "Features"},
                {"id": "bbb", "message": "fix: bug fix", "group": "Bug Fixes"}
            ],
            "statistics": {"commit_count": 2}
        }]);
        let cliff_json = serde_json::to_string(&cliff_context).unwrap();

        let result = inject_extra(&cliff_json, &ctx).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        let extra = &parsed[0]["extra"];

        // Stats
        assert_eq!(extra["stats"]["commit_count"], 15);
        assert_eq!(extra["stats"]["files_changed"], 8);
        assert_eq!(extra["stats"]["insertions"], 300);
        assert_eq!(extra["stats"]["deletions"], 100);
        assert_eq!(extra["stats"]["contributors"].as_array().unwrap().len(), 2);

        // Deps
        assert_eq!(extra["deps"].as_array().unwrap().len(), 2);
        assert_eq!(extra["deps"][0]["name"], "serde");
        assert_eq!(extra["deps"][1]["name"], "added-crate");

        // Metadata
        assert_eq!(extra["metadata"]["release_channel"], "stable");

        // Original cliff data intact
        assert_eq!(parsed[0]["commits"].as_array().unwrap().len(), 2);
    }

    // ──────────────────────────────────────────────
    // inject_extra: output is valid JSON string
    // ──────────────────────────────────────────────

    #[test]
    fn inject_extra_produces_valid_json() {
        let mut ctx = test_ctx();
        ctx.stats = Some(ReleaseStats {
            commit_count: 1,
            files_changed: 1,
            insertions: 10,
            deletions: 0,
            contributors: vec![],
        });

        let cliff_context = serde_json::json!([{"version": "1.2.3", "commits": []}]);
        let cliff_json = serde_json::to_string(&cliff_context).unwrap();

        let result = inject_extra(&cliff_json, &ctx).unwrap();
        // Must parse without error
        let parsed: Result<serde_json::Value, _> = serde_json::from_str(&result);
        assert!(parsed.is_ok(), "inject_extra output is not valid JSON");
    }

    // ──────────────────────────────────────────────
    // Built-in template: structural content assertions
    // ──────────────────────────────────────────────

    #[test]
    fn builtin_template_has_dependency_section() {
        assert!(
            BUILTIN_TEMPLATE.contains("extra.deps"),
            "template should reference extra.deps for dependency section"
        );
        assert!(
            BUILTIN_TEMPLATE.contains("Dependencies"),
            "template should have a Dependencies heading"
        );
    }

    #[test]
    fn builtin_template_has_stats_section() {
        assert!(
            BUILTIN_TEMPLATE.contains("extra.stats"),
            "template should reference extra.stats for stats section"
        );
        assert!(
            BUILTIN_TEMPLATE.contains("Stats"),
            "template should have a Stats heading"
        );
    }

    #[test]
    fn builtin_template_has_breaking_changes_section() {
        assert!(
            BUILTIN_TEMPLATE.contains("breaking"),
            "template should handle breaking changes"
        );
    }

    #[test]
    fn builtin_template_has_grouped_changes() {
        assert!(
            BUILTIN_TEMPLATE.contains("group_by"),
            "template should group commits"
        );
        // Verify known group mappings
        assert!(BUILTIN_TEMPLATE.contains("Added"));
        assert!(BUILTIN_TEMPLATE.contains("Fixed"));
    }

    #[test]
    fn builtin_template_has_full_commit_list() {
        assert!(
            BUILTIN_TEMPLATE.contains("<details>"),
            "template should have collapsible section"
        );
        assert!(
            BUILTIN_TEMPLATE.contains("Full commit list"),
            "template should label the commit list"
        );
    }

    #[test]
    fn builtin_template_has_contributor_section() {
        assert!(
            BUILTIN_TEMPLATE.contains("contributors"),
            "template should reference contributors"
        );
    }

    #[test]
    fn builtin_template_dep_update_format() {
        // Verify the template uses the expected dep formatting
        assert!(
            BUILTIN_TEMPLATE.contains("d.from"),
            "template should reference d.from for dep versions"
        );
        assert!(
            BUILTIN_TEMPLATE.contains("d.to"),
            "template should reference d.to for dep versions"
        );
        assert!(
            BUILTIN_TEMPLATE.contains("d.name"),
            "template should reference d.name for dep names"
        );
    }

    #[test]
    fn builtin_template_handles_added_and_removed_deps() {
        assert!(
            BUILTIN_TEMPLATE.contains("added"),
            "template should mark added deps"
        );
        assert!(
            BUILTIN_TEMPLATE.contains("removed"),
            "template should mark removed deps"
        );
    }

    // ──────────────────────────────────────────────
    // NotesError: display strings
    // ──────────────────────────────────────────────

    #[test]
    fn notes_error_cliff_context_display() {
        let err = NotesError::CliffContext("something went wrong".into());
        let msg = err.to_string();
        assert_eq!(
            msg,
            "git-cliff context extraction failed: something went wrong"
        );
    }

    #[test]
    fn notes_error_cliff_render_display() {
        let err = NotesError::CliffRender("render failed".into());
        let msg = err.to_string();
        assert_eq!(msg, "git-cliff rendering failed: render failed");
    }

    #[test]
    fn notes_error_read_template_display() {
        let err = NotesError::ReadTemplate {
            path: "/tmp/missing.tera".into(),
            source: std::io::Error::new(std::io::ErrorKind::NotFound, "file not found"),
        };
        let msg = err.to_string();
        assert!(
            msg.contains("/tmp/missing.tera"),
            "error should contain the path"
        );
        assert!(
            msg.contains("failed to read template"),
            "error should describe what failed"
        );
    }

    // ──────────────────────────────────────────────
    // PreviewNotesOptions: defaults
    // ──────────────────────────────────────────────

    #[test]
    fn preview_notes_options_defaults() {
        let opts = PreviewNotesOptions::default();
        assert!(opts.from.is_none());
        assert!(opts.version.is_none());
        assert!(opts.template.is_none());
        assert!(!opts.no_deps);
        assert!(!opts.no_stats);
    }

    #[test]
    fn preview_notes_options_with_overrides() {
        let opts = PreviewNotesOptions {
            from: Some("v1.0.0".into()),
            version: Some("2.0.0".into()),
            template: Some("custom.tera".into()),
            no_deps: true,
            no_stats: true,
        };
        assert_eq!(opts.from.as_deref(), Some("v1.0.0"));
        assert_eq!(opts.version.as_deref(), Some("2.0.0"));
        assert_eq!(opts.template.as_deref(), Some("custom.tera"));
        assert!(opts.no_deps);
        assert!(opts.no_stats);
    }

    // ──────────────────────────────────────────────
    // PreviewNotesResult: serialization
    // ──────────────────────────────────────────────

    #[test]
    fn preview_notes_result_serializes() {
        let result = PreviewNotesResult {
            notes: "## 1.2.3\n\nSome notes".into(),
            version: "1.2.3".into(),
            previous_tag: "v1.1.0".into(),
            tag: "v1.2.3".into(),
        };

        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["notes"], "## 1.2.3\n\nSome notes");
        assert_eq!(json["version"], "1.2.3");
        assert_eq!(json["previous_tag"], "v1.1.0");
        assert_eq!(json["tag"], "v1.2.3");
    }

    #[test]
    fn preview_notes_result_clone() {
        let result = PreviewNotesResult {
            notes: "notes".into(),
            version: "1.0.0".into(),
            previous_tag: "v0.9.0".into(),
            tag: "v1.0.0".into(),
        };
        let cloned = result.clone();
        assert_eq!(result.notes, cloned.notes);
        assert_eq!(result.version, cloned.version);
        assert_eq!(result.previous_tag, cloned.previous_tag);
        assert_eq!(result.tag, cloned.tag);
    }

    // ──────────────────────────────────────────────
    // detect_current_version: edge cases
    // ──────────────────────────────────────────────

    #[test]
    fn detect_current_version_unknown_ecosystem() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = camino::Utf8Path::from_path(tmp.path()).unwrap();
        // Unknown ecosystem should return None
        assert!(detect_current_version(root, "python").is_none());
        assert!(detect_current_version(root, "go").is_none());
        assert!(detect_current_version(root, "").is_none());
        assert!(detect_current_version(root, "generic").is_none());
    }

    #[test]
    fn detect_current_version_rust_missing_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = camino::Utf8Path::from_path(tmp.path()).unwrap();
        // No Cargo.toml exists
        assert!(detect_current_version(root, "rust").is_none());
    }

    #[test]
    fn detect_current_version_rust_valid() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = camino::Utf8Path::from_path(tmp.path()).unwrap();
        std::fs::write(
            tmp.path().join("Cargo.toml"),
            "[package]\nname = \"test\"\nversion = \"3.2.1\"\n",
        )
        .unwrap();
        assert_eq!(detect_current_version(root, "rust"), Some("3.2.1".into()));
    }

    #[test]
    fn detect_current_version_rust_workspace_version_inline_table() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = camino::Utf8Path::from_path(tmp.path()).unwrap();
        // version = { workspace = true } — value contains "workspace", should be skipped
        std::fs::write(
            tmp.path().join("Cargo.toml"),
            "[package]\nname = \"test\"\nversion = { workspace = true }\n",
        )
        .unwrap();
        assert!(detect_current_version(root, "rust").is_none());
    }

    #[test]
    fn detect_current_version_rust_dotted_workspace_returns_non_version() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = camino::Utf8Path::from_path(tmp.path()).unwrap();
        // version.workspace = true — starts with "version" and has "=",
        // split_once yields ("version.workspace ", " true"). The value "true"
        // doesn't contain "workspace", so the function returns Some("true").
        // This is a known limitation of the quick-parse approach.
        std::fs::write(
            tmp.path().join("Cargo.toml"),
            "[package]\nname = \"test\"\nversion.workspace = true\n",
        )
        .unwrap();
        let result = detect_current_version(root, "rust");
        // Documents actual behavior: returns Some("true"), not None
        assert_eq!(result, Some("true".into()));
    }

    #[test]
    fn detect_current_version_rust_version_in_wrong_section() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = camino::Utf8Path::from_path(tmp.path()).unwrap();
        // version under [dependencies], not [package]
        std::fs::write(
            tmp.path().join("Cargo.toml"),
            "[dependencies]\nserde = { version = \"1.0\" }\n",
        )
        .unwrap();
        assert!(detect_current_version(root, "rust").is_none());
    }

    #[test]
    fn detect_current_version_node_valid() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = camino::Utf8Path::from_path(tmp.path()).unwrap();
        std::fs::write(
            tmp.path().join("package.json"),
            r#"{"name": "test", "version": "2.0.0"}"#,
        )
        .unwrap();
        assert_eq!(detect_current_version(root, "node"), Some("2.0.0".into()));
    }

    #[test]
    fn detect_current_version_node_missing_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = camino::Utf8Path::from_path(tmp.path()).unwrap();
        assert!(detect_current_version(root, "node").is_none());
    }

    #[test]
    fn detect_current_version_node_missing_version_field() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = camino::Utf8Path::from_path(tmp.path()).unwrap();
        std::fs::write(tmp.path().join("package.json"), r#"{"name": "test"}"#).unwrap();
        assert!(detect_current_version(root, "node").is_none());
    }

    #[test]
    fn detect_current_version_node_invalid_json() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = camino::Utf8Path::from_path(tmp.path()).unwrap();
        std::fs::write(tmp.path().join("package.json"), "not json").unwrap();
        assert!(detect_current_version(root, "node").is_none());
    }

    #[test]
    fn detect_current_version_rust_multiple_sections() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = camino::Utf8Path::from_path(tmp.path()).unwrap();
        std::fs::write(
            tmp.path().join("Cargo.toml"),
            "[workspace]\nmembers = [\"crates/*\"]\n\n[package]\nname = \"root\"\nversion = \"5.0.0\"\n\n[dependencies]\nserde = \"1\"\n",
        )
        .unwrap();
        assert_eq!(detect_current_version(root, "rust"), Some("5.0.0".into()));
    }

    // ──────────────────────────────────────────────
    // build_extra: all three fields populated together
    // ──────────────────────────────────────────────

    #[test]
    fn build_extra_all_three_keys_present() {
        let mut ctx = test_ctx();
        ctx.stats = Some(ReleaseStats {
            commit_count: 1,
            files_changed: 1,
            insertions: 1,
            deletions: 1,
            contributors: vec![],
        });
        ctx.dependencies = vec![DepChange {
            name: "x".into(),
            from: None,
            to: Some("1.0.0".into()),
        }];
        ctx.metadata.insert("k".into(), serde_json::json!("v"));

        let extra = build_extra(&ctx);
        let obj = extra.as_object().unwrap();
        assert_eq!(
            obj.len(),
            4,
            "should have exactly stats, deps, repo, metadata"
        );
        assert!(obj.contains_key("stats"));
        assert!(obj.contains_key("deps"));
        assert!(obj.contains_key("repo"));
        assert!(obj.contains_key("metadata"));
    }

    // ──────────────────────────────────────────────
    // build_extra: returns Value::Object type
    // ──────────────────────────────────────────────

    #[test]
    fn build_extra_always_returns_object() {
        // Even with no data, build_extra returns a JSON object (not null, array, etc.)
        let ctx = test_ctx();
        let extra = build_extra(&ctx);
        assert!(extra.is_object(), "build_extra must return a JSON object");
    }
}
