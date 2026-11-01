#!/usr/bin/env bash
# Build kopanod (ThabangVision collaboration, the Kopano service; slice S4) for the Light
# installers on macOS or Linux. Windows has its own: windows/collab-package.ps1.
#
# Usage: ubunye/packaging/build-kopanod.sh <kopano lab dir> <out dir> [--arch universal|aarch64|x86_64]
#
#   <kopano lab dir>  research/kopano/lab of ubunye-creative-studio-tools
#   <out dir>         gets one file, kopanod (macOS: one universal binary unless --arch says
#                     otherwise; it is signed with the app by packaging/macos/package.sh)
# Builds in KOPANO_TARGET_DIR, else <lab>/target.
set -euo pipefail
LAB="${1:?kopano lab dir}"
OUT="${2:?out dir}"
shift 2
ARCH=universal
while [ $# -gt 0 ]; do
  case "$1" in
    --arch) ARCH="$2"; shift 2 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done
[ -f "$LAB/kopano-node/Cargo.toml" ] || { echo "error: $LAB is not research/kopano/lab" >&2; exit 1; }
mkdir -p "$OUT"
OUT="$(cd "$OUT" && pwd)"
export CARGO_TARGET_DIR="${KOPANO_TARGET_DIR:-$(cd "$LAB" && pwd)/target}"
cd "$LAB"
if [ "$(uname -s)" = Darwin ]; then
  export MACOSX_DEPLOYMENT_TARGET=11.0 # as Light (packaging/macos/package.sh)
  case "$ARCH" in
    universal) TARGETS=(aarch64-apple-darwin x86_64-apple-darwin) ;;
    aarch64) TARGETS=(aarch64-apple-darwin) ;;
    x86_64) TARGETS=(x86_64-apple-darwin) ;;
    *) echo "unknown --arch $ARCH" >&2; exit 2 ;;
  esac
  inputs=()
  for t in "${TARGETS[@]}"; do
    cargo build --release --locked -p kopano-node --bin kopanod --target "$t"
    inputs+=("$CARGO_TARGET_DIR/$t/release/kopanod")
  done
  lipo -create -output "$OUT/kopanod" "${inputs[@]}"
  lipo -info "$OUT/kopanod"
  strip -x "$OUT/kopanod"
else
  cargo build --release --locked -p kopano-node --bin kopanod
  install -m755 "$CARGO_TARGET_DIR/release/kopanod" "$OUT/kopanod"
  strip "$OUT/kopanod" 2>/dev/null || true
fi
# it starts and keeps a device key (a throwaway home)
tmp="$(mktemp -d)"
a="$("$OUT/kopanod" init --home "$tmp/k" | tail -1)"
b="$("$OUT/kopanod" id --home "$tmp/k" | tail -1)"
rm -rf "$tmp"
[ ${#a} -eq 64 ] && [ "$a" = "$b" ] || { echo "error: kopanod init/id did not agree ($a / $b)" >&2; exit 1; }
echo "ok kopanod: device id $a"
