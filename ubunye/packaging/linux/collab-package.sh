#!/usr/bin/env bash
# Ubunye Light for Linux with ThabangVision collaboration (kopanod) inside every package.
#
# kopanod is shared by Ubunye Light, Photo and Film, and a computer runs one:
# - .deb / .rpm: this app's copy goes to /usr/lib/lightcraft/kopanod with the start-at-sign-in
#   user unit; /usr/bin/kopanod and /usr/lib/systemd/user/thabangvision-collab.service are
#   update-alternatives links to the newest copy installed (postinst.sh.in, prerm.sh.in). So
#   the packages never clash over a file, removing one app falls back to another's copy, and
#   removing the last one stops kopanod and its start at sign-in. Data folders stay.
# - .AppImage and .tar.gz: usr/bin/kopanod beside the app; Light sets start at sign-in up itself
#   (apps/lightcraft/src/ubunye_collab.rs), or uses an installed /usr/bin/kopanod when there is one.
#
# Usage: ubunye/packaging/linux/collab-package.sh <kopano lab dir> [package.sh arguments...]
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../../.." && pwd)"
APP=lightcraft
LAB="${1:?kopano lab dir}"
shift
OUT="${KOPANO_TARGET_DIR:-$LAB/target}/$APP-collab/linux"
rm -rf "$OUT"
"$ROOT/ubunye/packaging/build-kopanod.sh" "$LAB" "$OUT/bin"
KV="$("$ROOT/ubunye/packaging/kopanod-version.sh" "$LAB")"
IFS=. read -r a b c n <<<"$KV"
PRIORITY=$(( ((a * 100 + b) * 100 + c) * 10000 + n ))
echo "kopanod $KV: alternatives priority $PRIORITY"

mkdir -p "$OUT/pkglib" "$OUT/scripts"
cp "$HERE/thabangvision-collab.service" "$OUT/pkglib/"
fill() { sed -e "s|@APP@|$APP|g" -e "s|@PRIORITY@|$PRIORITY|g" -e "s|@KOPANOD_VERSION@|$KV|g" "$1"; }
# postinst: the alternatives, then the app's own (desktop and icon caches; it ends with exit 0)
{ fill "$HERE/postinst.sh.in"; tail -n +2 "$ROOT/packaging/linux/postinst.sh"; } >"$OUT/scripts/postinst.sh"
fill "$HERE/prerm.sh.in" >"$OUT/scripts/prerm.sh"
chmod 755 "$OUT/scripts/"*.sh
sh -n "$OUT/scripts/postinst.sh"
sh -n "$OUT/scripts/prerm.sh"

UBUNYE_EXTRA_BIN_DIR="$OUT/bin" UBUNYE_LINUX_PKGLIB_DIR="$OUT/pkglib" \
  UBUNYE_LINUX_POSTINSTALL="$OUT/scripts/postinst.sh" UBUNYE_LINUX_PREREMOVE="$OUT/scripts/prerm.sh" \
  "$ROOT/packaging/linux/package.sh" "$@"
