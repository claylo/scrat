//! Python ecosystem detection.
//!
//! Probes `PATH` for `uv`, `pytest`, `python`/`python3`, and `twine`.
//! Version bumping is done directly on `pyproject.toml` (when a
//! `[project] version` exists), so `bump_cmd` is `None`.

use camino::Utf8Path;
use tracing::debug;

use super::has_binary;
use crate::ecosystem::{DetectedTools, Ecosystem, ProjectDetection, VersionStrategy};

/// Detect Python project tooling and build a [`ProjectDetection`].
pub(super) fn detect_python(
    _project_root: &Utf8Path,
    version_strategy: VersionStrategy,
) -> ProjectDetection {
    let has_uv = has_binary("uv");
    let has_pytest = has_binary("pytest");
    let has_python = has_binary("python3") || has_binary("python");
    let has_twine = has_binary("twine");
    debug!(
        has_uv,
        has_pytest, has_python, has_twine, "probed Python tools"
    );

    let test_cmd = if has_uv {
        "uv run pytest".into()
    } else if has_pytest {
        "pytest".into()
    } else {
        String::new()
    };
    let build_cmd = if has_uv {
        "uv build".into()
    } else if has_python {
        "python -m build".into()
    } else {
        String::new()
    };
    let publish_cmd = if has_uv {
        Some("uv publish".into())
    } else if has_twine {
        Some("twine upload dist/*".into())
    } else {
        None
    };

    let changelog_tool = version_strategy.changelog_tool();

    ProjectDetection {
        ecosystem: Ecosystem::Python,
        version_strategy,
        tools: DetectedTools {
            test_cmd,
            build_cmd,
            publish_cmd,
            bump_cmd: None, // Python bump is done directly in pyproject.toml
            changelog_tool,
        },
    }
}
