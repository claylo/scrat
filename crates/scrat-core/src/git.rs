//! Git operations for release workflows.
//!
//! Shells out to `git` for all operations. This ensures we inherit the user's
//! SSH keys, GPG signing, hooks, and other configuration.

use std::process::Command;

use thiserror::Error;
use tracing::{debug, instrument};

/// Errors from git operations.
#[derive(Error, Debug)]
pub enum GitError {
    /// Failed to execute the `git` command.
    #[error("failed to run git: {0}")]
    Exec(#[from] std::io::Error),

    /// `git` returned a non-zero exit code.
    #[error("git {command} failed: {stderr}")]
    Command {
        /// The git subcommand that failed (e.g., "status").
        command: String,
        /// Captured stderr.
        stderr: String,
    },

    /// Not inside a git repository.
    #[error("not a git repository (or any parent up to mount point)")]
    NotARepo,
}

/// Result alias for git operations.
pub type GitResult<T> = Result<T, GitError>;

/// Check whether the working tree is clean (no uncommitted changes).
///
/// Returns `true` if both staged and unstaged changes are empty.
#[instrument]
pub fn is_clean() -> GitResult<bool> {
    let output = git(&["status", "--porcelain"])?;
    let clean = output.trim().is_empty();
    debug!(clean, "working tree status");
    Ok(clean)
}

/// Get the current branch name.
///
/// Returns `None` if in a detached HEAD state.
#[instrument]
pub fn current_branch() -> GitResult<Option<String>> {
    let output = git(&["rev-parse", "--abbrev-ref", "HEAD"])?;
    let branch = output.trim().to_string();
    if branch == "HEAD" {
        debug!("detached HEAD");
        Ok(None)
    } else {
        debug!(%branch, "current branch");
        Ok(Some(branch))
    }
}

/// Detect the release branch by checking for `main` then `master`.
///
/// Returns the first one that exists as a local branch.
#[instrument]
pub fn detect_release_branch() -> GitResult<Option<String>> {
    for candidate in &["main", "master"] {
        let result = git(&["rev-parse", "--verify", candidate]);
        if result.is_ok() {
            debug!(branch = candidate, "detected release branch");
            return Ok(Some((*candidate).to_string()));
        }
    }
    debug!("no main/master branch found");
    Ok(None)
}

/// Check whether the local branch is in sync with its remote tracking branch.
///
/// Returns `true` if there are no unpulled or unpushed commits.
/// Returns `true` if there is no upstream configured (nothing to be out-of-sync with).
#[instrument]
#[expect(clippy::literal_string_with_formatting_args)]
pub fn is_remote_in_sync() -> GitResult<bool> {
    // Get the upstream tracking ref — @{upstream} is a git refspec, not a format arg
    let upstream = git(&["rev-parse", "--abbrev-ref", "@{upstream}"]);
    let Ok(upstream) = upstream else {
        // No upstream configured — nothing to sync against
        debug!("no upstream tracking branch");
        return Ok(true);
    };
    let upstream = upstream.trim();

    // Fetch to get latest remote state (non-fatal if it fails)
    let _ = git(&["fetch", "--quiet"]);

    // Compare local HEAD with upstream
    let local = git(&["rev-parse", "HEAD"])?.trim().to_string();
    let remote = git(&["rev-parse", upstream])?.trim().to_string();

    let in_sync = local == remote;
    debug!(%local, %remote, in_sync, "remote sync check");
    Ok(in_sync)
}

/// Get the latest semver tag, if any.
///
/// Looks for tags matching `v*` and sorts by version.
#[instrument]
pub fn latest_version_tag() -> GitResult<Option<String>> {
    // Use git tag with version sort to find the latest semver tag
    let output = git(&["tag", "--list", "v*", "--sort=-version:refname"]);
    let Ok(output) = output else {
        return Ok(None);
    };

    let tag = output.lines().next().map(|s| s.trim().to_string());
    debug!(?tag, "latest version tag");
    Ok(tag)
}

/// Get recent commits since a ref (or all commits if `None`).
///
/// Returns a list of `(short_hash, subject)` tuples, newest first.
#[instrument]
pub fn recent_commits(since: Option<&str>, limit: usize) -> GitResult<Vec<(String, String)>> {
    let range = since.map_or_else(|| "HEAD".to_string(), |tag| format!("{tag}..HEAD"));

    let output = git(&[
        "log",
        &range,
        &format!("--max-count={limit}"),
        "--format=%h %s",
    ])?;

    let commits: Vec<(String, String)> = output
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| {
            let (hash, subject) = line.split_once(' ').unwrap_or((line, ""));
            (hash.to_string(), subject.to_string())
        })
        .collect();

