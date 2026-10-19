# Share & Collaborate (Window > Share & Collaborate)

People see this as ThabangVision collaboration. Inside the code it is the Kopano protocol
(`crates/ui-egui/src/ubunye/kopano*.rs`), but the window never shows the words "Kopano" or
"protocol", and shows device ids and project ids only in the collapsed "Advanced (testing)" part.

One service per computer, `kopanod`, holds the sign-in (device certificate) and does all the
syncing. Light never syncs by itself: the window only talks to the service, and the service reads
and writes Light through Light's control port.

## What the window shows

- "Signed in to ThabangVision as <name>", or a "Sign in with ThabangVision" button when this
  computer has no certificate yet (for now it says "Sign-in is coming soon"), or "Collaboration is
  not running on this computer" with a plain hint
- the shared libraries, each with its teammates by name: a green dot when online, grey when not
- recent activity, for example "Member1 changed photos · 3 s ago" or "Up to date with Alice"
- **Share this library**: a name and the teammate's ThabangVision account
- **Advanced (testing)**: your device id and the project ids (with Copy), and Join by ids.
  ThabangVision invites by username will replace this part.

It opens from the Window menu and from the two-people button in the top bar.

## Commands

| Command | What it does |
|---|---|
| `kopano.panel {open?, advanced?}` | shows or hides the window (and the Advanced part) |
| `kopano.share {name, account}` | shares this library; `account` may list several, comma separated |
| `kopano.join {project, from}` | joins a teammate's shared library into this window |
| `kopano.signIn` | the sign-in button (not built yet: says so) |
| `kopano.status` | what the window shows, as JSON (for agents and tests) |

## How the service finds this window

The service needs Light's control port. If Light was started with `--control PORT`, that port is
used. If not, the first Share or Join starts Light's own control server on a free port on
127.0.0.1 and gives that port to the service. Nothing listens until then.

## Defaults (the same as kopanod)

- API port: `KOPANO_API_PORT`, else 47100
- service folder (holds `api.token` and `cert.json`): `KOPANO_HOME`, else `%LOCALAPPDATA%\Kopano`
  on Windows, `~/Library/Application Support/Kopano` on macOS, `~/.local/share/kopano` on Linux

The window never blocks Light: it asks the service every 3 s on its own thread, keeps an event
stream open on another, and finds the service again by itself after a restart.
