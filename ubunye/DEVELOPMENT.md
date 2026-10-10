# Developing Ubunye Creative Light

## Toolchain
`rust-toolchain.toml` pins Rust 1.97.0 (upstream MSRV is 1.90). rustup installs it automatically.

## Prerequisites
- **Linux (Ubuntu 22.04+):** `sudo apt-get install -y build-essential pkg-config libxkbcommon-dev libwayland-dev libx11-dev libxrandr-dev libxi-dev libgl1-mesa-dev`
- **macOS 11+:** Xcode Command Line Tools
- **Windows 10+:** Visual Studio Build Tools with the MSVC C++ workload
- **Optional:** the `craft-fonts` checkout for CJK glyphs: `git clone https://github.com/storytold/craft-fonts ../craft-fonts` (an Ubunye mirror is planned), then `CRAFT_FONTS_DIR=../craft-fonts`

## Build and run
```bash
cargo run --release -p lightcraft            # desktop app
cargo build --release -p lightcraft-cli         # CLI + MCP server
cargo xtask ci                           # upstream's full local gate (fmt, clippy, tests, layers, …)
cargo xtask web --serve 8080             # browser build (needs wasm32 target + wasm-bindgen-cli =0.2.129)
```

## Tests
- `cargo test --workspace --locked`: unit and integration tests.
- `ubunye/scripts/smoke.sh <path-to-lightcraft-cli>`: headless import → edit → save → reopen → export.
- GPU tests skip without an adapter. On Linux, install `mesa-vulkan-drivers` (lavapipe) to run them on the CPU.

## Verified build log

| Date | Platform | Command | Result |
|---|---|---|---|
| 2026-10-10 | Linux x86_64, 4 vCPU, rustc 1.97.0, no GPU | `cargo build --locked -p lightcraft-cli` | ✅ 13 min 41 s; smoke test passed (CPU path) |
