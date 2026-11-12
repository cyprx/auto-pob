#!/usr/bin/env bash
# Tests scripts/install.sh in a temp root: install, install --register-scheme, uninstall.
# xdg-mime and update-desktop-database are stubs on PATH that record their calls. Nothing is
# written outside the temp root, and the test checks that.
set -euo pipefail

root=$(dirname "$(dirname "$(readlink -f "${BASH_SOURCE[0]}")")")
tmp=$(mktemp -d "${TMPDIR:-/tmp}/auto-pob-install-test.XXXXXX")
trap 'rm -rf "$tmp"' EXIT

fails=0
fail() { echo "FAIL: $*" >&2; fails=$((fails + 1)); }
check() { # description, command...
    local what=$1
    shift
    "$@" || fail "$what"
}
mode_of() { stat -c '%a' "$1"; }

# A release layout (R26) with a fake binary: the real installer, desktop file and icon.
stage=$tmp/stage
mkdir -p "$stage/bin" "$stage/share/applications" "$stage/share/icons/hicolor/256x256/apps"
printf '#!/bin/sh\necho fake auto-pob\n' >"$stage/bin/auto-pob"
chmod 755 "$stage/bin/auto-pob"
cp "$root/packaging/auto-pob.desktop" "$stage/share/applications/auto-pob.desktop"
cp "$root/assets/icon.png" "$stage/share/icons/hicolor/256x256/apps/auto-pob.png"
if [[ -f "$root/scripts/install.sh" ]]; then
    cp "$root/scripts/install.sh" "$stage/install.sh"
    chmod 755 "$stage/install.sh"
else
    fail "scripts/install.sh does not exist"
fi

# stubs
home=$tmp/home
mkdir -p "$home" "$tmp/bin" "$tmp/xdg-data" "$tmp/xdg-config"
calls=$tmp/calls.log
for stub in xdg-mime update-desktop-database; do
    printf '#!/bin/sh\necho "%s $*" >>"%s"\n' "$stub" "$calls" >"$tmp/bin/$stub"
    chmod 755 "$tmp/bin/$stub"
done
run_install() { # args...
    env HOME="$home" XDG_DATA_HOME="$tmp/xdg-data" XDG_CONFIG_HOME="$tmp/xdg-config" \
        PATH="$tmp/bin:$PATH" "$stage/install.sh" "$@"
}

prefix=$home/.local
bin=$prefix/bin/auto-pob
desktop=$prefix/share/applications/auto-pob.desktop
icon=$prefix/share/icons/hicolor/256x256/apps/auto-pob.png

check_installed() {
    check "$bin is missing" test -f "$bin"
    check "$bin should be mode 755, is $(mode_of "$bin" 2>/dev/null)" test "$(mode_of "$bin" 2>/dev/null)" = 755
    check "$desktop is missing" test -f "$desktop"
    check "$desktop should be mode 644" test "$(mode_of "$desktop" 2>/dev/null)" = 644
    check "$icon is missing" test -f "$icon"
    check "$icon should be mode 644" test "$(mode_of "$icon" 2>/dev/null)" = 644
    check "the installed binary is not the staged one" cmp -s "$bin" "$stage/bin/auto-pob"
    check "the installed icon is not the staged one" cmp -s "$icon" "$stage/share/icons/hicolor/256x256/apps/auto-pob.png"
    check "Exec line should be 'Exec=auto-pob %u'" grep -qx 'Exec=auto-pob %u' "$desktop"
    if command -v desktop-file-validate >/dev/null; then
        check "desktop-file-validate rejects $desktop" desktop-file-validate "$desktop"
    else
        echo "SKIPPED: desktop-file-validate not installed"
    fi
}

tree() { (cd "$tmp" && find . -mindepth 1 -not -path './stage/*' -not -path './stage' \
    -not -path './bin/*' -not -path './bin' -not -name calls.log | sort); }
before=$(tree)