    debug!(count = commits.len(), "recent commits");
    Ok(commits)
}

/// Get the remote URL for a named remote (default: `"origin"`).
#[instrument]
pub fn remote_url(remote: &str) -> GitResult<Option<String>> {
    let result = git(&["remote", "get-url", remote]);
    match result {
        Ok(url) => {
            let url = url.trim().to_string();
            debug!(%remote, %url, "remote URL");
            Ok(Some(url))
        }
        Err(GitError::Command { .. }) => Ok(None),
        Err(e) => Err(e),
    }
}

/// Parse owner and repo from a git remote URL.
///
/// Handles both HTTPS and SSH formats:
/// - `https://github.com/owner/repo.git`
/// - `git@github.com:owner/repo.git`
///
/// Returns `None` if the URL cannot be parsed.
pub fn parse_owner_repo(url: &str) -> Option<(String, String)> {
    let path = url.strip_prefix("git@").map_or_else(
        || {
            // HTTPS format: https://github.com/owner/repo.git
            url.split("//")
                .nth(1)
                .and_then(|after_scheme| after_scheme.split_once('/').map(|(_, path)| path))
        },
        |rest| {
            // SSH format: git@github.com:owner/repo.git
            rest.split_once(':').map(|(_, path)| path)
        },
    )?;

    let path = path.strip_suffix(".git").unwrap_or(path);
    let (owner, repo) = path.split_once('/')?;

    if owner.is_empty() || repo.is_empty() || repo.contains('/') {
        return None;
    }

    Some((owner.to_string(), repo.to_string()))
}

/// Get the unified diff for a specific file between a ref and HEAD.
///
/// Returns an empty string if the file doesn't exist in either ref
/// or has no changes.
#[instrument]
pub fn diff_file(since: &str, path: &str) -> GitResult<String> {
    let range = format!("{since}..HEAD");
    let result = git(&["diff", &range, "--", path]);
    match result {
        Ok(output) => {
            debug!(path, bytes = output.len(), "diff_file");
            Ok(output)
        }
        Err(GitError::Command { .. }) => {
            debug!(
                path,
                "diff_file: no diff (command error, treating as empty)"
            );
            Ok(String::new())
        }
        Err(e) => Err(e),
    }
}

/// Statistics about changes since a given ref.
#[derive(Debug, Clone, serde::Serialize)]
pub struct GitStats {
    /// Number of commits since the ref.
    pub commit_count: usize,
    /// Number of files changed.
    pub files_changed: usize,
    /// Total lines inserted.
    pub insertions: usize,
    /// Total lines deleted.
    pub deletions: usize,
}

/// Stage files and create a commit.
///
/// Returns the new commit hash.
#[instrument(skip(message))]
pub fn commit(files: &[&str], message: &str) -> GitResult<String> {
    // Stage files
    let mut args = vec!["add", "--"];
    args.extend_from_slice(files);
    git(&args)?;

    // Create the commit
    git(&["commit", "-m", message])?;

    // Return the new commit hash
    let hash = git(&["rev-parse", "--short", "HEAD"])?.trim().to_string();
    debug!(%hash, "created commit");
    Ok(hash)
}

/// Create an annotated tag.
#[instrument]
pub fn create_tag(name: &str, message: &str) -> GitResult<()> {
    git(&["tag", "-a", name, "-m", message])?;
    debug!(%name, "created tag");
    Ok(())
}

