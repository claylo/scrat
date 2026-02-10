# GitHub Actions Caching

This repository uses a caching approach adapted from the rust-yaml project
(.github/CACHING.md in their repo). Hat tip to their team for the structure
and cache-key strategy.

## What We Keep from rust-yaml

- Separate caches for:
  - Rust toolchains (rustup)
  - Cargo registry + git dependencies
  - Build artifacts (`target/`)
  - Cargo tools (e.g., `cargo-nextest`)
- Multi-level restore keys to improve cache hit rates.
- Lightweight composite actions to keep workflow YAML small.

## Composite Actions

Located at:

- `.github/actions/setup-rust-cache`
- `.github/actions/setup-cargo-tools`

These mirror rust-yaml's approach and are used in CI to speed up lint, test,
and MSRV checks.

## Cache Keys (Summary)

- **Toolchain cache**: OS + toolchain + toolchain file hash
- **Registry cache**: OS + Cargo.lock hash
- **Build cache**: OS + toolchain + Cargo.lock hash + source hash
- **Tools cache**: OS + arch + tools list + binary hash

## Notes

- Caches are intentionally per-job (lint/test/msrv) to reduce cross-contamination.
- Expect slower first runs; cache hits should improve after the first CI cycle.
