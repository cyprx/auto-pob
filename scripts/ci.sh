#!/usr/bin/env bash
# The CI "check" steps: fmt, clippy, workspace tests, the installer test, and shellcheck /
# actionlint when installed. The workflows and scripts/ci-local.sh both run this.
set -euo pipefail

root=$(dirname "$(dirname "$(readlink -f "${BASH_SOURCE[0]}")")")
cd "$root"

cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
# the gate's probe counters exist only with this feature (scripts/gate.sh builds with it)
cargo clippy --workspace --all-targets --features auto-pob/gate-probes -- -D warnings
cargo test --workspace
scripts/test-install.sh

if command -v shellcheck >/dev/null 2>&1; then
    shellcheck scripts/*.sh
else
    echo "SKIPPED: shellcheck not installed"
fi
if command -v actionlint >/dev/null 2>&1; then
    actionlint
else
    echo "SKIPPED: actionlint not installed"
fi
echo "ci.sh: ok"
