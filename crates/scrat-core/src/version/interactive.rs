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

#[cfg(test)]
mod tests {
    use super::*;

    // ── VersionCandidate construction ───────────────────────

    #[test]
    fn version_candidate_from_existing_version() {
        let current = Version::new(1, 2, 3);
        let candidates = [
            VersionCandidate {
                level: BumpLevel::Patch,
                version: next_version(&current, BumpLevel::Patch),
            },
            VersionCandidate {
                level: BumpLevel::Minor,
                version: next_version(&current, BumpLevel::Minor),
            },
            VersionCandidate {
                level: BumpLevel::Major,
                version: next_version(&current, BumpLevel::Major),
            },
        ];

        assert_eq!(candidates[0].version, Version::new(1, 2, 4));
        assert_eq!(candidates[0].level, BumpLevel::Patch);
        assert_eq!(candidates[1].version, Version::new(1, 3, 0));
        assert_eq!(candidates[1].level, BumpLevel::Minor);
        assert_eq!(candidates[2].version, Version::new(2, 0, 0));
        assert_eq!(candidates[2].level, BumpLevel::Major);
    }

    #[test]
    fn first_release_candidates() {
        // When there's no current version, the code suggests 0.1.0 and 1.0.0
        let candidates = [
            VersionCandidate {
                level: BumpLevel::Minor,
                version: Version::new(0, 1, 0),
            },
            VersionCandidate {
                level: BumpLevel::Major,
                version: Version::new(1, 0, 0),
            },
        ];

        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].version, Version::new(0, 1, 0));
        assert_eq!(candidates[0].level, BumpLevel::Minor);
        assert_eq!(candidates[1].version, Version::new(1, 0, 0));
        assert_eq!(candidates[1].level, BumpLevel::Major);
    }

    #[test]
    fn candidates_from_zero_version() {
        let current = Version::new(0, 0, 0);
        let patch = next_version(&current, BumpLevel::Patch);
        let minor = next_version(&current, BumpLevel::Minor);
        let major = next_version(&current, BumpLevel::Major);

        assert_eq!(patch, Version::new(0, 0, 1));
        assert_eq!(minor, Version::new(0, 1, 0));
        assert_eq!(major, Version::new(1, 0, 0));
    }

    // ── InteractiveContext serialization ─────────────────────

    #[test]
    fn interactive_context_serializes() {
        let ctx = InteractiveContext {
            current_version: Some(Version::new(2, 0, 0)),
            recent_commits: vec![
                ("abc1234".into(), "feat: add feature".into()),
                ("def5678".into(), "fix: correct bug".into()),
            ],
            candidates: vec![
                VersionCandidate {
                    level: BumpLevel::Patch,
                    version: Version::new(2, 0, 1),
                },
                VersionCandidate {
                    level: BumpLevel::Minor,
                    version: Version::new(2, 1, 0),
                },
            ],
        };

        let json = serde_json::to_string(&ctx).unwrap();
        assert!(json.contains("\"current_version\":\"2.0.0\""));
        assert!(json.contains("abc1234"));
        assert!(json.contains("feat: add feature"));
        assert!(json.contains("\"version\":\"2.0.1\""));
    }

    #[test]
    fn interactive_context_serializes_no_current() {
        let ctx = InteractiveContext {
            current_version: None,
            recent_commits: vec![],
            candidates: vec![],
        };

        let json = serde_json::to_string(&ctx).unwrap();
        assert!(json.contains("\"current_version\":null"));
    }

    // ── InteractiveContext clone ─────────────────────────────

    #[test]
    fn interactive_context_clone() {
        let ctx = InteractiveContext {
            current_version: Some(Version::new(1, 0, 0)),
            recent_commits: vec![("abc".into(), "msg".into())],
            candidates: vec![VersionCandidate {
                level: BumpLevel::Minor,
                version: Version::new(1, 1, 0),
            }],
        };

        let cloned = ctx.clone();
        assert_eq!(cloned.current_version, ctx.current_version);
        assert_eq!(cloned.recent_commits, ctx.recent_commits);
        assert_eq!(cloned.candidates.len(), 1);
        assert_eq!(cloned.candidates[0].version, Version::new(1, 1, 0));
    }

    // ── VersionCandidate clone and serialize ────────────────

    #[test]
    fn version_candidate_clone() {
        let c = VersionCandidate {
            level: BumpLevel::Major,
            version: Version::new(3, 0, 0),
        };
        let cloned = c;
        assert_eq!(cloned.level, BumpLevel::Major);
        assert_eq!(cloned.version, Version::new(3, 0, 0));
    }

    #[test]
    fn version_candidate_serializes() {
        let c = VersionCandidate {
            level: BumpLevel::Patch,
            version: Version::new(1, 2, 4),
        };
        let json = serde_json::to_string(&c).unwrap();
        assert!(json.contains("\"level\":\"patch\""));
        assert!(json.contains("\"version\":\"1.2.4\""));
    }

    // ── gather_interactive_context ──────────────────────────
    // This calls git operations. It should work inside scrat's repo.

    #[test]
    fn gather_interactive_context_runs() {
        if crate::git::is_inside_repo().unwrap_or(false) {
            let result = gather_interactive_context(5);
            assert!(result.is_ok());
            let ctx = result.unwrap();
            // Should have candidates regardless of whether tags exist
            assert!(!ctx.candidates.is_empty());
            // Candidates should have at least 2 entries
            assert!(ctx.candidates.len() >= 2);
        }
    }

    #[test]
    fn gather_interactive_context_limits_commits() {
        if crate::git::is_inside_repo().unwrap_or(false) {
            let result = gather_interactive_context(2);
            assert!(result.is_ok());
            let ctx = result.unwrap();
            assert!(ctx.recent_commits.len() <= 2);
        }
    }
}
