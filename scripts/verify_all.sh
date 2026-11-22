#!/usr/bin/env bash
# Definition-of-done check: every generated file is current, every key and
# grader passes its self-test, every test passes. Exit 0 only if all pass.
# Usage: scripts/verify_all.sh [--fast]   (--fast skips holemap, the slow step)
set -u
cd "$(dirname "$0")/.."

fast=0
[ "${1:-}" = "--fast" ] && fast=1
failed=()

step() {
    local name="$1"; shift
    echo "== $name"
    if "$@"; then
        echo "   PASS"
    else
        echo "   FAIL"
        failed+=("$name")
    fi
}

ext=""
case "$(uname -s)" in MINGW*|MSYS*|CYGWIN*) ext=".exe" ;; esac
T="target/release/tutor$ext"

step "toolchain pinned" bash -c '[ "$(rustc --version | cut -d" " -f2)" = "$(cat RUST_VERSION)" ]'
step "build"            cargo build --release --quiet
step "tutor tests"      cargo test --quiet
step "clippy (tutor)"   cargo clippy --quiet --all-targets -- -D warnings
step "odeint tests"     bash -c 'cd projects/odeint/reference && cargo test --quiet'
step "clippy (odeint)"  bash -c 'cd projects/odeint/reference && cargo clippy --quiet --all-targets -- -D warnings'
step "reference clean"  "$T" check projects/odeint/reference/src projects/odeint/reference/tests
step "scaffold"         "$T" scaffold odeint --check
step "baseline"         "$T" baseline odeint --check
if [ "$fast" = 0 ]; then
    step "holemap"      "$T" holemap odeint --check
fi
step "fixtures"         "$T" fixtures odeint --check
step "lessons"          "$T" lessons verify
step "course"           "$T" course verify

echo
if [ ${#failed[@]} -eq 0 ]; then
    echo "verify_all: PASS"
else
    echo "verify_all: FAIL (${failed[*]})"
    exit 1
fi
