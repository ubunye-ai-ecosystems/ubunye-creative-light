# Agent instructions: Ubunye layer for Ubunye Creative Light

These rules apply in addition to upstream's `AGENTS.md`/`CLAUDE.md`. Where they conflict, **these win**.

1. **Never force-push**, and never rewrite history on `develop`, `release/*`, `main` or `upstream-main`.
2. **Never commit to `upstream-main`.** Only the sync workflow fast-forwards it.
3. Bring upstream in only with **merge commits** through `integration/upstream` → PR → human approval. Never auto-merge.
4. **Prefer new files under `ubunye/`** over editing upstream files. If you must edit an upstream file, add it to `ubunye/patched-paths.txt` and keep the diff minimal.
5. **Never remove or alter** LICENSE-MIT, LICENSE-APACHE, NOTICE, ATTRIBUTION.md, contributor credits or copyright headers. When you substantially modify an upstream file, add a line under its header: `// Modified by Ubunye AI Ecosystems, <year>.`
6. Branding goes through `ubunye/brand.toml` and the brand layer only. Never hand-replace product names across the tree.
7. Do not add AI/ML or Ubunye Engine dependencies without an approved plan (Milestone 7).
8. Do not touch release signing, R2 or ThabangVision deployment config without explicit approval.
9. The upstream `plan/` directory and the `../craftrules` repo referenced by upstream docs are **not available**. Don't rely on them.
10. Before pushing: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace --locked`, and `ubunye/scripts/smoke.sh`.
