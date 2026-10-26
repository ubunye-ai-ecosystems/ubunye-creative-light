#!/usr/bin/env bash
# Ubunye Light for macOS with ThabangVision collaboration (kopanod) inside the app:
# Ubunye Light.app/Contents/MacOS/kopanod, signed with the app (inside out) by
# packaging/macos/package.sh, and its version in Contents/Resources/kopanod.version.
#
# kopanod is shared by Ubunye Light, Photo and Film and a Mac runs one: the LaunchAgent
# (org.ubunye.kopano, start at sign-in) points at the kopanod of one app. An app repoints it only
# when that file is gone or has an older kopanod.version than its own
# (apps/lightcraft/src/ubunye_collab.rs), so apps never take it from each other in turn. The
# agent keeps kopanod alive only while that file exists: after the app is moved to the Trash it is
# not started again, and the next Ubunye app opened repoints it.
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
OUT="${KOPANO_TARGET_DIR:-$LAB/target}/lightcraft-collab/macos-$ARCH"
rm -rf "$OUT"
"$ROOT/ubunye/packaging/build-kopanod.sh" "$LAB" "$OUT/bin" --arch "$ARCH"
mkdir -p "$OUT/resources"
"$ROOT/ubunye/packaging/kopanod-version.sh" "$LAB" >"$OUT/resources/kopanod.version"
echo "kopanod $(cat "$OUT/resources/kopanod.version")"
UBUNYE_EXTRA_BIN_DIR="$OUT/bin" UBUNYE_EXTRA_RESOURCES_DIR="$OUT/resources" "$ROOT/packaging/macos/package.sh" "$@"
