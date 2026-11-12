#!/usr/bin/env bash
# Everything CI does, locally: ci.sh, then the PoB gate, then a test package.
# Needs AUTO_POB_POB_DIR (the pinned PoB folder) for the gate.
set -euo pipefail

root=$(dirname "$(dirname "$(readlink -f "${BASH_SOURCE[0]}")")")
cd "$root"

scripts/ci.sh
scripts/gate.sh -j4
scripts/package.sh --version 0.0.0-test --out target/dist
echo "ci-local.sh: ok"
