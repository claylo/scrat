//! Hook executor for release workflow phases.
//!
//! Hooks are user-configured shell commands that run at phase boundaries
//! during the ship workflow. Commands support variable interpolation and
//! can run in parallel or sequentially.
//!
//! # Execution model
//!
//! Commands run in parallel by default. Two prefixes alter execution:
//!
//! ## `sync:` — barrier
//!
//! 1. All prior (parallel) commands must finish
//! 2. The sync command runs alone
//! 3. Subsequent commands resume in parallel
//!
//! ## `filter:` — barrier + JSON piping
//!
//! Like `sync:`, a filter command creates a barrier and runs alone.
//! Additionally, it receives the full [`PipelineContext`](crate::pipeline::PipelineContext)
//! as JSON on stdin and must return valid JSON on stdout. The output
//! replaces the pipeline context. Multiple filters chain: each receives
//! the previous filter's output.
//!
//! Invalid JSON output is a hard error — the release aborts.
//!
//! # Variables
//!
//! Commands support `{var}` interpolation for:
//! `{version}`, `{prev_version}`, `{tag}`, `{changelog_path}`,
//! `{owner}`, `{repo}`.

use std::io::Write as _;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use camino::Utf8Path;
use thiserror::Error;
use tracing::{debug, instrument};

/// Errors from hook execution.
#[derive(Error, Debug)]
pub enum HookError {
    /// A hook command exited with a non-zero status.
    #[error("hook command failed: {command}")]
    CommandFailed {
        /// The command that failed.
        command: String,
        /// The exit code, if available.
        exit_code: Option<i32>,
        /// Captured stderr.
        stderr: String,
    },

    /// A filter hook returned invalid JSON.
    #[error("filter hook returned invalid output: {command}")]
    FilterOutputInvalid {
        /// The command that produced invalid output.
        command: String,
        /// The parse error details.
        detail: String,
    },

    /// Failed to spawn a hook command.
    #[error("failed to execute hook: {0}")]
    Exec(#[from] std::io::Error),
}

/// Result alias for hook operations.
pub type HookResult<T> = Result<T, HookError>;

/// Variables available for interpolation in hook commands.
///
/// Typically derived from [`PipelineContext::hook_context()`](crate::pipeline::PipelineContext::hook_context)
/// during the ship workflow rather than constructed directly.
#[derive(Debug, Clone)]
pub struct HookContext {
    /// The new version being released (e.g., `1.2.3`).
    pub version: String,
    /// The previous version (e.g., `1.1.0`).
    pub prev_version: String,
    /// The git tag (e.g., `v1.2.3`).
    pub tag: String,
    /// Path to the generated changelog file.
    pub changelog_path: String,
    /// Repository owner (from git remote).
    pub owner: String,
    /// Repository name (from git remote).
    pub repo: String,
}

/// Result of running a single hook command.
#[derive(Debug, Clone)]
pub struct HookOutput {
    /// The original command (before interpolation).
    pub command: String,
    /// Whether the command succeeded.
    pub success: bool,
    /// Captured stdout.
    pub stdout: String,
    /// Captured stderr.
    pub stderr: String,
    /// How long the command took to run.
    pub duration: Duration,
}

/// Output from running a list of hooks, including any filter results.
#[derive(Debug)]
pub struct RunHooksOutput {
    /// Results from each individual hook command.
    pub results: Vec<HookOutput>,
    /// If any `filter:` hooks ran, the final JSON output from the last filter.
    /// `None` if no filter hooks were present.
    pub filter_output: Option<String>,
}

/// Run a list of hook commands with variable interpolation.
///
/// Commands run in parallel by default. Prefix a command with `sync:`
/// to create a barrier — all prior commands must finish, the sync
/// command runs alone, then subsequent commands resume in parallel.
/// Prefix with `filter:` for a barrier that also pipes JSON through
/// the command via stdin/stdout.
///
/// `pipeline_json` provides the initial JSON for `filter:` hooks.
/// If `None` and a filter is encountered, `"{}"` is used as input.
///
/// Returns the results of all executed commands plus any filter output.
/// If any command in a batch fails, remaining batches are skipped and
/// an error is returned.
#[instrument(skip_all, fields(count = commands.len()))]
pub fn run_hooks(
    commands: &[String],
    context: &HookContext,
    project_root: &Utf8Path,
    pipeline_json: Option<&str>,
) -> HookResult<RunHooksOutput> {
    if commands.is_empty() {
        return Ok(RunHooksOutput {
            results: Vec::new(),
            filter_output: None,
        });
    }

    let batches = split_batches(commands);
    debug!(batch_count = batches.len(), "executing hook batches");

    let mut all_results = Vec::new();
    let mut filter_json: Option<String> = None;

    for batch in &batches {
        match batch.kind {
            BatchKind::Filter => {
                // Filter batches are always single-element
                let cmd = batch.commands[0];
                let input = filter_json.as_deref().or(pipeline_json).unwrap_or("{}");
                let result = run_filter_single(cmd, input, context, project_root)?;
                filter_json = Some(result.stdout.clone());
                all_results.push(result);
            }
            BatchKind::Parallel | BatchKind::Sync => {
                let results = run_batch(batch, context, project_root)?;
                all_results.extend(results);
            }
        }
    }

    Ok(RunHooksOutput {
        results: all_results,
        filter_output: filter_json,
    })
}

// ──────────────────────────────────────────────
// Internal helpers
// ──────────────────────────────────────────────

/// How a batch of commands should be executed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BatchKind {
    /// Multiple commands run concurrently.
    Parallel,
    /// A single command that runs alone (barrier).
    Sync,
    /// A single command that runs alone and pipes JSON via stdin/stdout.
    Filter,
}

