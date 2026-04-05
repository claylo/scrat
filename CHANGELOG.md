## [0.1.0] - 2026-04-05

### 🚀 Features

- Add ecosystem detection and extended config model (#2)
- Add preflight, version, git, and bump modules (M2) (#3)
- Add ship orchestrator, hook executor, and inline image rendering (M3) (#4)
- Add confirm-by-default for `scrat ship` and git-cliff version check (#5)
- Add pipeline context model for structured release data (M4 #1) (#6)
- Add built-in deps diff for lockfile change detection (M4 #2) (#7)
- Add built-in stats collection for release statistics (M4 #3) (#8)
- Add `filter:` hook prefix for JSON pipeline mutation (M4 #4) (#9)
- Add release notes via git-cliff context injection (M4 #5) (#10)
- Configurable gh release and systematic --no-* flags (M4 #6) (#11)
- Add Generic ecosystem variant with interactive selection (#12)
- Add Go and PHP ecosystem support (#18)
- Add Python, Ruby, and Swift ecosystem support (#19)
- *(preflight)* Validate credentials and tag before mutation (#25)
- *(notes)* Inject repo name into release notes extra context (#27)
- *(config)* Add no_* phase-skip flags to [ship] config section (#28)

### 🐛 Bug Fixes

- Correct release notes rendering bugs (#13)
- *(ship)* Move publish phase after git and release (#21)
- *(cli)* Support variables in asset paths, improve shipit output (#22)
- Clean up release notes (#26)
- *(preflight)* Respect [ship] config flags in standalone preflight (#30)

### 🧪 Testing

- Expand coverage from 215 to 456 tests (#17)

### ⚙️ Miscellaneous Tasks

- First commit
- Fix template defaults (#1)
- Cherry-pick template updates from claylo-rs beta.2 (#14)
- Update template (#15)
- Merge claylo-rs template v1.1.0 (#16)
- Update keywords and categories for crates.io
- Update gitignore
- Update to claylo-rs 1.2.0 (#20)
- Format fixes (#23)
- Update to claylo-rs 1.3.0 (#24)
- Coda config
- Update workflows based on what scrat will do (#29)
