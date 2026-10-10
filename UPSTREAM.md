# Upstream

Ubunye Creative Light (`ubunye-creative-light`) is a downstream distribution of **LightCraft** by the ArtCraft team and the LightCraft contributors: https://github.com/storytold/lightcraft.
It is used under MIT OR Apache-2.0. The upstream LICENSE, NOTICE and ATTRIBUTION files in this repository are kept unchanged.

## Remotes

| Remote | URL | Use |
|---|---|---|
| `origin` | https://github.com/ubunye-ai-ecosystems/ubunye-creative-light.git | our repository |
| `upstream` | https://github.com/storytold/lightcraft.git | read-only; push is disabled locally |

```bash
git remote add upstream https://github.com/storytold/lightcraft.git
git remote set-url --push upstream DISABLED
git fetch upstream --tags
```

## Branches

| Branch | Meaning |
|---|---|
| `upstream-main` | exact copy of `upstream/main`, fast-forwarded by the sync bot only |
| `integration/upstream` | rolling integration branch; the bot merges new upstream into it and keeps one PR open into `develop` |
| `develop` | default branch; all Ubunye work lands by PR |
| `release/X.Y` | stabilisation branches |
| `main` | latest stable Ubunye release |

Rules: never force-push; never rebase Ubunye work onto upstream; bring upstream in with merge commits; merge integration PRs only after human review and green checks.

## Sync

`.github/workflows/ubunye-upstream-sync.yml` runs daily at 03:17 UTC. It:
1. fetches upstream;
2. compares with `UPSTREAM_STATE.json`;
3. fast-forwards `upstream-main`;
4. merges into `integration/upstream`;
5. opens or updates the PR with a report from `ubunye/scripts/upstream_report.py`.

`UPSTREAM_STATE.json` only changes on `develop` when that PR is merged.

To run a sync by hand:
```bash
git fetch upstream
python3 -I ubunye/scripts/upstream_report.py --from "$(jq -r .last_integrated.sha UPSTREAM_STATE.json)" --to upstream/main --out /tmp/report.md
git switch -c integration/upstream origin/develop && git merge --no-ff upstream/main
```

## Ubunye patches against upstream

Every upstream file we modify is listed in `ubunye/patched-paths.txt`. Keep that list short. Prefer adding new files under `ubunye/` and offering generic hooks upstream.
