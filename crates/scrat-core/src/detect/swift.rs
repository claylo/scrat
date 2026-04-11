//! Swift / SwiftPM ecosystem detection.
//!
//! Probes `PATH` for `swift`. SwiftPM has no publish step — versioning
//! lives in git tags — so `publish_cmd` and `bump_cmd` are both `None`.

use camino::Utf8Path;
use tracing::debug;

use super::has_binary;
use crate::ecosystem::{DetectedTools, Ecosystem, ProjectDetection, VersionStrategy};

/// Detect Swift project tooling and build a [`ProjectDetection`].
pub(super) fn detect_swift(
    _project_root: &Utf8Path,
    version_strategy: VersionStrategy,
) -> ProjectDetection {
    let has_swift = has_binary("swift");
    debug!(has_swift, "probed Swift tools");

    let changelog_tool = version_strategy.changelog_tool();

    ProjectDetection {
        ecosystem: Ecosystem::Swift,
        version_strategy,
        tools: DetectedTools {
            test_cmd: if has_swift {
                "swift test".into()
            } else {
                String::new()
            },
            build_cmd: if has_swift {
                "swift build -c release".into()
            } else {
                String::new()
            },
            publish_cmd: None, // SwiftPM publishes via git tags
            bump_cmd: None,
            changelog_tool,
        },
    }
}