# 1. plain install
out=$(run_install)
echo "$out"
check_installed
check "plain install must not call xdg-mime or update-desktop-database" test ! -s "$calls"
check "plain install must not write a MimeType line" bash -c "! grep -q '^MimeType' '$desktop'"
for p in "$bin" "$desktop" "$icon"; do
    check "install did not print $p" grep -qF "$p" <<<"$out"
done
# every path under the temp root that the install created is one it printed
installed_files=$(cd "$tmp" && find home -type f | sort)
expected_files=$(printf '%s\n' "home/.local/bin/auto-pob" "home/.local/share/applications/auto-pob.desktop" \
    "home/.local/share/icons/hicolor/256x256/apps/auto-pob.png" | sort)
check "install wrote other files than the three: $installed_files" test "$installed_files" = "$expected_files"

# 2. install --register-scheme
: >"$calls"
out=$(run_install --register-scheme)
echo "$out"
check_installed
mime_lines=$(grep -c '^MimeType' "$desktop" || true)
check "expected exactly one MimeType line, got $mime_lines" test "$mime_lines" = 1
check "MimeType should be exactly 'MimeType=x-scheme-handler/pob;'" grep -qxF 'MimeType=x-scheme-handler/pob;' "$desktop"
check "xdg-mime was not called as expected: $(cat "$calls")" grep -qxF 'xdg-mime default auto-pob.desktop x-scheme-handler/pob' "$calls"
check "update-desktop-database was not called on the prefix" grep -qxF "update-desktop-database $prefix/share/applications" "$calls"
check "unexpected extra stub calls: $(cat "$calls")" test "$(wc -l <"$calls")" = 2

# 3. re-running plain install goes back to no MimeType
: >"$calls"
run_install >/dev/null
check "a plain re-install must drop the MimeType line" bash -c "! grep -q '^MimeType' '$desktop'"
check "a plain re-install must not call the stubs" test ! -s "$calls"

# 4. a custom --prefix
alt=$tmp/alt-prefix
run_install --prefix "$alt" >/dev/null
check "--prefix install is missing the binary" test -x "$alt/bin/auto-pob"

# 5. uninstall removes what was written and nothing else
: >"$calls"
mkdir -p "$prefix/bin"
echo keep >"$prefix/bin/other-tool"
out=$(run_install --uninstall)
echo "$out"
run_install --prefix "$alt" --uninstall >/dev/null
for p in "$bin" "$desktop" "$icon" "$alt/bin/auto-pob" "$alt/share/applications/auto-pob.desktop"; do
    check "uninstall left $p" test ! -e "$p"
done
check "uninstall removed an unrelated file" test "$(cat "$prefix/bin/other-tool")" = keep
check "uninstall must not call the stubs" test ! -s "$calls"
run_install --uninstall >/dev/null || fail "a second uninstall should succeed"
rm -f "$prefix/bin/other-tool"
leftover=$(cd "$tmp" && find home alt-prefix -type f | sort)
check "files left after uninstall: $leftover" test -z "$leftover"

# 6. nothing outside the temp root: the XDG dirs and the stage were never written
check "install wrote into XDG_DATA_HOME" test -z "$(find "$tmp/xdg-data" -mindepth 1)"
check "install wrote into XDG_CONFIG_HOME" test -z "$(find "$tmp/xdg-config" -mindepth 1)"
after=$(tree | grep -v -e '^./home' -e '^./alt-prefix' || true)
check "unexpected entries in the temp root: $after" test "$after" = "$(printf '%s\n' "$before" | grep -v -e '^./home' -e '^./alt-prefix' || true)"

# 7. bad arguments fail and write nothing
if run_install --bogus >/dev/null 2>&1; then fail "--bogus should be rejected"; fi
check "a rejected run wrote files" test -z "$(find "$home" -type f)"

if [[ $fails -ne 0 ]]; then
    echo "test-install.sh: $fails check(s) failed" >&2
    exit 1
fi
echo "test-install.sh: ok"
