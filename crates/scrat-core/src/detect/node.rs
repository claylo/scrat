//! Node.js ecosystem detection.
//!
//! Probes `PATH` for `npm`/`yarn`/`pnpm` and picks a sensible package
//! manager for test/build/publish. The version bump is always a direct
//! `package.json` edit — scrat is not a lockfile manager.

use camino::Utf8Path;
use tracing::debug;

use super::has_binary;
use crate::ecosystem::{DetectedTools, Ecosystem, ProjectDetection, VersionStrategy};

/// Detect Node.js project tooling and build a [`ProjectDetection`].
pub(super) fn detect_node(
    _project_root: &Utf8Path,
    version_strategy: VersionStrategy,
) -> ProjectDetection {
    let has_npm = has_binary("npm");
    let has_yarn = has_binary("yarn");
    let has_pnpm = has_binary("pnpm");
    debug!(has_npm, has_yarn, has_pnpm, "probed Node tools");

    let (test_cmd, build_cmd, publish_cmd) = if has_pnpm {
        (
            "pnpm test".to_string(),
            "pnpm run build".to_string(),
            Some("pnpm publish".to_string()),
        )
    } else if has_yarn {
        (
            "yarn test".to_string(),
            "yarn build".to_string(),
            Some("yarn publish".to_string()),
        )
    } else {
        (
            "npm test".to_string(),
            "npm run build".to_string(),
            has_npm.then(|| "npm publish".to_string()),
        )
    };

    let changelog_tool = version_strategy.changelog_tool();

    ProjectDetection {
        ecosystem: Ecosystem::Node,
        version_strategy,
        tools: DetectedTools {
            test_cmd,
            build_cmd,
            publish_cmd,
            bump_cmd: None, // handled via direct package.json edit
            changelog_tool,
        },
    }
}
