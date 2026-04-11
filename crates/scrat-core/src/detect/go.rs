//! Go ecosystem detection.
//!
//! Probes `PATH` for `go`. Go modules have no publish step — versioning
//! lives entirely in git tags — so `publish_cmd` and `bump_cmd` are both
//! `None`.

use camino::Utf8Path;
use tracing::debug;

use super::has_binary;
use crate::ecosystem::{DetectedTools, Ecosystem, ProjectDetection, VersionStrategy};

/// Detect Go project tooling and build a [`ProjectDetection`].
pub(super) fn detect_go(
    _project_root: &Utf8Path,
    version_strategy: VersionStrategy,
) -> ProjectDetection {
    let has_go = has_binary("go");
    debug!(has_go, "probed Go tools");

    let changelog_tool = version_strategy.changelog_tool();

    ProjectDetection {
        ecosystem: Ecosystem::Go,
        version_strategy,
        tools: DetectedTools {
            test_cmd: if has_go {
                "go test ./...".into()
            } else {
                String::new()
            },
            build_cmd: if has_go {
                "go build ./...".into()
            } else {
                String::new()
            },
            publish_cmd: None,
            bump_cmd: None, // Go modules version lives in git tags
            changelog_tool,
        },
    }
}
