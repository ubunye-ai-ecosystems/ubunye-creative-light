#!/usr/bin/env bash
# The version of the kopanod built from <kopano lab dir>, as four numbers: the lab's workspace
# version (Cargo.toml) and the number of commits that changed the lab. Every app's installer
# carries it so the newest kopanod on a computer wins and none is ever replaced by an older one
# (Windows file version, Linux alternatives priority, macOS kopanod.version).
#
# Usage: ubunye/packaging/kopanod-version.sh <kopano lab dir>   prints e.g. 0.0.1.65
# Needs the git history of the lab (CI: fetch-depth 0); without it the last number is 0.
set -euo pipefail
LAB="${1:?kopano lab dir}"
# awk, not sed: the same on macOS (BSD) and Linux
v="$(awk '/^\[/ { p = ($0 == "[workspace.package]") } p && /^version *=/ { gsub(/^[^"]*"|".*$/, ""); print; exit }' "$LAB/Cargo.toml")"
[ -n "$v" ] || { echo "error: no [workspace.package] version in $LAB/Cargo.toml" >&2; exit 1; }
n="$(git -C "$LAB" rev-list --count HEAD -- . 2>/dev/null || echo 0)"
echo "$v.$n"
