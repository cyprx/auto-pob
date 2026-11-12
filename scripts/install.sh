#!/usr/bin/env bash
# Installs auto-pob from an extracted release tarball.
#
#   ./install.sh [--prefix DIR] [--register-scheme] [--uninstall]
#
# Copies bin/auto-pob, share/applications/auto-pob.desktop and the 256x256 icon into DIR
# (default $HOME/.local) and prints every path it writes. Nothing else is touched.
#
# Registering the pob:// link handler is opt-in. Only --register-scheme adds
# MimeType=x-scheme-handler/pob; to the installed desktop file, runs
# `xdg-mime default auto-pob.desktop x-scheme-handler/pob` and runs update-desktop-database.
# --uninstall removes exactly the three files this script installs (it doesn't undo an
# xdg-mime association or remove directories).
set -euo pipefail

here=$(dirname "$(readlink -f "${BASH_SOURCE[0]}")")
prefix=${HOME:?HOME is not set}/.local
register=0
uninstall=0

usage() { echo "usage: install.sh [--prefix DIR] [--register-scheme] [--uninstall]" >&2; }
while [[ $# -gt 0 ]]; do
    case $1 in
        --prefix)
            [[ $# -ge 2 && -n $2 ]] || { usage; exit 2; }
            prefix=$2
            shift 2
            ;;
        --register-scheme) register=1; shift ;;
        --uninstall) uninstall=1; shift ;;
        -h | --help) usage; exit 0 ;;
        *) echo "install.sh: unknown argument '$1'" >&2; usage; exit 2 ;;
    esac
done

bin_dst=$prefix/bin/auto-pob
desktop_dst=$prefix/share/applications/auto-pob.desktop
icon_dst=$prefix/share/icons/hicolor/256x256/apps/auto-pob.png

if [[ $uninstall -eq 1 ]]; then
    for f in "$bin_dst" "$desktop_dst" "$icon_dst"; do
        if [[ -e $f || -L $f ]]; then
            rm -f -- "$f"
            echo "removed $f"
        fi
    done
    exit 0
fi

bin_src=$here/bin/auto-pob
desktop_src=$here/share/applications/auto-pob.desktop
icon_src=$here/share/icons/hicolor/256x256/apps/auto-pob.png
for f in "$bin_src" "$desktop_src" "$icon_src"; do
    [[ -f $f ]] || { echo "install.sh: $f is missing; run this from an extracted release tarball" >&2; exit 1; }
done

install -D -m 755 "$bin_src" "$bin_dst"
echo "wrote $bin_dst"

install -D -m 644 "$desktop_src" "$desktop_dst"
# the packaged file has no MimeType line (some desktops read that line alone as a scheme
# registration); --register-scheme adds it to the installed copy only
sed -i '/^MimeType=/d' "$desktop_dst"
if [[ $register -eq 1 ]]; then
    printf 'MimeType=x-scheme-handler/pob;\n' >>"$desktop_dst"
fi
echo "wrote $desktop_dst"

install -D -m 644 "$icon_src" "$icon_dst"
echo "wrote $icon_dst"

if [[ $register -eq 1 ]]; then
    if command -v xdg-mime >/dev/null 2>&1; then
        xdg-mime default auto-pob.desktop x-scheme-handler/pob
        echo "ran: xdg-mime default auto-pob.desktop x-scheme-handler/pob (writes your mimeapps.list)"
    else
        echo "xdg-mime not found: skipped registering the pob:// handler"
    fi
    if command -v update-desktop-database >/dev/null 2>&1; then
        update-desktop-database "$prefix/share/applications"
        echo "ran: update-desktop-database $prefix/share/applications"
    else
        echo "update-desktop-database not found: skipped"
    fi
fi

case ":$PATH:" in
    *":$prefix/bin:"*) ;;
    *) echo "note: $prefix/bin is not on your PATH" ;;
esac
