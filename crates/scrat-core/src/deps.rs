//! Dependency diff — parse lockfile diffs to extract dependency changes.
//!
//! Computes `Vec<DepChange>` from `git diff` of ecosystem-specific lockfiles
//! between a previous tag and HEAD. This data feeds release notes templates
//! and `filter:` hooks via the [`PipelineContext`](crate::pipeline::PipelineContext).
//!
//! Per-ecosystem parsing is delegated to [`Ecosystem::driver`](crate::ecosystem::Ecosystem::driver)
//! via the [`EcosystemDriver`](crate::ecosystem::EcosystemDriver) trait.

use tracing::{debug, warn};

use crate::ecosystem::Ecosystem;
use crate::git;
use crate::pipeline::DepChange;

/// Compute dependency changes between a ref and HEAD for the given ecosystem.
///
/// Returns an empty `Vec` if the lockfile doesn't exist or hasn't changed.
/// Deps diff failure is non-fatal — logs a warning and returns empty.
pub fn compute_deps(ecosystem: Ecosystem, previous_tag: &str) -> Vec<DepChange> {
    let Some(lockfile) = ecosystem.lockfile_path() else {
        debug!(%ecosystem, "no lockfile for ecosystem, skipping deps diff");
        return Vec::new();
    };

    let diff = match git::diff_file(previous_tag, lockfile) {
        Ok(d) => d,
        Err(e) => {
            warn!(%e, lockfile, "failed to diff lockfile, skipping deps");
            return Vec::new();
        }
    };

    if diff.is_empty() {
        debug!(lockfile, "no lockfile changes");
        return Vec::new();
    }

    let changes = ecosystem.driver().parse_lockfile_diff(&diff);

    debug!(lockfile, count = changes.len(), "parsed dep changes");
    changes
}
