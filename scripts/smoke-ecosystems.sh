#!/usr/bin/env bash
# End-to-end smoke test for every ecosystem scrat claims to support.
#
# Builds tiny throwaway projects in /tmp/scrat-smoke/<ecosystem>/, exercises
# the four user-facing entry points (info, preflight, notes, bump) against
# each, and reports PASS/FAIL per stage.
#
# Each fixture is a git repo with two commits — the first introduces a
# marker file, lockfile, and `.scrat.toml`, the second changes the lockfile
# so the deps-diff parser has something real to detect. The first commit
# is tagged `v0.1.0` so `git tag --list "v*"` returns it.
#
# Usage:
#   scripts/smoke-ecosystems.sh                    # use ./target/debug/scrat
#   SCRAT=/path/to/scrat scripts/smoke-ecosystems.sh
#
# Exits 0 if every check passes, 1 otherwise.

set -uo pipefail

# Resolve the scrat binary: env override first, then ./target/debug/scrat
# from the repo root (script lives in scripts/, so .. is the root).
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
SCRAT="${SCRAT:-$REPO_ROOT/target/debug/scrat}"
SMOKE="/tmp/scrat-smoke"
RESULTS=()

if [[ ! -x "$SCRAT" ]]; then
  echo "scrat binary not found at: $SCRAT" >&2
  echo "  build it first with: cargo build -p scrat" >&2
  echo "  or pass SCRAT=/path/to/scrat to override" >&2
  exit 1
fi

red()    { printf '\033[31m%s\033[0m' "$*"; }
green()  { printf '\033[32m%s\033[0m' "$*"; }
yellow() { printf '\033[33m%s\033[0m' "$*"; }

setup_repo() {
  local dir="$1"
  rm -rf "$dir"
  mkdir -p "$dir"
  (cd "$dir" && \
    git init -q && \
    git config user.email "smoke@test.local" && \
    git config user.name "Smoke Test" && \
    git config commit.gpgsign false)
}

# Drop a `.scrat.toml` into every fixture so standalone preflight skips
# the gh/registry-auth checks (those depend on the user's environment,
# not the project under test).
write_config() {
  local dir="$1"
  cat > "$dir/.scrat.toml" <<'EOF'
[ship]
no_release = true
no_publish = true
no_fetch = true
EOF
}

commit_and_tag() {
  local dir="$1"
  (cd "$dir" && git add -A && git commit -q -m "initial" && git tag v0.1.0)
}

second_commit() {
  local dir="$1"
  (cd "$dir" && git add -A && git commit -q -m "feat: bump deps")
}

run_check() {
  local label="$1" dir="$2" cmd="$3"
  local out
  out=$(cd "$dir" && eval "$cmd" 2>&1)
  local rc=$?
  if [[ $rc -eq 0 ]]; then
    echo "  $(green PASS) $label"
    RESULTS+=("PASS: $dir | $label")
    return 0
  else
    echo "  $(red FAIL) $label (rc=$rc)"
    echo "$out" | head -10 | sed 's/^/    /'
    RESULTS+=("FAIL: $dir | $label | rc=$rc")
    return 1
  fi
}

# ───────────────────────────────────────────────────────
# Ecosystem fixtures
# ───────────────────────────────────────────────────────

setup_rust() {
  local dir="$SMOKE/rust"
  setup_repo "$dir"
  cat > "$dir/Cargo.toml" <<'EOF'
[package]
name = "smoke-rust"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "smoke-rust"
path = "src/main.rs"

[dependencies]
serde = "1.0.0"
EOF
  mkdir -p "$dir/src"
  echo 'fn main() {}' > "$dir/src/main.rs"
  cat > "$dir/Cargo.lock" <<'EOF'
[[package]]
name = "serde"
version = "1.0.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
EOF
  write_config "$dir"
  commit_and_tag "$dir"
  cat > "$dir/Cargo.lock" <<'EOF'
[[package]]
name = "serde"
version = "1.0.5"
source = "registry+https://github.com/rust-lang/crates.io-index"
EOF
  second_commit "$dir"
  echo "$dir"
}

setup_node() {
  local dir="$SMOKE/node"
  setup_repo "$dir"
  cat > "$dir/package.json" <<'EOF'
{
  "name": "smoke-node",
  "version": "0.1.0"
}
EOF
  cat > "$dir/package-lock.json" <<'EOF'
{
  "name": "smoke-node",
  "version": "0.1.0",
  "lockfileVersion": 3,
  "packages": {
    "": { "name": "smoke-node", "version": "0.1.0" },
    "node_modules/express": {
      "version": "4.17.1",
      "resolved": "https://registry.npmjs.org/express/-/express-4.17.1.tgz"
    }
  }
}
EOF
  write_config "$dir"
  commit_and_tag "$dir"
  cat > "$dir/package-lock.json" <<'EOF'
{
  "name": "smoke-node",
  "version": "0.1.0",
  "lockfileVersion": 3,
  "packages": {
    "": { "name": "smoke-node", "version": "0.1.0" },
    "node_modules/express": {
      "version": "4.18.2",
      "resolved": "https://registry.npmjs.org/express/-/express-4.18.2.tgz"
    }
  }
}
EOF
  second_commit "$dir"
  echo "$dir"
}

setup_go() {
  local dir="$SMOKE/go"
  setup_repo "$dir"
  cat > "$dir/go.mod" <<'EOF'
module example.com/smoke-go

go 1.21

require (
	github.com/spf13/cobra v1.7.0
)
EOF
  write_config "$dir"
  commit_and_tag "$dir"
  cat > "$dir/go.mod" <<'EOF'
module example.com/smoke-go

go 1.21

require (
	github.com/spf13/cobra v1.8.0
)
EOF
  second_commit "$dir"
  echo "$dir"
}