/// A batch of commands to run. Sync and filter commands form single-element batches.
#[derive(Debug)]
struct Batch<'a> {
    commands: Vec<&'a str>,
    kind: BatchKind,
}

/// Split commands into batches at `sync:` and `filter:` boundaries.
///
/// Each `sync:` or `filter:` command creates three implicit batches:
/// 1. Everything before the barrier (parallel)
/// 2. The barrier command itself (alone)
/// 3. Everything after (parallel, until next barrier)
fn split_batches(commands: &[String]) -> Vec<Batch<'_>> {
    let mut batches = Vec::new();
    let mut current = Vec::new();

    for cmd in commands {
        if let Some(filter_cmd) = cmd.strip_prefix("filter:") {
            // Flush the current parallel batch
            if !current.is_empty() {
                batches.push(Batch {
                    commands: std::mem::take(&mut current),
                    kind: BatchKind::Parallel,
                });
            }
            // Add the filter command as a solo batch
            batches.push(Batch {
                commands: vec![filter_cmd.trim_start()],
                kind: BatchKind::Filter,
            });
        } else if let Some(sync_cmd) = cmd.strip_prefix("sync:") {
            // Flush the current parallel batch
            if !current.is_empty() {
                batches.push(Batch {
                    commands: std::mem::take(&mut current),
                    kind: BatchKind::Parallel,
                });
            }
            // Add the sync command as a solo batch
            batches.push(Batch {
                commands: vec![sync_cmd.trim_start()],
                kind: BatchKind::Sync,
            });
        } else {
            current.push(cmd.as_str());
        }
    }

    // Flush any remaining parallel commands
    if !current.is_empty() {
        batches.push(Batch {
            commands: current,
            kind: BatchKind::Parallel,
        });
    }

    batches
}

/// Run all commands in a batch. Multiple commands run in parallel.
fn run_batch(
    batch: &Batch<'_>,
    context: &HookContext,
    project_root: &Utf8Path,
) -> HookResult<Vec<HookOutput>> {
    if batch.commands.len() == 1 {
        // Single command — run directly without spawning threads
        let cmd = batch.commands[0];
        let result = run_single(cmd, context, project_root)?;
        return Ok(vec![result]);
    }

    // Multiple commands — spawn all, then collect
    let mut children: Vec<(&str, std::process::Child, Instant)> = Vec::new();

    for &cmd in &batch.commands {
        let interpolated = interpolate(cmd, context);
        debug!(%interpolated, "spawning hook");

        let start = Instant::now();
        let child = Command::new("sh")
            .args(["-c", &interpolated])
            .current_dir(project_root.as_std_path())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()?;

        children.push((cmd, child, start));
    }

    // Wait for all children and collect results
    let mut results = Vec::new();
    let mut first_error: Option<HookError> = None;

    for (cmd, child, start) in children {
        let output = child.wait_with_output()?;
        let duration = start.elapsed();
        let success = output.status.success();

        results.push(HookOutput {
            command: cmd.to_string(),
            success,
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            duration,
        });

        if !success && first_error.is_none() {
            first_error = Some(HookError::CommandFailed {
                command: cmd.to_string(),
                exit_code: output.status.code(),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            });
        }
    }

    if let Some(err) = first_error {
        return Err(err);
    }

    Ok(results)
}

