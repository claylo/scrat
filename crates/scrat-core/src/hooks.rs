//! Hook executor for release workflow phases.
//!
//! Hooks are user-configured shell commands that run at phase boundaries
//! during the ship workflow. Commands support variable interpolation and
//! can run in parallel or sequentially.
//!
//! # Execution model
//!
//! Commands run in parallel by default. Prefix a command with `sync:`
//! to create a barrier:
//!
//! 1. All prior (parallel) commands must finish
//! 2. The sync command runs alone
//! 3. Subsequent commands resume in parallel
//!
//! # Variables
//!
//! Commands support `{var}` interpolation for:
//! `{version}`, `{prev_version}`, `{tag}`, `{changelog_path}`,
//! `{owner}`, `{repo}`.

use std::process::Command;
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

/// Run a list of hook commands with variable interpolation.
///
/// Commands run in parallel by default. Prefix a command with `sync:`
/// to create a barrier — all prior commands must finish, the sync
/// command runs alone, then subsequent commands resume in parallel.
///
/// Returns the results of all executed commands. If any command in a
/// batch fails, remaining batches are skipped and an error is returned.
#[instrument(skip_all, fields(count = commands.len()))]
pub fn run_hooks(
    commands: &[String],
    context: &HookContext,
    project_root: &Utf8Path,
) -> HookResult<Vec<HookOutput>> {
    if commands.is_empty() {
        return Ok(Vec::new());
    }

    let batches = split_batches(commands);
    debug!(batch_count = batches.len(), "executing hook batches");

    let mut all_results = Vec::new();

    for batch in &batches {
        let results = run_batch(batch, context, project_root)?;
        all_results.extend(results);
    }

    Ok(all_results)
}

// ──────────────────────────────────────────────
// Internal helpers
// ──────────────────────────────────────────────

/// A batch of commands to run. Sync commands form single-element batches.
#[derive(Debug)]
struct Batch<'a> {
    commands: Vec<&'a str>,
}

/// Split commands into batches at `sync:` boundaries.
///
/// Each `sync:` command creates three implicit batches:
/// 1. Everything before the sync (parallel)
/// 2. The sync command itself (alone)
/// 3. Everything after (parallel, until next sync)
fn split_batches(commands: &[String]) -> Vec<Batch<'_>> {
    let mut batches = Vec::new();
    let mut current = Vec::new();

    for cmd in commands {
        if let Some(sync_cmd) = cmd.strip_prefix("sync:") {
            // Flush the current parallel batch
            if !current.is_empty() {
                batches.push(Batch {
                    commands: std::mem::take(&mut current),
                });
            }
            // Add the sync command as a solo batch
            batches.push(Batch {
                commands: vec![sync_cmd.trim_start()],
            });
        } else {
            current.push(cmd.as_str());
        }
    }

    // Flush any remaining parallel commands
    if !current.is_empty() {
        batches.push(Batch { commands: current });
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
        assert_eq!(batches[1].commands, vec!["echo barrier"]);
        assert_eq!(batches[2].commands, vec!["echo c", "echo d"]);
    }

    #[test]
    fn split_batches_sync_at_start() {
        let commands: Vec<String> = vec!["sync:echo first".into(), "echo a".into()];
        let batches = split_batches(&commands);
        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0].commands, vec!["echo first"]);
        assert_eq!(batches[1].commands, vec!["echo a"]);
    }

    #[test]
    fn split_batches_sync_at_end() {
        let commands: Vec<String> = vec!["echo a".into(), "sync:echo last".into()];
        let batches = split_batches(&commands);
        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0].commands, vec!["echo a"]);
        assert_eq!(batches[1].commands, vec!["echo last"]);
    }

    #[test]
    fn split_batches_consecutive_syncs() {
        let commands: Vec<String> = vec!["sync:echo one".into(), "sync:echo two".into()];
        let batches = split_batches(&commands);
        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0].commands, vec!["echo one"]);
        assert_eq!(batches[1].commands, vec!["echo two"]);
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
        let results = run_hooks(&[], &ctx, root).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn run_hooks_single_command() {
        let ctx = test_context();
        let tmp = tempfile::TempDir::new().unwrap();
        let root = camino::Utf8Path::from_path(tmp.path()).unwrap();
        let commands = vec!["echo {version}".to_string()];
        let results = run_hooks(&commands, &ctx, root).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].success);
        assert_eq!(results[0].stdout.trim(), "1.2.3");
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
        let result = run_hooks(&commands, &ctx, root);
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
        let results = run_hooks(&commands, &ctx, root).unwrap();
        assert_eq!(results.len(), 3);
        assert!(results.iter().all(|r| r.success));
    }

    #[test]
    fn run_hooks_interpolates_variables() {
        let ctx = test_context();
        let tmp = tempfile::TempDir::new().unwrap();
        let root = camino::Utf8Path::from_path(tmp.path()).unwrap();
        let commands = vec!["echo {owner}/{repo}@{tag}".to_string()];
        let results = run_hooks(&commands, &ctx, root).unwrap();
        assert_eq!(results[0].stdout.trim(), "claylo/scrat@v1.2.3");
    }
}
