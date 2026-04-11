//! PHP / Composer ecosystem detection.
//!
//! Probes `PATH` for `composer`. PHP version bumping is done directly
//! on `composer.json` (when a `version` field exists), so `bump_cmd` is
//! `None`.

use camino::Utf8Path;
use tracing::debug;

use super::has_binary;
use crate::ecosystem::{DetectedTools, Ecosystem, ProjectDetection, VersionStrategy};

/// Detect PHP/Composer project tooling and build a [`ProjectDetection`].
pub(super) fn detect_php(
    _project_root: &Utf8Path,
    version_strategy: VersionStrategy,
) -> ProjectDetection {
    let has_composer = has_binary("composer");
    debug!(has_composer, "probed PHP tools");

    let changelog_tool = version_strategy.changelog_tool();

    ProjectDetection {
        ecosystem: Ecosystem::Php,
        version_strategy,
        tools: DetectedTools {
            test_cmd: if has_composer {
                "composer test".into()
            } else {
                String::new()
            },
            build_cmd: String::new(),
            publish_cmd: None,
            bump_cmd: None, // PHP bump is done directly in composer.json
            changelog_tool,
        },
    }
}
