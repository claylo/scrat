# Repository Guidelines

## Additional Notes
- Consider the project a monorepo containing multiple crates unless explicitly told otherwise.
- Follow the existing crate/workspace layout. If creating a new workspace, prefer a simple, discoverable structure (e.g. crates under `crates/`).
- Treat `target/` as build output only; do not commit artifacts.
- Capture architectural decisions in Markdown ADRs under `docs/decisions/`.