setup_php() {
  local dir="$SMOKE/php"
  setup_repo "$dir"
  cat > "$dir/composer.json" <<'EOF'
{
  "name": "smoke/php",
  "version": "0.1.0"
}
EOF
  cat > "$dir/composer.lock" <<'EOF'
{
  "packages": [
    {
      "name": "vendor/lib",
      "version": "1.0.0"
    }
  ]
}
EOF
  write_config "$dir"
  commit_and_tag "$dir"
  cat > "$dir/composer.lock" <<'EOF'
{
  "packages": [
    {
      "name": "vendor/lib",
      "version": "1.0.1"
    }
  ]
}
EOF
  second_commit "$dir"
  echo "$dir"
}

setup_python() {
  local dir="$SMOKE/python"
  setup_repo "$dir"
  cat > "$dir/pyproject.toml" <<'EOF'
[project]
name = "smoke-python"
version = "0.1.0"
EOF
  cat > "$dir/uv.lock" <<'EOF'
version = 1
requires-python = ">=3.10"

[[package]]
name = "requests"
version = "2.31.0"
EOF
  write_config "$dir"
  commit_and_tag "$dir"
  cat > "$dir/uv.lock" <<'EOF'
version = 1
requires-python = ">=3.10"

[[package]]
name = "requests"
version = "2.32.0"
EOF
  second_commit "$dir"
  echo "$dir"
}

setup_ruby() {
  local dir="$SMOKE/ruby"
  setup_repo "$dir"
  mkdir -p "$dir/lib/smoke_ruby"
  cat > "$dir/Gemfile" <<'EOF'
source "https://rubygems.org"
gem "rails"
EOF
  cat > "$dir/lib/smoke_ruby/version.rb" <<'EOF'
module SmokeRuby
  VERSION = "0.1.0"
end
EOF
  cat > "$dir/Gemfile.lock" <<'EOF'
GEM
  remote: https://rubygems.org/
  specs:
    rails (7.1.2)

DEPENDENCIES
  rails
EOF
  write_config "$dir"
  commit_and_tag "$dir"
  cat > "$dir/Gemfile.lock" <<'EOF'
GEM
  remote: https://rubygems.org/
  specs:
    rails (7.1.3)

DEPENDENCIES
  rails
EOF
  second_commit "$dir"
  echo "$dir"
}

setup_swift() {
  local dir="$SMOKE/swift"
  setup_repo "$dir"
  cat > "$dir/Package.swift" <<'EOF'
// swift-tools-version:5.5
import PackageDescription
let package = Package(name: "smoke-swift")
EOF
  cat > "$dir/Package.resolved" <<'EOF'
{
  "pins" : [
    {
      "identity" : "swift-nio",
      "kind" : "remoteSourceControl",
      "state" : {
        "version" : "2.92.0"
      }
    }
  ],
  "version" : 2
}
EOF
  write_config "$dir"
  commit_and_tag "$dir"
  cat > "$dir/Package.resolved" <<'EOF'
{
  "pins" : [
    {
      "identity" : "swift-nio",
      "kind" : "remoteSourceControl",
      "state" : {
        "version" : "2.92.1"
      }
    }
  ],
  "version" : 2
}
EOF
  second_commit "$dir"
  echo "$dir"
}

# ───────────────────────────────────────────────────────
# Run the four user-facing entry points against one fixture
# ───────────────────────────────────────────────────────

test_one() {
  local label="$1" dir="$2" expect_eco="$3" version_file="$4"
  echo
  echo "$(yellow "── $label ──")"
  echo "  fixture: $dir"

  run_check "info detects $expect_eco" "$dir" \
    "'$SCRAT' info --json | grep -qF '\"ecosystem\": \"$expect_eco\"'"

  run_check "preflight runs" "$dir" \
    "'$SCRAT' preflight --no-fetch --json >/dev/null"

  run_check "notes renders deps changes" "$dir" \
    "'$SCRAT' notes --version 0.2.0 --from v0.1.0 --json >/dev/null"

  run_check "bump --dry-run" "$dir" \
    "'$SCRAT' bump --version 0.2.0 --no-changelog --dry-run >/dev/null"

  if [[ -n "$version_file" ]]; then
    run_check "bump real edit writes 0.2.0 to $version_file" "$dir" \
      "'$SCRAT' bump --version 0.2.0 --no-changelog >/dev/null && grep -q '0.2.0' '$version_file'"
  else
    echo "  $(yellow SKIP) bump real edit (version lives in git tag for $label)"
  fi
}

main() {
  echo "Smoke testing scrat ecosystems..."
  echo "  binary: $SCRAT"
  rm -rf "$SMOKE"
  mkdir -p "$SMOKE"

  test_one "Rust"   "$(setup_rust)"   "rust"   "Cargo.toml"
  test_one "Node"   "$(setup_node)"   "node"   "package.json"
  test_one "Go"     "$(setup_go)"     "go"     ""
  test_one "PHP"    "$(setup_php)"    "php"    "composer.json"
  test_one "Python" "$(setup_python)" "python" "pyproject.toml"
  test_one "Ruby"   "$(setup_ruby)"   "ruby"   "lib/smoke_ruby/version.rb"
  test_one "Swift"  "$(setup_swift)"  "swift"  ""

  echo
  echo "════════════════════════════════════════════"
  echo "RESULTS"
  echo "════════════════════════════════════════════"
  local pass=0 fail=0
  for r in "${RESULTS[@]}"; do
    if [[ "$r" == PASS:* ]]; then
      pass=$((pass+1))
    else
      fail=$((fail+1))
      echo "  $(red "$r")"
    fi
  done
  echo
  echo "$(green "$pass passed"), $(red "$fail failed")"
  [[ $fail -eq 0 ]]
}

main "$@"
