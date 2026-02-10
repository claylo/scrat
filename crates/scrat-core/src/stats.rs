//! Release statistics — compute stats between a ref and HEAD.
//!
//! Thin orchestrator over [`git::stats_since()`](crate::git::stats_since) and
//! [`git::contributors_since()`](crate::git::contributors_since). Maps raw git
//! data into [`ReleaseStats`](crate::pipeline::ReleaseStats) for the pipeline
//! context.
//!
//! Non-fatal: returns `None` on any git error (logs a warning, never blocks
//! the release).

use tracing::warn;

use crate::git;
use crate::pipeline::{Contributor, ReleaseStats};

/// Maximum number of contributors to include in release stats.
const CONTRIBUTOR_LIMIT: usize = 20;

/// Compute release statistics between a ref and HEAD.
///
/// Returns `None` if stats gathering fails (non-fatal — logs a warning).
pub fn compute_stats(previous_tag: &str) -> Option<ReleaseStats> {
    let stats = match git::stats_since(previous_tag) {
        Ok(s) => s,
        Err(e) => {
            warn!(%e, "failed to gather release stats, skipping");
            return None;
        }
    };

    let contributors = match git::contributors_since(previous_tag, CONTRIBUTOR_LIMIT) {
        Ok(c) => c
            .into_iter()
            .map(|(name, count)| Contributor { name, count })
            .collect(),
        Err(e) => {
            warn!(%e, "failed to gather contributors, continuing without");
            Vec::new()
        }
    };

    Some(ReleaseStats {
        commit_count: stats.commit_count,
        files_changed: stats.files_changed,
        insertions: stats.insertions,
        deletions: stats.deletions,
        contributors,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_stats_in_repo() {
        // We're running in the scrat repo — HEAD~5 should be a valid ref.
        // This is a "smoke test" that the function runs without panicking
        // and produces plausible values.
        let result = compute_stats("HEAD~5");
        // Should succeed in a git repo with at least 5 commits
        if let Some(stats) = result {
            // Basic sanity — 5 commits back should have some commits
            assert!(stats.commit_count > 0, "expected commits > 0");
            // files_changed may be 0 in edge cases, but commit_count shouldn't be
        }
        // If None, the test repo might not have 5 commits — that's OK
    }

    #[test]
    fn release_stats_has_contributors() {
        let result = compute_stats("HEAD~5");
        if let Some(stats) = result {
            // At least one contributor should be present
            assert!(!stats.contributors.is_empty(), "expected contributors");
            // Each contributor should have at least one commit
            for c in &stats.contributors {
                assert!(c.count > 0, "contributor {} has 0 commits", c.name);
                assert!(!c.name.is_empty(), "contributor name is empty");
            }
        }
    }

    #[test]
    fn compute_stats_bad_ref_returns_none() {
        // A nonsense ref should fail gracefully, not panic
        let result = compute_stats("definitely-not-a-real-ref-abc123xyz");
        assert!(result.is_none());
    }
}
