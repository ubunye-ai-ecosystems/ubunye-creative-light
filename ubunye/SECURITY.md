# Security policy: Ubunye Creative Light

## Reporting
Please **do not** open public issues for vulnerabilities. Use GitHub's private reporting ("Security" → "Report a vulnerability") on `ubunye-ai-ecosystems/ubunye-creative-light`. We acknowledge within 5 working days.
If the issue also affects upstream LightCraft, we coordinate disclosure with upstream (https://github.com/storytold/lightcraft).

## Supported versions
Only the latest `vX.Y.Z-ubunye.N` release. Pre-1.0 builds are previews.

## Threat model summary
- **Untrusted files.** Every importer (DNG, CR2, CR3, ARW, NEF, RAF, ORF, PEF, RW2, SRW, TIFF, JPEG, PNG, HEIF, XMP, Lightroom .lrcat, presets) parses attacker-controlled data with in-house Rust parsers. Fuzzing status: proptest robustness suites only; no coverage-guided fuzzing yet (Milestone 2.5).
- **Local automation.** The `--control` port and the MCP server can read and write files. They are off by default; never expose them beyond localhost. **The upstream control port has no authentication** (fix planned in Milestone 2.7).
- **Downloads.** Optional AI model downloads are user-initiated and hash-pinned upstream.
- **Supply chain.** `deny.toml` + `cargo audit` in CI (`ubunye-audit.yml`); toolchain pinned; Actions pinned by SHA in Ubunye workflows.

## Known advisories at import (cargo audit, 2026-10-09 DB)
- RUSTSEC-2023-0071 `rsa` 0.9.10, plus RUSTSEC-2026-0049/0098/0099/0104 `rustls-webpki` 0.102.8, both via `rustls-rustcrypto` 0.0.2-alpha (`crates/fetch`). Fix: switch to the rustls default provider.
- RUSTSEC-2026-0194/0195 `quick-xml` 0.38.4 (XMP parsing). Fix: bump to 0.41.
