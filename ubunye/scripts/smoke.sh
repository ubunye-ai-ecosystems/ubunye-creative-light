#!/usr/bin/env bash
# Headless smoke test for ubunye-creative-light:
#   import → edit → save (library) → reopen in a new process → export → verify.
# Usage: smoke-light.sh <path-to-lightcraft-cli> [workdir]
# Verified against upstream lightcraft b70ae17 (2026-10-10) on Linux x86_64 without a GPU.
set -euo pipefail

CLI=${1:?path to lightcraft-cli}
WORK=${2:-$(mktemp -d)}
mkdir -p "$WORK" && cd "$WORK"

# 1. Fixture: a small synthetic RGB PNG (no external assets, so no licence questions).
python3 -I - <<'PY'
import struct, zlib
w, h = 64, 48
rows = b"".join(b"\x00" + b"".join(bytes([(x * 4) % 256, (y * 5) % 256, 128]) for x in range(w)) for y in range(h))
def chunk(t, d): return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xFFFFFFFF)
open("in.png", "wb").write(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
                           + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b""))
PY

rm -rf lib lib0
# 2. Control: import and export with no edit.
"$CLI" run --library lib0 --import in.png app.export path="$WORK/out0.jpg" longEdge=64 >/dev/null
# 3. Import, edit, export; the edit is saved into the library.
"$CLI" run --library lib --import in.png develop.set control=light.exposure value=0.7 \
    app.export path="$WORK/out1.jpg" longEdge=64 >/dev/null
# 4. Reopen the library in a fresh process and export again.
"$CLI" run --library lib app.export path="$WORK/out2.jpg" longEdge=64 >/dev/null

# 5. Assertions.
file out1.jpg | grep -q "JPEG image data" || { echo "FAIL: export is not a JPEG"; exit 1; }
cmp -s out1.jpg out2.jpg || { echo "FAIL: reopened library does not reproduce the edit"; exit 1; }
if cmp -s out0.jpg out1.jpg; then echo "FAIL: edit had no effect"; exit 1; fi
test -f lib/catalog.log || { echo "FAIL: library not persisted"; exit 1; }
echo "PASS: import → edit → save → reopen → export"
