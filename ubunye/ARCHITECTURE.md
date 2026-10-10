# Ubunye Creative Light: architecture (Ubunye view)

Ubunye Creative Light is LightCraft at the upstream revision in `../UPSTREAM_STATE.json`, plus the Ubunye patch set in `patched-paths.txt`. For the upstream design, read upstream's own `docs/` and `AGENTS.md`. This file covers only what Ubunye adds and how it fits on top.

Full due-diligence report: `ubunye-creative-studio-tools/research/lightcraft.md`.

## Shape (upstream)

```
apps/lightcraft         desktop shell (egui/eframe 0.36 on wgpu 30)
apps/lightcraft-cli     headless CLI + MCP server (lightcraft-cli mcp)
apps/lightcraft-web     wasm32 static web build
crates/ui-egui      panels, menus, theme, brand, i18n
crates/engine       headless document model + command registry (every action is a command)
crates/*            colour, geometry, GPU, codecs and format crates (layering enforced by `cargo xtask layers`)
xtask               build, CI, packaging helpers
packaging/          macOS DMG, Windows MSI (WiX), Linux AppImage/deb/rpm/Flatpak, FreeBSD, web
```

## Ubunye layer

| Path | Purpose |
|---|---|
| `UPSTREAM.md`, `UPSTREAM_STATE.json` | upstream tracking |
| `rust-toolchain.toml` | pinned toolchain (upstream has none) |
| `deny.toml` | licence, advisory and source policy (cargo-deny) |
| `.github/workflows/ubunye-*.yml` | our CI, audit and sync; upstream workflows are left untouched |
| `ubunye/brand.toml` | Ubunye identity (app id, vendor, links); wired in during Milestone 4 |
| `ubunye/scripts/` | upstream report and smoke tests |

Principle: add files, don't edit upstream ones. Every unavoidable upstream edit is listed in `patched-paths.txt`.
