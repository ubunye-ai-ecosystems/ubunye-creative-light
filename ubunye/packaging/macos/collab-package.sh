#!/usr/bin/env bash
# Ubunye Light for macOS with ThabangVision collaboration (kopanod) inside the app:
# Ubunye Light.app/Contents/MacOS/kopanod, signed with the app (inside out) by
# packaging/macos/package.sh. On first launch Light makes the device key, adds a LaunchAgent
# (start at sign-in) and starts it (apps/lightcraft/src/ubunye_collab.rs).
#
# Usage: ubunye/packaging/macos/collab-package.sh <kopano lab dir> [package.sh arguments...]
# Signing and notarization: the same environment as packaging/macos/package.sh (none = ad-hoc).
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../../.." && pwd)"
LAB="${1:?kopano lab dir}"
shift
ARCH=universal
args=("$@")
for i in "${!args[@]}"; do
  if [ "${args[$i]}" = --arch ]; then ARCH="${args[$((i + 1))]}"; fi
done
OUT="${KOPANO_TARGET_DIR:-$LAB/target}/light-collab/macos-$ARCH"
rm -rf "$OUT"
"$ROOT/ubunye/packaging/build-kopanod.sh" "$LAB" "$OUT" --arch "$ARCH"
UBUNYE_EXTRA_BIN_DIR="$OUT" "$ROOT/packaging/macos/package.sh" "$@"
