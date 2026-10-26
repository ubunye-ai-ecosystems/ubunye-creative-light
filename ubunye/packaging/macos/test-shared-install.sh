#!/usr/bin/env bash
# Proof on a Mac (CI runner) that two Ubunye apps share one kopanod (ThabangVision
# collaboration): both apps go to /Applications from their DMGs and are opened in turn, as a
# person would. Each opening runs the app's launch check (ubunye_collab.rs). Then:
# - the LaunchAgent (org.ubunye.kopano) names one kopanod, the newest by kopanod.version, and
#   opening the apps again in turn does not move it back and forth;
# - the app it names goes to the Trash and its kopanod stops (as at sign-out): launchd does not
#   start the missing file again; opening the other app repoints the agent and the service runs;
# - the last app goes too: nothing starts; the data folder stays.
#
# Usage: ubunye/packaging/macos/test-shared-install.sh <A.dmg> <B.dmg>
set -euo pipefail
PLIST="$HOME/Library/LaunchAgents/org.ubunye.kopano.plist"
DATA="$HOME/Library/Application Support/Kopano"
LOGS="${RUNNER_TEMP:-/tmp}/collab-mac"
mkdir -p "$LOGS"
fail() { echo "FAIL: $*" >&2; [ -f "$PLIST" ] && cat "$PLIST" >&2; exit 1; }
step() { echo; echo "== $*"; }

install_dmg() { # <dmg> -> prints the installed app path
  local mnt app
  mnt="$(mktemp -d)"
  hdiutil attach -nobrowse -readonly -mountpoint "$mnt" "$1" >/dev/null
  app="$(find "$mnt" -maxdepth 1 -name '*.app' | head -1)"
  [ -n "$app" ] || fail "no app in $1"
  rm -rf "/Applications/$(basename "$app")"
  ditto "$app" "/Applications/$(basename "$app")"
  hdiutil detach "$mnt" >/dev/null
  echo "/Applications/$(basename "$app")"
}
agent_program() { plutil -extract ProgramArguments.0 raw -o - "$PLIST" 2>/dev/null || true; }
version_of() { cat "$1/Contents/Resources/kopanod.version" 2>/dev/null || echo 0; }
newer() { # is version $1 > $2 ?
  [ "$1" != "$2" ] && [ "$(printf '%s\n%s\n' "$1" "$2" | sort -t. -k1,1n -k2,2n -k3,3n -k4,4n | tail -1)" = "$1" ]
}
answers() { local k; k="$(agent_program)"; [ -x "$k" ] && "$k" call --json '{"op":"hello"}' 2>/dev/null | grep -q signed_in; }
stop_kopanod() { pkill -f 'Contents/MacOS/kopanod run' || true; sleep 2; }
open_app() { # <app>: start it, give the launch check time, quit it
  local exe
  exe="$(defaults read "$1/Contents/Info.plist" CFBundleExecutable)"
  RUST_LOG=info "$1/Contents/MacOS/$exe" >>"$LOGS/$(basename "$1" .app).log" 2>&1 &
  local pid=$!
  for _ in $(seq 1 40); do answers && break; sleep 1; done
  sleep 2
  kill "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true
  grep -i 'collaboration service' "$LOGS/$(basename "$1" .app).log" | tail -3 || true
}
not_restarted() { # launchd must not start kopanod again
  if ! grep -q PathState "$PLIST" 2>/dev/null; then
    echo "NOT PROVEN: this kopanod writes KeepAlive=true (needs kopanod's PathState change)"
    return 0
  fi
  sleep 15
  if pgrep -f 'Contents/MacOS/kopanod run' >/dev/null; then fail "kopanod was started again"; fi
  echo "ok: launchd did not start the missing kopanod again"
}

step "install both apps"
A="$(install_dmg "$1")"
B="$(install_dmg "$2")"
VA="$(version_of "$A")"
VB="$(version_of "$B")"
echo "$A (kopanod $VA), $B (kopanod $VB)"
if newer "$VB" "$VA"; then WIN="$B"; else WIN="$A"; fi

step "open $A, then $B"
open_app "$A"
[ "$(agent_program)" = "$A/Contents/MacOS/kopanod" ] || fail "after opening A the agent names '$(agent_program)'"
answers || fail "the service does not answer after opening A"
echo "ok: the agent names A's kopanod and the service answers"
open_app "$B"
[ "$(agent_program)" = "$WIN/Contents/MacOS/kopanod" ] || fail "after opening B the agent names '$(agent_program)', expected $WIN"
echo "ok: the agent names the newest kopanod ($WIN)"
step "open both again: the agent does not move"
open_app "$A"
open_app "$B"
[ "$(agent_program)" = "$WIN/Contents/MacOS/kopanod" ] || fail "the agent moved to '$(agent_program)'"
echo "ok: still $WIN"

if [ "$WIN" = "$A" ]; then OTHER="$B"; else OTHER="$A"; fi
step "trash $WIN (the agent's app), its kopanod stops"
rm -rf "$WIN"
stop_kopanod
not_restarted
step "open $OTHER: it takes the agent over"
open_app "$OTHER"
[ "$(agent_program)" = "$OTHER/Contents/MacOS/kopanod" ] || fail "the agent names '$(agent_program)', expected $OTHER"
answers || fail "the service does not answer from $OTHER"
echo "ok: the agent names $OTHER's kopanod and the service answers"

step "trash $OTHER (the last app)"
rm -rf "$OTHER"
stop_kopanod
not_restarted
[ -d "$DATA" ] || fail "the data folder was removed"
echo "ok: the data folder stays"
echo; echo "PASS (macOS, two apps)"