/// Push branch and optionally tags to a remote.
#[instrument]
pub fn push(remote: &str, branch: &str, push_tags: bool) -> GitResult<()> {
    git(&["push", remote, branch])?;
    if push_tags {
        git(&["push", remote, "--tags"])?;
    }
    debug!(%remote, %branch, push_tags, "pushed");
    Ok(())
}

/// Get statistics since a ref: commit count, files changed, insertions, deletions.
#[instrument]
pub fn stats_since(since: &str) -> GitResult<GitStats> {
    // Count commits
    let log_output = git(&["log", &format!("{since}..HEAD"), "--oneline"])?;
    let commit_count = log_output.lines().filter(|l| !l.is_empty()).count();

    // Get diff stats
    let diff_output = git(&["diff", "--stat", since])?;
    let (files_changed, insertions, deletions) = parse_diff_stat(&diff_output);

    let stats = GitStats {
        commit_count,
        files_changed,
        insertions,
        deletions,
    };
    debug!(?stats, "stats since {since}");
    Ok(stats)
}

/// Get top contributors since a ref.
///
/// Returns `(name, commit_count)` tuples sorted by count descending.
#[instrument]
pub fn contributors_since(since: &str, limit: usize) -> GitResult<Vec<(String, usize)>> {
    let output = git(&["shortlog", "-sn", "--no-merges", &format!("{since}..HEAD")])?;

    let contributors: Vec<(String, usize)> = output
        .lines()
        .filter(|l| !l.is_empty())
        .take(limit)
        .filter_map(|line| {
            let line = line.trim();
            let (count_str, name) = line.split_once('\t').or_else(|| line.split_once(' '))?;
            let count = count_str.trim().parse().ok()?;
            Some((name.trim().to_string(), count))
        })
        .collect();

    debug!(count = contributors.len(), "contributors since {since}");
    Ok(contributors)
}

/// Parse the summary line from `git diff --stat`.
///
/// Example: ` 3 files changed, 42 insertions(+), 10 deletions(-)`
fn parse_diff_stat(output: &str) -> (usize, usize, usize) {
    let Some(summary) = output.lines().last() else {
        return (0, 0, 0);
    };

    let mut files = 0;
    let mut insertions = 0;
    let mut deletions = 0;

    for part in summary.split(',') {
        let part = part.trim();
        let n = part
            .split_whitespace()
            .next()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);

        if part.contains("file") {
            files = n;
        } else if part.contains("insertion") {
            insertions = n;
        } else if part.contains("deletion") {
            deletions = n;
        }
    }

    (files, insertions, deletions)
}

/// Check if we're inside a git repository.
#[instrument]
pub fn is_inside_repo() -> GitResult<bool> {
    let result = git(&["rev-parse", "--is-inside-work-tree"]);
    match result {
        Ok(output) => Ok(output.trim() == "true"),
        Err(GitError::Command { .. }) => Ok(false),
        Err(e) => Err(e),
    }
}

