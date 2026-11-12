#!/usr/bin/env bash
# Builds the auto-pob release tarball.
#
#   scripts/package.sh [--version V] [--out DIR]
#
# Runs `cargo build -p auto-pob --release --features vendored-lua` and writes
# DIR/auto-pob-V-linux-x86_64.tar.gz plus DIR/auto-pob-V-linux-x86_64.tar.gz.sha256.
# V defaults to app/Cargo.toml's version; a vX.Y.Z that differs from it is refused.
set -euo pipefail

root=$(dirname "$(dirname "$(readlink -f "${BASH_SOURCE[0]}")")")
cd "$root"

cargo_version=$(sed -n '/^\[package\]/,/^\[/{s/^version = "\(.*\)"$/\1/p}' app/Cargo.toml | head -1)
version=$cargo_version
out=$root/target/dist
while [[ $# -gt 0 ]]; do
    case $1 in
        --version) [[ $# -ge 2 ]] || { echo "package.sh: --version needs a value" >&2; exit 2; }; version=${2#v}; v_arg=$2; shift 2 ;;
        --out) [[ $# -ge 2 ]] || { echo "package.sh: --out needs a value" >&2; exit 2; }; out=$2; shift 2 ;;
        *) echo "usage: scripts/package.sh [--version V] [--out DIR]" >&2; exit 2 ;;
    esac
done
if [[ -z $cargo_version ]]; then
    echo "package.sh: couldn't read the version from app/Cargo.toml" >&2
    exit 1
fi
# a release tag must match the crate version; other versions (0.0.0-test) are free-form
if [[ ${v_arg:-} =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ && $version != "$cargo_version" ]]; then
    echo "package.sh: tag $v_arg differs from app/Cargo.toml's version $cargo_version" >&2
    exit 1
fi
if ! [[ $version =~ ^[0-9A-Za-z.+-]+$ ]]; then
    echo "package.sh: bad version '$version'" >&2
    exit 2
fi

cargo build -p auto-pob --release --features vendored-lua
# the target dir may be redirected with CARGO_TARGET_DIR
target=$(cargo metadata --format-version 1 --no-deps | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')
binary=$target/release/auto-pob
[[ -x $binary ]] || { echo "package.sh: $binary was not built" >&2; exit 1; }

name=auto-pob-$version-linux-x86_64
mkdir -p "$out"
out=$(cd "$out" && pwd)
stage=$(mktemp -d "${TMPDIR:-/tmp}/auto-pob-package.XXXXXX")
trap 'rm -rf "$stage"' EXIT
dir=$stage/$name
mkdir -p "$dir/bin" "$dir/share/applications" "$dir/share/icons/hicolor/256x256/apps"
install -m 755 "$binary" "$dir/bin/auto-pob"
install -m 644 packaging/auto-pob.desktop "$dir/share/applications/auto-pob.desktop"
install -m 644 assets/icon.png "$dir/share/icons/hicolor/256x256/apps/auto-pob.png"
install -m 755 scripts/install.sh "$dir/install.sh"
install -m 644 README.md "$dir/README.md"
install -m 644 LICENSE "$dir/LICENSE"

tarball=$out/$name.tar.gz
tar --owner=0 --group=0 --numeric-owner --sort=name -C "$stage" -czf "$tarball" "$name"
(cd "$out" && sha256sum "$name.tar.gz" >"$name.tar.gz.sha256")
echo "package.sh: wrote $tarball"
echo "package.sh: wrote $tarball.sha256"
