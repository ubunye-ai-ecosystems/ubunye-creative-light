#!/usr/bin/env python3
"""Render every app icon file from ubunye/brand/assets/icon.svg (the Ubunye Creative Light icon).

    python ubunye/brand/render_icons.py [--resvg PATH]

Follows packaging/icons.sh: the SVG is the full-bleed 512 tile, used as is for Windows and Linux;
macOS icons follow Apple's grid (824/1024 body with a transparent margin, made by widening the
viewBox to -62 -62 636 636). Needs resvg (cargo install resvg) and Pillow (for .ico and .icns), so
it is separate from apply.py, which is stdlib-only. Writes ubunye/brand/icons.sha256, which
`apply.py --check` uses to tell our rendered icons from upstream's.
"""
from __future__ import annotations

import argparse
import hashlib
import shutil
import subprocess
import sys
import tempfile
import tomllib
from pathlib import Path

from PIL import Image

ROOT = Path(__file__).resolve().parents[2]
SRC = ROOT / "ubunye" / "brand" / "assets" / "icon.svg"
# Simplified icon (fewer, thicker rays, no glow) used at 32 px and below.
SMALL = ROOT / "ubunye" / "brand" / "assets" / "icon-small.svg"
DIR = ROOT / "assets" / "app-icon"
with open(ROOT / "ubunye" / "brand.toml", "rb") as fh:
    APP_ID = tomllib.load(fh)["identifiers"]["app_id"]


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--resvg", default=shutil.which("resvg") or "T:/rust/cargo/bin/resvg.exe")
    args = ap.parse_args()
    svg = SRC.read_text(encoding="utf-8")
    if 'viewBox="0 0 512 512"' not in svg:
        sys.exit(f"{SRC}: expected viewBox=\"0 0 512 512\"")
    written: list[Path] = []

    def out(p: Path) -> Path:
        p.parent.mkdir(parents=True, exist_ok=True)
        written.append(p)
        return p

    with tempfile.TemporaryDirectory() as tmp_s:
        tmp = Path(tmp_s)
        mac = tmp / "macos.svg"
        mac.write_text(svg.replace('viewBox="0 0 512 512"', 'viewBox="-62 -62 636 636"'), encoding="utf-8")

        def render(src: Path, size: int, dst: Path) -> Path:
            subprocess.run([args.resvg, "-w", str(size), "-h", str(size), str(src), str(dst)], check=True)
            return dst

        out(DIR / "lightcraft.svg").write_text(svg, encoding="utf-8", newline="\n")
        small_svg = SMALL.read_text(encoding="utf-8") if SMALL.exists() else svg
        out(DIR / "lightcraft-small.svg").write_text(small_svg, encoding="utf-8", newline="\n")
        out(DIR / "hicolor" / "scalable" / "apps" / f"{APP_ID}.svg").write_text(svg, encoding="utf-8", newline="\n")

        render(SRC, 1024, out(DIR / "lightcraft-1024.png"))
        render(mac, 512, out(DIR / "lightcraft-macos-512.png"))
        for s in (16, 24, 32, 48, 64, 128, 256, 512):
            render(SMALL if s <= 32 and SMALL.exists() else SRC, s, out(DIR / "hicolor" / f"{s}x{s}" / "apps" / f"{APP_ID}.png"))

        ico_sizes = (16, 20, 24, 32, 40, 48, 64, 128, 256)
        ico_imgs = [Image.open(render(SMALL if s <= 32 and SMALL.exists() else SRC, s, tmp / f"ico-{s}.png")).convert("RGBA") for s in ico_sizes]
        ico_imgs[-1].save(out(DIR / "lightcraft.ico"), format="ICO", sizes=[(s, s) for s in ico_sizes],
                          append_images=ico_imgs[:-1])

        icns_sizes = (16, 32, 64, 128, 256, 512, 1024)
        icns_imgs = [Image.open(render(SMALL if s <= 32 and SMALL.exists() else mac, s, tmp / f"icns-{s}.png")).convert("RGBA") for s in icns_sizes]
        icns_imgs[-1].save(out(DIR / "lightcraft.icns"), format="ICNS", append_images=icns_imgs[:-1])

    def file_digest(p: Path) -> str:  # same as apply.py: SVG hashed with LF endings
        data = p.read_bytes()
        if p.suffix.lower() == ".svg":
            data = data.replace(b"\r\n", b"\n")
        return hashlib.sha256(data).hexdigest()

    lines = [f"{file_digest(SRC)}  {SRC.relative_to(ROOT).as_posix()}"]
    if SMALL.exists():
        lines.append(f"{file_digest(SMALL)}  {SMALL.relative_to(ROOT).as_posix()}")
    for p in sorted(written):
        digest = file_digest(p)
        lines.append(f"{digest}  {p.relative_to(ROOT).as_posix()}")
    (ROOT / "ubunye" / "brand" / "icons.sha256").write_text("\n".join(lines) + "\n", encoding="utf-8", newline="\n")
    print(f"rendered {len(written)} icon files into {DIR.relative_to(ROOT).as_posix()}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
