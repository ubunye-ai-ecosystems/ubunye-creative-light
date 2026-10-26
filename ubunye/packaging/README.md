# Installers with ThabangVision collaboration

One download per operating system installs Ubunye Light together with `kopanod`, the small
background service that does the collaboration (inside the code it is the Kopano protocol;
people only ever see "ThabangVision collaboration").

| System | Download | Where kopanod goes | Start at sign-in | Removed by |
|---|---|---|---|---|
| Windows | `.msi` (and a portable `.zip`) | `Program Files\ThabangVision\Collaboration` (zip: beside `lightcraft.exe`) | a machine-wide Run value, part of the MSI | uninstalling the last Ubunye app |
| macOS | `.dmg` | `Ubunye Light.app/Contents/MacOS/` (+ `Resources/kopanod.version`) | a LaunchAgent (made by the app) | moving the app to the Trash (see below) |
| Linux | `.deb`, `.rpm` | `/usr/lib/lightcraft/kopanod`, linked as `/usr/bin/kopanod` | a packaged systemd user unit | removing the last Ubunye app |
| Linux | `.AppImage`, `.tar.gz` | inside (AppImage: copied to `~/.local/share/kopano/bin`) | a systemd user unit (made by the app) | nothing (no uninstaller) |

Why this way: each one is the package Light already ships, with one more program inside. No
second download, no second installer, nothing to set up.

## One kopanod per computer

Ubunye Light, Photo and Film all carry kopanod. A computer runs one, whatever apps are installed
and in whatever order; removing one app never breaks another; removing the last one stops
kopanod and its start at sign-in; each person's data folder always stays. A newer kopanod
replaces an older one, never the other way round.

kopanod has its own version, `<lab version>.<commits to the lab>` (`kopanod-version.sh`, e.g.
`0.0.1.65`), the same whichever app is built with it.

- Windows: `windows/collab.wxs` is **the same file in all three apps** (component GUIDs, folder,
  Run value). Windows Installer counts the apps that installed that component and removes it with
  the last one. kopanod.exe's file version is kopanod's version, so the MSI never replaces a newer
  one with an older one. The MSI stops kopanod only when it removes or replaces the file. The Run
  value starts `kopanod run` for each person who signs in, with their own `%LOCALAPPDATA%\Kopano`.
- Linux `.deb` / `.rpm`: each package owns only `/usr/lib/<app>/` (no file clash between apps).
  `/usr/bin/kopanod` and `/usr/lib/systemd/user/thabangvision-collab.service` (enabled for every
  user) are `update-alternatives` links to the copy with the highest kopanod version
  (`linux/postinst.sh.in`). Removing an app drops its copy and the links fall back to another
  app's; after the last one the links are gone and `linux/prerm.sh.in` stops kopanod.
- macOS has no installer: the app makes the LaunchAgent (`kopanod autostart on`). An app repoints
  it only when the file it names is gone or its `kopanod.version` is older than the app's own, so
  apps never take it from each other in turn. The agent keeps kopanod alive only while that file
  exists, so a trashed app's kopanod is not started again; the next Ubunye app opened repoints it.
- AppImage / `.tar.gz`: as macOS (a per-user unit), unless an installed `/usr/bin/kopanod` exists:
  then the app uses that one.

## What happens on the first start

Light does it, as the signed-in person, silently (`apps/lightcraft/src/ubunye_collab.rs`):

1. finds the shared `kopanod` (Windows MSI, Linux package), else the one beside itself (a
   developer build has none: nothing happens);
2. `kopanod init`: makes this computer's device key (into Windows Credential Manager or the
   macOS Keychain);
3. start at sign-in: left to the installer where there is one (a per-user entry from an older
   build is removed with `kopanod autostart off`); otherwise `kopanod autostart on` as above;
4. starts kopanod if nothing answers yet (Linux package: `systemctl --user start
   thabangvision-collab`).

Until the person signs in with ThabangVision, `kopanod` only answers Light on 127.0.0.1 and
stays offline, so Share & Collaborate shows the "Sign in with ThabangVision" button.

## Proof

- `linux/test-shared-install.sh A.deb B.deb` (or `.rpm`): install A, then B, remove A
  (kopanod, its unit and the running service stay), remove B (all gone, data stays), and the
  other order. `windows/test-shared-install.ps1 A.msi B.msi` does the same with msiexec.
- CI: `.github/workflows/ubunye-collab-packages.yml` runs both with Light alone;
  `ubunye-collab-interop.yml` runs them with Light, Photo and Film packages from their own CI.

## Building

- Windows: `ubunye/packaging/windows/collab-package.ps1 -Arch x64 -KopanoLab <lab>`
- macOS: `ubunye/packaging/macos/collab-package.sh <lab> --arch universal`
- Linux: `ubunye/packaging/linux/collab-package.sh <lab>`

`<lab>` is `research/kopano/lab` of ubunye-creative-studio-tools. Each script builds `kopanod`
and then runs the normal `packaging/<os>/package.*` with `UBUNYE_EXTRA_BIN_DIR` set (a small
hook in those scripts). CI: `.github/workflows/ubunye-collab-packages.yml`.

## Not done yet (needs the owner)

- Code signing. Without it Windows Smart App Control blocks the programs on new PCs and macOS
  asks people to allow the app. The workflow has a marked place for the secrets; the release
  signing in `release.yml` is unchanged.
- Store accounts (Microsoft Store, Apple Developer ID) are not set up.
