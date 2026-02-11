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
    // Detect ecosystem
    let detection = detect::detect_project(project_root);
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

    // Write template to a temp file (git-cliff --body reads a file path)
    let mut template_file = tempfile::NamedTempFile::new()
        .map_err(|e| NotesError::CliffRender(format!("failed to create temp file: {e}")))?;
    template_file
        .write_all(template_body.as_bytes())
        .map_err(|e| NotesError::CliffRender(format!("failed to write template: {e}")))?;

    let template_path = template_file.path().to_string_lossy().to_string();

    let mut child = Command::new("git-cliff")
        .args(["--from-context", "-", "--body", &template_path])
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

        // Metadata present
        assert!(obj.contains_key("metadata"));
        assert_eq!(obj["metadata"]["custom"], "value");
    }

    #[test]
    fn build_extra_with_empty_context() {
        let ctx = test_ctx();
        let extra = build_extra(&ctx);
        let obj = extra.as_object().unwrap();

        // No stats, deps, or metadata when empty
        assert!(!obj.contains_key("stats"));
        assert!(!obj.contains_key("deps"));
        assert!(!obj.contains_key("metadata"));
    }

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
        assert_eq!(release["extra"]["stats"]["files_changed"], 3);
        assert_eq!(release["extra"]["stats"]["contributors"][0]["name"], "Clay");
    }

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

    #[test]
    fn builtin_template_is_non_empty() {
        assert!(!BUILTIN_TEMPLATE.is_empty());
        assert!(BUILTIN_TEMPLATE.contains("version"));
    }

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
        assert!(obj.contains_key("metadata"));
        assert_eq!(obj["metadata"]["key"], "val");
    }
}