/// Run a git command and return its stdout.
fn git(args: &[&str]) -> GitResult<String> {
    let output = Command::new("git").args(args).output()?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

        // Detect "not a git repo" specifically
        if stderr.contains("not a git repository") {
            return Err(GitError::NotARepo);
        }

        Err(GitError::Command {
            command: args.first().unwrap_or(&"").to_string(),
            stderr,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // These tests are designed to work both inside and outside a git repo.
    // The scrat project itself IS a git repo, so they should pass in normal
    // development. In CI or isolated environments, they gracefully handle
    // the non-repo case.

    #[test]
    fn is_inside_repo_returns_bool() {
        // Should not error regardless of whether we're in a repo
        let result = is_inside_repo();
        assert!(result.is_ok());
    }

    #[test]
    fn is_clean_works_in_repo() {
        if is_inside_repo().unwrap_or(false) {
            // Just verify it doesn't error — the actual clean/dirty
            // state depends on the working tree
            let result = is_clean();
            assert!(result.is_ok());
        }
    }

    #[test]
    fn current_branch_works_in_repo() {
        if is_inside_repo().unwrap_or(false) {
            let result = current_branch();
            assert!(result.is_ok());
            // In a normal checkout, we should have a branch name
            if let Ok(Some(branch)) = result {
                assert!(!branch.is_empty());
            }
        }
    }

    #[test]
    fn detect_release_branch_works_in_repo() {
        if is_inside_repo().unwrap_or(false) {
            let result = detect_release_branch();
            assert!(result.is_ok());
        }
    }

    #[test]
    fn latest_version_tag_works() {
        if is_inside_repo().unwrap_or(false) {
            let result = latest_version_tag();
            assert!(result.is_ok());
        }
    }

    #[test]
    fn recent_commits_works() {
        if is_inside_repo().unwrap_or(false) {
            let result = recent_commits(None, 5);
            assert!(result.is_ok());
        }
    }

    #[test]
    fn diff_file_nonexistent() {
        if is_inside_repo().unwrap_or(false) {
            // A file that doesn't exist should produce an empty diff, not an error
            let result = diff_file("HEAD", "this-file-does-not-exist-at-all.xyz");
            assert!(result.is_ok());
            assert!(result.unwrap().is_empty());
        }
    }

    #[test]
    fn git_error_on_bad_command() {
        // This should fail with a GitError::Command
        let result = git(&["not-a-real-subcommand"]);
        assert!(result.is_err());
    }

    #[test]
    fn remote_url_works_in_repo() {
        if is_inside_repo().unwrap_or(false) {
            let result = remote_url("origin");
            assert!(result.is_ok());
        }
    }

    #[test]
    fn parse_owner_repo_https() {
        let result = parse_owner_repo("https://github.com/claylo/scrat.git");
        assert_eq!(result, Some(("claylo".into(), "scrat".into())));
    }

    #[test]
    fn parse_owner_repo_https_no_suffix() {
        let result = parse_owner_repo("https://github.com/claylo/scrat");
        assert_eq!(result, Some(("claylo".into(), "scrat".into())));
    }

    #[test]
    fn parse_owner_repo_ssh() {
        let result = parse_owner_repo("git@github.com:claylo/scrat.git");
        assert_eq!(result, Some(("claylo".into(), "scrat".into())));
    }

    #[test]
    fn parse_owner_repo_ssh_no_suffix() {
        let result = parse_owner_repo("git@github.com:claylo/scrat");
        assert_eq!(result, Some(("claylo".into(), "scrat".into())));
    }

    #[test]
    fn parse_owner_repo_invalid() {
        assert!(parse_owner_repo("not-a-url").is_none());
        assert!(parse_owner_repo("").is_none());
    }

    #[test]
    fn parse_diff_stat_full_line() {
        let output = " file1.rs | 10 +++\n file2.rs | 5 ---\n 2 files changed, 10 insertions(+), 5 deletions(-)\n";
        let (files, ins, del) = parse_diff_stat(output);
        assert_eq!(files, 2);
        assert_eq!(ins, 10);
        assert_eq!(del, 5);
    }

    #[test]
    fn parse_diff_stat_insertions_only() {
        let output = " 1 file changed, 42 insertions(+)\n";
        let (files, ins, del) = parse_diff_stat(output);
        assert_eq!(files, 1);
        assert_eq!(ins, 42);
        assert_eq!(del, 0);
    }

    #[test]
    fn parse_diff_stat_empty() {
        let (files, ins, del) = parse_diff_stat("");
        assert_eq!(files, 0);
        assert_eq!(ins, 0);
        assert_eq!(del, 0);
    }

    #[test]
    fn git_stats_serializes() {
        let stats = GitStats {
            commit_count: 5,
            files_changed: 3,
            insertions: 42,
            deletions: 10,
        };
        let json = serde_json::to_string(&stats).unwrap();
        assert!(json.contains("\"commit_count\":5"));
    }
}
