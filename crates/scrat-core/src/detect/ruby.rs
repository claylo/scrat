//! Ruby ecosystem detection.
//!
//! Probes `PATH` for `bundle`, `rake`, and `gem`. Version bumping is
//! handled via `lib/**/version.rb` and gemspec walkers in the bump
//! module, so `bump_cmd` is `None`.

use camino::Utf8Path;
use tracing::debug;

use super::has_binary;
use crate::ecosystem::{DetectedTools, Ecosystem, ProjectDetection, VersionStrategy};

/// Detect Ruby project tooling and build a [`ProjectDetection`].
pub(super) fn detect_ruby(
    _project_root: &Utf8Path,
    version_strategy: VersionStrategy,
) -> ProjectDetection {
    let has_bundle = has_binary("bundle");
    let has_rake = has_binary("rake");
    let has_gem = has_binary("gem");
    debug!(has_bundle, has_rake, has_gem, "probed Ruby tools");

    let test_cmd = if has_bundle && has_rake {
        "bundle exec rake test".into()
    } else if has_rake {
        "rake test".into()
    } else {
        String::new()
    };
    let build_cmd = if has_gem {
        "gem build".into()
    } else {
        String::new()
    };
    let publish_cmd = has_gem.then(|| "gem push".to_string());

    let changelog_tool = version_strategy.changelog_tool();

    ProjectDetection {
        ecosystem: Ecosystem::Ruby,
        version_strategy,
        tools: DetectedTools {
            test_cmd,
            build_cmd,
            publish_cmd,
            bump_cmd: None, // handled via lib/**/version.rb + gemspec
            changelog_tool,
        },
    }
}
