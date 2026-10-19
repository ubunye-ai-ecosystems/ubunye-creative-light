#!/usr/bin/env bash
# Ubunye Light for Linux with ThabangVision collaboration (kopanod) inside every package:
# /usr/bin/kopanod in the .deb, .rpm and .tar.gz, usr/bin/kopanod in the AppImage. On first
# launch Light makes the device key, adds a systemd user unit (start at sign-in) and starts it
# (apps/lightcraft/src/ubunye_collab.rs). Removing the .deb or .rpm stops it and removes the
# unit (prerm.sh).
#
# Usage: ubunye/packaging/linux/collab-package.sh <kopano lab dir> [package.sh arguments...]
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../../.." && pwd)"
LAB="${1:?kopano lab dir}"
shift
OUT="${KOPANO_TARGET_DIR:-$LAB/target}/light-collab/linux"
rm -rf "$OUT"
"$ROOT/ubunye/packaging/build-kopanod.sh" "$LAB" "$OUT"
UBUNYE_EXTRA_BIN_DIR="$OUT" UBUNYE_LINUX_PREREMOVE="$HERE/prerm.sh" "$ROOT/packaging/linux/package.sh" "$@"