/// Run a single command synchronously.
fn run_single(cmd: &str, context: &HookContext, project_root: &Utf8Path) -> HookResult<HookOutput> {
    let interpolated = interpolate(cmd, context);
    debug!(%interpolated, "running hook");

    let start = Instant::now();
    let output = Command::new("sh")
        .args(["-c", &interpolated])
        .current_dir(project_root.as_std_path())
        .output()?;
    let duration = start.elapsed();

    let success = output.status.success();
    let result = HookOutput {
        command: cmd.to_string(),
        success,
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        duration,
    };

    if !success {
        return Err(HookError::CommandFailed {
            command: cmd.to_string(),
            exit_code: output.status.code(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }

    Ok(result)
}

/// Run a single filter command: pipe JSON to stdin, capture stdout.
///
/// The command runs through `sh -c` with stdin/stdout/stderr piped.
/// On success, validates that stdout is valid JSON before returning.
fn run_filter_single(
    cmd: &str,
    json_stdin: &str,
    context: &HookContext,
    project_root: &Utf8Path,
) -> HookResult<HookOutput> {
    let interpolated = interpolate(cmd, context);
    debug!(%interpolated, "running filter hook");

    let start = Instant::now();
    let mut child = Command::new("sh")
        .args(["-c", &interpolated])
        .current_dir(project_root.as_std_path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    // Write JSON to stdin, then drop to close the pipe
    if let Some(mut stdin) = child.stdin.take() {
        // Ignore write errors — the child may have exited early
        let _ = stdin.write_all(json_stdin.as_bytes());
    }

    let output = child.wait_with_output()?;
    let duration = start.elapsed();
    let success = output.status.success();

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    if !success {
        return Err(HookError::CommandFailed {
            command: cmd.to_string(),
            exit_code: output.status.code(),
            stderr: stderr.trim().to_string(),
        });
    }

    // Validate that stdout is valid JSON
    let trimmed = stdout.trim();
    if serde_json::from_str::<serde_json::Value>(trimmed).is_err() {
        return Err(HookError::FilterOutputInvalid {
            command: cmd.to_string(),
            detail: if trimmed.len() > 200 {
                format!("{}...", &trimmed[..200])
            } else {
                trimmed.to_string()
            },
        });
    }

    Ok(HookOutput {
        command: cmd.to_string(),
        success,
        stdout: trimmed.to_string(),
        stderr,
        duration,
    })
}

/// Replace `{var}` placeholders with values from the context.
///
/// Public so that the ship orchestrator can preview interpolated commands
/// for dry-run display.
pub fn interpolate_command(command: &str, context: &HookContext) -> String {
    interpolate(command, context)
}

/// Replace `{var}` placeholders with values from the context.
fn interpolate(command: &str, context: &HookContext) -> String {
    command
        .replace("{version}", &context.version)
        .replace("{prev_version}", &context.prev_version)
        .replace("{tag}", &context.tag)
        .replace("{changelog_path}", &context.changelog_path)
        .replace("{owner}", &context.owner)
        .replace("{repo}", &context.repo)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_context() -> HookContext {
        HookContext {
            version: "1.2.3".into(),
            prev_version: "1.1.0".into(),
            tag: "v1.2.3".into(),
            changelog_path: "CHANGELOG.md".into(),
            owner: "claylo".into(),
            repo: "scrat".into(),
        }
    }

    #[test]
    fn interpolate_all_variables() {
        let ctx = test_context();
        let result = interpolate(
            "echo {version} {prev_version} {tag} {changelog_path} {owner}/{repo}",
            &ctx,
        );
        assert_eq!(result, "echo 1.2.3 1.1.0 v1.2.3 CHANGELOG.md claylo/scrat");
    }

    #[test]
    fn interpolate_preserves_unknown_braces() {
        let ctx = test_context();
        let result = interpolate("echo {unknown} {version}", &ctx);
        assert_eq!(result, "echo {unknown} 1.2.3");
    }

    #[test]
    fn interpolate_no_variables() {
        let ctx = test_context();
        let result = interpolate("echo hello", &ctx);
        assert_eq!(result, "echo hello");
    }

    #[test]
    fn split_batches_no_sync() {
        let commands: Vec<String> = vec!["echo a".into(), "echo b".into(), "echo c".into()];
        let batches = split_batches(&commands);
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].commands.len(), 3);
        assert_eq!(batches[0].kind, BatchKind::Parallel);
    }

    #[test]
    fn split_batches_with_sync() {
        let commands: Vec<String> = vec![
            "echo a".into(),
            "echo b".into(),
            "sync:echo barrier".into(),
            "echo c".into(),
            "echo d".into(),
        ];
        let batches = split_batches(&commands);
        assert_eq!(batches.len(), 3);
        assert_eq!(batches[0].commands, vec!["echo a", "echo b"]);
        assert_eq!(batches[0].kind, BatchKind::Parallel);
        assert_eq!(batches[1].commands, vec!["echo barrier"]);
        assert_eq!(batches[1].kind, BatchKind::Sync);
        assert_eq!(batches[2].commands, vec!["echo c", "echo d"]);
        assert_eq!(batches[2].kind, BatchKind::Parallel);
    }

    #[test]
    fn split_batches_sync_at_start() {
        let commands: Vec<String> = vec!["sync:echo first".into(), "echo a".into()];
        let batches = split_batches(&commands);
        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0].commands, vec!["echo first"]);
        assert_eq!(batches[0].kind, BatchKind::Sync);
        assert_eq!(batches[1].commands, vec!["echo a"]);
        assert_eq!(batches[1].kind, BatchKind::Parallel);
    }

    #[test]
    fn split_batches_sync_at_end() {
        let commands: Vec<String> = vec!["echo a".into(), "sync:echo last".into()];
        let batches = split_batches(&commands);
        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0].commands, vec!["echo a"]);
        assert_eq!(batches[0].kind, BatchKind::Parallel);
        assert_eq!(batches[1].commands, vec!["echo last"]);
        assert_eq!(batches[1].kind, BatchKind::Sync);
    }

    #[test]
    fn split_batches_consecutive_syncs() {
        let commands: Vec<String> = vec!["sync:echo one".into(), "sync:echo two".into()];
        let batches = split_batches(&commands);
        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0].commands, vec!["echo one"]);
        assert_eq!(batches[0].kind, BatchKind::Sync);
        assert_eq!(batches[1].commands, vec!["echo two"]);
        assert_eq!(batches[1].kind, BatchKind::Sync);
    }

    #[test]
    fn split_batches_empty() {
        let commands: Vec<String> = vec![];
        let batches = split_batches(&commands);
        assert!(batches.is_empty());
    }

    #[test]
    fn run_hooks_empty_list() {
        let ctx = test_context();
        let tmp = tempfile::TempDir::new().unwrap();
        let root = camino::Utf8Path::from_path(tmp.path()).unwrap();
        let output = run_hooks(&[], &ctx, root, None).unwrap();
        assert!(output.results.is_empty());
        assert!(output.filter_output.is_none());
    }

    #[test]
    fn run_hooks_single_command() {
        let ctx = test_context();
        let tmp = tempfile::TempDir::new().unwrap();
        let root = camino::Utf8Path::from_path(tmp.path()).unwrap();
        let commands = vec!["echo {version}".to_string()];
        let output = run_hooks(&commands, &ctx, root, None).unwrap();
        assert_eq!(output.results.len(), 1);
        assert!(output.results[0].success);
        assert_eq!(output.results[0].stdout.trim(), "1.2.3");
    }

    #[test]
    fn run_hooks_failure_stops_execution() {
        let ctx = test_context();
        let tmp = tempfile::TempDir::new().unwrap();
        let root = camino::Utf8Path::from_path(tmp.path()).unwrap();
        let commands = vec![
            "false".to_string(),           // fails
            "sync:echo after".to_string(), // should not run
        ];
        let result = run_hooks(&commands, &ctx, root, None);
        assert!(result.is_err());
    }

    #[test]
    fn run_hooks_with_sync_barrier() {
        let ctx = test_context();
        let tmp = tempfile::TempDir::new().unwrap();
        let root = camino::Utf8Path::from_path(tmp.path()).unwrap();
        let commands = vec![
            "echo first".to_string(),
            "sync:echo barrier".to_string(),
            "echo last".to_string(),
        ];
        let output = run_hooks(&commands, &ctx, root, None).unwrap();
        assert_eq!(output.results.len(), 3);
        assert!(output.results.iter().all(|r| r.success));
    }

    #[test]
    fn run_hooks_interpolates_variables() {
        let ctx = test_context();
        let tmp = tempfile::TempDir::new().unwrap();
        let root = camino::Utf8Path::from_path(tmp.path()).unwrap();
        let commands = vec!["echo {owner}/{repo}@{tag}".to_string()];
        let output = run_hooks(&commands, &ctx, root, None).unwrap();
        assert_eq!(output.results[0].stdout.trim(), "claylo/scrat@v1.2.3");
    }

    // ── Filter tests ──

    #[test]
    fn split_batches_with_filter() {
        let commands: Vec<String> = vec!["echo a".into(), "filter: jq '.'".into(), "echo b".into()];
        let batches = split_batches(&commands);
        assert_eq!(batches.len(), 3);
        assert_eq!(batches[0].commands, vec!["echo a"]);
        assert_eq!(batches[0].kind, BatchKind::Parallel);
        assert_eq!(batches[1].commands, vec!["jq '.'"]);
        assert_eq!(batches[1].kind, BatchKind::Filter);
        assert_eq!(batches[2].commands, vec!["echo b"]);
        assert_eq!(batches[2].kind, BatchKind::Parallel);
    }

    #[test]
    fn split_batches_filter_and_sync() {
        let commands: Vec<String> = vec![
            "echo a".into(),
            "sync:echo barrier".into(),
            "filter: jq '.'".into(),
            "echo b".into(),
        ];
        let batches = split_batches(&commands);
        assert_eq!(batches.len(), 4);
        assert_eq!(batches[0].kind, BatchKind::Parallel);
        assert_eq!(batches[1].kind, BatchKind::Sync);
        assert_eq!(batches[2].kind, BatchKind::Filter);
        assert_eq!(batches[2].commands, vec!["jq '.'"]);
        assert_eq!(batches[3].kind, BatchKind::Parallel);
    }

    #[test]
    fn run_filter_single_pipes_stdin() {
        let ctx = test_context();
        let tmp = tempfile::TempDir::new().unwrap();
        let root = camino::Utf8Path::from_path(tmp.path()).unwrap();
        let input = r#"{"key":"value"}"#;
        // `cat` passes stdin through unchanged
        let result = run_filter_single("cat", input, &ctx, root).unwrap();
        assert!(result.success);
        assert_eq!(result.stdout, input);
    }

    #[test]
    fn run_hooks_filter_mutates_json() {
        let ctx = test_context();
        let tmp = tempfile::TempDir::new().unwrap();
        let root = camino::Utf8Path::from_path(tmp.path()).unwrap();
        let commands = vec![r#"filter: jq '.added = true'"#.to_string()];
        let input = r#"{"version":"1.0.0"}"#;
        let output = run_hooks(&commands, &ctx, root, Some(input)).unwrap();
        assert_eq!(output.results.len(), 1);
        assert!(output.filter_output.is_some());
        let json: serde_json::Value =
            serde_json::from_str(output.filter_output.as_ref().unwrap()).unwrap();
        assert_eq!(json["version"], "1.0.0");
        assert_eq!(json["added"], true);
    }

    #[test]
    fn run_hooks_filter_chain() {
        let ctx = test_context();
        let tmp = tempfile::TempDir::new().unwrap();
        let root = camino::Utf8Path::from_path(tmp.path()).unwrap();
        let commands = vec![
            r#"filter: jq '.step1 = true'"#.to_string(),
            r#"filter: jq '.step2 = true'"#.to_string(),
        ];
        let input = r#"{"base":true}"#;
        let output = run_hooks(&commands, &ctx, root, Some(input)).unwrap();
        assert_eq!(output.results.len(), 2);
        let json: serde_json::Value =
            serde_json::from_str(output.filter_output.as_ref().unwrap()).unwrap();
        // Both filters applied
        assert_eq!(json["base"], true);
        assert_eq!(json["step1"], true);
        assert_eq!(json["step2"], true);
    }

    #[test]
    fn run_hooks_filter_invalid_json_errors() {
        let ctx = test_context();
        let tmp = tempfile::TempDir::new().unwrap();
        let root = camino::Utf8Path::from_path(tmp.path()).unwrap();
        let commands = vec![
            // echo outputs plain text, not valid JSON
            "filter: echo not-json".to_string(),
        ];
        let result = run_hooks(&commands, &ctx, root, Some("{}"));
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err, HookError::FilterOutputInvalid { .. }),
            "expected FilterOutputInvalid, got: {err:?}"
        );
    }

    #[test]
    fn run_hooks_filter_no_pipeline_json() {
        let ctx = test_context();
        let tmp = tempfile::TempDir::new().unwrap();
        let root = camino::Utf8Path::from_path(tmp.path()).unwrap();
        let commands = vec![
            // With no pipeline_json, filter receives "{}"
            r#"filter: jq '.injected = "yes"'"#.to_string(),
        ];
        let output = run_hooks(&commands, &ctx, root, None).unwrap();
        let json: serde_json::Value =
            serde_json::from_str(output.filter_output.as_ref().unwrap()).unwrap();
        assert_eq!(json["injected"], "yes");
    }
}
