# Rebranding LightCraft → Ubunye Creative Light

Status: **inventory only.** Branding changes start after the brand kit is approved (Milestone 4).

## Obligations
- The ArtCraft name, wordmark and logos (`docs/brand/`) are proprietary. Upstream's `docs/brand/LICENSE-brand.txt` requires them to be **removed** from any distributed modified version.
- We may say "Based on LightCraft by the ArtCraft team" in plain text. That credit appears in About, the README and the website.
- LICENSE-MIT, LICENSE-APACHE, NOTICE and ATTRIBUTION stay unchanged. Our notice is added alongside them, not in place of them.

## Target identity (from `brand.toml`)
| Item | Upstream | Ubunye |
|---|---|---|
| Product name | LightCraft | Ubunye Creative Light |
| App / bundle / Flatpak id | `ai.storyteller.lightcraft` | `org.ubunye.creative.light` |
| Vendor / manufacturer | Learning Machines LLC | Ubunye AI Ecosystems |
| Windows MSI UpgradeCode | upstream GUID | `9210D947-FF7A-4C2A-817A-EE355C6A0E4F` |
| Env var prefix | `LIGHTCRAFT_` | `UBUNYE_LIGHT_` (keep reading `LIGHTCRAFT_` as a fallback) |
| Links | getartcraft.com, discord.gg/artcraft, storytold | thabangvision.com/studio, ubunye-ai-ecosystems |
| AppImage update info | `gh-releases-zsync\|storytold\|lightcraft\|…` | downloads.thabangvision.com zsync |

## Touchpoints (from the Phase-1 inventory)
- "LightCraft": about 1,201 occurrences, 526 of them in locale files
- `APP_ID` in `apps/lightcraft/src/main.rs`; `packaging/macos/Info.plist.in`; `packaging/linux/ai.storyteller.lightcraft.*`; `packaging/windows/lightcraft.wxs` (UpgradeCode, Manufacturer, registry key, ARP URLs)
- icons in `assets/app-icon/` (lynx); ArtCraft logos in `docs/brand/`
- 32 `LIGHTCRAFT_*` env vars; config dir names; XMP namespace `ns.lightcraft.app` (**keep reading it** for compatibility)
- AppImage update info in `packaging/linux/package.sh`

## How (keeps upstream merges cheap)
1. **Brand layer.** One module (`crates/ui-egui/src/brand.rs` or a new `ubunye-brand` crate) exposes the name, ids, links and logos, generated from `ubunye/brand.toml` by `build.rs`. Upstream call sites get a one-line indirection. Offer that hook upstream.
2. **Locale overlay.** Load `ubunye/i18n/<lang>.*` after upstream catalogs to override product-name strings, instead of editing upstream locale files.
3. **Packaging overlay.** `ubunye/packaging/` holds templated Info.plist, WiX, nfpm and Flatpak files rendered from `brand.toml`. Our release workflow uses these; upstream `packaging/` stays untouched.
4. **Brand-leak gate.** CI greps for `ArtCraft|getartcraft|ai\.storyteller|Learning Machines|storytold` outside the allow-list (LICENSE*, NOTICE, ATTRIBUTION*, UPSTREAM*, credits). It is informational until M4, then blocking.
