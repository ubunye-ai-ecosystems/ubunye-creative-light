# Ubunye Light app icon

The Ubunye Light icon: Ubunye AI Ecosystems artwork adapted from the ThabangVision logo, with a letter outline from
DM Serif Display (SIL OFL 1.1). It replaces upstream LightCraft's icon in this distribution.

The source is `ubunye/brand/assets/icon.svg` (a full-bleed 512 tile, `viewBox="0 0 512 512"`).
Every file here is generated from it; do not edit them by hand. macOS files (`.icns`,
`lightcraft-macos-512.png`) add Apple's transparent margin (an 824/1024 body); Windows and Linux files
use the full-bleed tile. The file names `lightcraft*` are kept from upstream so packaging scripts and
`include_bytes!` paths stay unchanged; the hicolor names follow the app id `org.ubunye.creative.light`.

| File | What |
|---|---|
| `lightcraft.svg`, `lightcraft-small.svg` | copies of the source SVG |
| `lightcraft-1024.png` | 1024 px render |
| `lightcraft-macos-512.png` | runtime window/Dock icon on macOS (embedded by `apps/lightcraft/src/main.rs`) |
| `lightcraft.icns` | macOS bundle icon |
| `lightcraft.ico` | Windows icon, 16-256 px, embedded in `lightcraft.exe` by `apps/lightcraft/build.rs` |
| `hicolor/<n>x<n>/apps/org.ubunye.creative.light.png` | Linux icon theme, 16-512 px; the 256 px one is also the runtime window icon on Windows and Linux |
| `hicolor/scalable/apps/org.ubunye.creative.light.svg` | Linux scalable icon |

To change the icon, replace `ubunye/brand/assets/icon.svg` and run `python -I ubunye/brand/apply.py`
(it rerenders through `ubunye/brand/render_icons.py`, which needs resvg and Pillow). Licence: `LICENSE.txt`.
