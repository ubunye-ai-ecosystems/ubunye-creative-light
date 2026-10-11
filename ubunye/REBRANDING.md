# Rebranding LightCraft to Ubunye Light

Status: **applied** (2026-10-11, branch `ubunye/rebrand`). The brand is re-applied by a script, so
it survives daily upstream merges.

## Obligations
- The ArtCraft name, wordmark and logos are proprietary. Upstream's `docs/brand/LICENSE-brand.txt`
  requires them to be **removed** from any distributed modified version. The logo files are deleted
  (the licence text stays), and no logo is shown or compiled into the app, the web build or the README.
- We say "Based on LightCraft by the ArtCraft team" in plain text, linking to
  https://github.com/storytold/lightcraft. It appears in About, `--help`, the README and the nfpm description.
- LICENSE-MIT, LICENSE-APACHE and NOTICE are unchanged. `assets/ATTRIBUTION.md` only gets an appended
  Ubunye section (our icon). All upstream copyright lines stay.

## Identity (single source: `ubunye/brand.toml`)
| Item | Upstream | Ubunye |
|---|---|---|
| Product name | LightCraft | Ubunye Light (part of Ubunye Creative Studio, by Ubunye AI Ecosystems, distributed by ThabangVision) |
| App / bundle / Flatpak id | `ai.storyteller.lightcraft` | `org.ubunye.creative.light` |
| Vendor / manufacturer | Learning Machines LLC | Ubunye AI Ecosystems |
| Windows MSI UpgradeCode | `229BB552-…` | `9210D947-FF7A-4C2A-817A-EE355C6A0E4F` (from `ubunye-creative-studio-tools/brand/identities.toml`) |
| Links | getartcraft.com, discord.gg/artcraft, storytold | https://www.thabangvision.com/tools/light, https://www.thabangvision.com/tools, github.com/ubunye-ai-ecosystems/ubunye-creative-light |
| AppImage update info | `gh-releases-zsync\|storytold\|lightcraft\|…` | `zsync\|https://downloads.thabangvision.com/light/stable/ubunye-creative-light-linux-<arch>.AppImage.zsync` |
| Config folder | `LightCraft` / `lightcraft` | `org.ubunye.creative.light` on every platform (never reads upstream's folder) |
| Default library | `~/Pictures/LightCraft Library` | `~/Pictures/Ubunye Light Library` |

To rename the product, change `name` in `ubunye/brand.toml` and rerun the script. Nothing else holds the name.

## What changed
- **Brand layer:** `crates/engine/src/brand.rs` (generated from `brand.toml`) holds the name, tagline,
  ids and links. Config folder, app id, `--version`, Help links and About read it.
- **UI text in every locale:** "LightCraft" is replaced inside Rust string literals (never comments or
  identifiers) and in `crates/ui-egui/locales/*.json`, keys and values alike, so translation lookups keep
  matching. The ArtCraft/Discord labels are renamed (Questions and Ideas, Ubunye Creative Studio Website,
  …) with our translations from `ubunye/i18n/overlay.json`. Command ids (`app.discord`, `app.artcraft`)
  stay, pointing at our issue tracker and suite page.
- **About:** "Ubunye Light", version, "Ubunye Light, part of Ubunye Creative Studio.", the credit link,
  MIT OR Apache-2.0, the existing font/third-party notices and the Contributors and Models tabs.
- **Packaging:** WiX (name, Manufacturer, UpgradeCode, own component GUID, registry key
  `Software\Ubunye AI Ecosystems\Ubunye Light`, ARP links, Start menu shortcut), Info.plist (name,
  bundle id, UTIs; extensions unchanged), `.desktop`/metainfo/MIME/Flatpak files renamed to the new
  app id, nfpm vendor/maintainer/homepage, Windows VERSIONINFO, sign.ps1 description, nix, omastore,
  packaging-lint and release workflows.
- **Icons:** every file in `assets/app-icon/` is rendered from `ubunye/brand/assets/icon.svg`
  (`ubunye/brand/render_icons.py`, following `packaging/icons.sh`, macOS grid margin included).
  The web build gets it as an inline SVG favicon.
- **README:** a generated Ubunye header with the credit; ArtCraft logos, Discord badges and
  getartcraft links removed; product mentions renamed; the Crafting Apps section replaced by a short
  suite section.
- **CI:** the `brand-leak` job in `ubunye-ci.yml` now runs `apply.py --check` and is blocking.

## Deliberately kept (merge cost or compatibility)
- Crate and package names (`lightcraft-*`), Rust identifiers, binary names (`lightcraft`,
  `lightcraft-cli`, macOS executable `LightCraft`, `LightCraft.icns`), `LIGHTCRAFT_*` environment variables,
  file extensions, magic numbers, XMP namespaces, MCP tool names, test fixtures.
- Metadata written into users' files: `Software`/`xmptk`/DNG `UniqueCameraModel` = "LightCraft", the
  demo photos' creator. Changing them could break fixtures and old sidecar expectations; revisit later.
- Adobe names and references (Lightroom, Photoshop, `.lrcat` import, parity docs) are untouched.
- Upstream docs (`docs/`, whats-new, CHANGELOG, ROADMAP, AGENTS.md, CLAUDE.md), NOTICE, LICENSE files
  and contributor credits (`contributors/`).
- `storytold/craft-fonts` links: it is the font build input, not our product.
- The MSI still registers `App Paths\lightcraft.exe` (binary name unchanged), so it would collide with
  an installed upstream LightCraft on that one key.

## After every upstream merge
1. Resolve conflicts in brand-touched lines by taking upstream's side (the rules are written against
   upstream text).
2. `python -I ubunye/brand/apply.py` (re-applies everything; rerenders icons if needed, which needs
   resvg and Pillow).
3. `python -I ubunye/brand/apply.py --check` must pass. "drift" lines name rules whose upstream text
   changed: update the rule in `apply.py`, rerun.
4. Commit the result in the merge PR. New upstream strings that say "LightCraft" are picked up
   automatically; new ArtCraft or Discord strings show up as leaks in `--check`.

Upstream files we modify are listed in `ubunye/patched-paths.txt`.
