//! Interactive version mode — data gathering.
//!
//! Provides the data the CLI needs to present an interactive version picker:
//! recent commits since the last tag, and candidate versions for each bump level.
//! The actual prompting (inquire) happens in the CLI crate.

use semver::Version;
use serde::Serialize;
use tracing::instrument;

use crate::version::{BumpLevel, VersionResult, current_version_from_tags, next_version};

/// Data for the interactive version picker.
#[derive(Debug, Clone, Serialize)]
pub struct InteractiveContext {
    /// Current version (from the latest tag). `None` if first release.
    pub current_version: Option<Version>,
    /// Recent commits since the last tag (hash, subject).
    pub recent_commits: Vec<(String, String)>,
    /// Candidate versions for each bump level.
    pub candidates: Vec<VersionCandidate>,
}

/// A candidate version with its bump level.
#[derive(Debug, Clone, Serialize)]
pub struct VersionCandidate {
    /// The bump level.
    pub level: BumpLevel,
    /// The resulting version.
    pub version: Version,
}

/// Gather the data for an interactive version prompt.
///
/// Returns recent commits and candidate versions. The CLI uses this
/// to display options and prompt the user.
#[instrument]
pub fn gather_interactive_context(max_commits: usize) -> VersionResult<InteractiveContext> {
    let current = current_version_from_tags()?;

    let since_tag = current.as_ref().map(|v| format!("v{v}"));
    let commits = crate::git::recent_commits(since_tag.as_deref(), max_commits)?;

    let candidates = current.as_ref().map_or_else(
        || {
            // First release — suggest 0.1.0 or 1.0.0
            vec![
                VersionCandidate {
                    level: BumpLevel::Minor,
                    version: Version::new(0, 1, 0),
                },
                VersionCandidate {
                    level: BumpLevel::Major,
                    version: Version::new(1, 0, 0),
                },
            ]
        },
        |v| {
            vec![
                VersionCandidate {
                    level: BumpLevel::Patch,
                    version: next_version(v, BumpLevel::Patch),
                },
                VersionCandidate {
                    level: BumpLevel::Minor,
                    version: next_version(v, BumpLevel::Minor),
                },
                VersionCandidate {
                    level: BumpLevel::Major,
                    version: next_version(v, BumpLevel::Major),
                },
            ]
        },
    );

    Ok(InteractiveContext {
        current_version: current,
        recent_commits: commits,
        candidates,
    })
}
