//! Lockfile diff parser for Python's `uv.lock`.
//!
//! `uv.lock` currently uses the same TOML `[[package]]` format as
//! `Cargo.lock`, so this parser literally delegates to
//! [`super::rust::RustLockfileParser`]. This is NOT a commitment to a
//! shared "TOML package diff" abstraction — it's an incidental format
//! match. If uv diverges from Cargo's lockfile format in a future
//! release, this module grows its own state machine and stops
//! delegating. Do not extract a shared TOML-package-diff helper on
//! the assumption that Python and Rust will always share an
//! implementation.

use super::{LockfileDiffParser, rust::RustLockfileParser};
use crate::pipeline::DepChange;

/// Lockfile diff parser for Python's `uv.lock`.
///
/// Delegates to [`RustLockfileParser`] because `uv.lock` currently uses
/// the same TOML `[[package]]` format as `Cargo.lock`. See the module
/// doc comment for the rationale behind this intentional delegation.
pub struct PythonLockfileParser;

impl LockfileDiffParser for PythonLockfileParser {
    fn parse_diff(&self, diff: &str) -> Vec<DepChange> {
        RustLockfileParser.parse_diff(diff)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_uv_lock_diff_update() {
        // Identical to Cargo.lock format
        let diff = r#"
 [[package]]
 name = "requests"
-version = "2.31.0"
+version = "2.32.0"
 source = { registry = "https://pypi.org/simple" }
"#;
        let changes = PythonLockfileParser.parse_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "requests");
        assert_eq!(changes[0].from.as_deref(), Some("2.31.0"));
        assert_eq!(changes[0].to.as_deref(), Some("2.32.0"));
    }

    #[test]
    fn parse_uv_lock_diff_added() {
        let diff = r#"
+[[package]]
+name = "new-dep"
+version = "1.0.0"
"#;
        let changes = PythonLockfileParser.parse_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].from, None);
        assert_eq!(changes[0].to.as_deref(), Some("1.0.0"));
    }

    #[test]
    fn parse_uv_lock_diff_skips_header() {
        // uv.lock has file-level version/requires-python before [[package]]
        let diff = r#"
-version = 1
+version = 2
 requires-python = ">=3.14"
 [[package]]
 name = "foo"
-version = "1.0.0"
+version = "1.1.0"
"#;
        let changes = PythonLockfileParser.parse_diff(diff);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].name, "foo");
    }
}
