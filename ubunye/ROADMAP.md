# Ubunye Creative Light: Ubunye roadmap

The upstream feature roadmap is in the root `ROADMAP.md` (owned by upstream). This file covers only Ubunye work. See `ubunye-creative-studio-tools/07-implementation-plan.md` for the full plan and acceptance criteria.

| Milestone | Status |
|---|---|
| M1: repository, history import, branches, docs | in progress |
| M2: 3-OS CI, cargo-deny/audit, smoke tests, fuzzing, SBOM, control-port auth | planned |
| M3: daily upstream sync (installed with M1; first integration PR after review) | planned |
| M4: branding (needs brand-kit approval) | blocked on approval |
| M5: signed release builds for macOS, Windows, Linux and web | planned |
| M6: ThabangVision distribution (downloads.thabangvision.com, /studio) | planned |
| M7: AI integration research | later |

Known issues inherited from upstream that we track: no 3-OS test CI upstream; unfuzzed parsers; unauthenticated control port; macOS Info.plist carries PhotoCraft UTIs; `crates/segment` and `crates/fetch` are Apache-2.0 only.
