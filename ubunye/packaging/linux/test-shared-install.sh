#!/usr/bin/env bash
# Proof that Ubunye apps share one kopanod (ThabangVision collaboration) on Linux, on real
# packages: install app A, then app B, remove A (kopanod, its unit and the running service
# stay, now from B's copy), remove B (all gone, the data folder stays). With one package: install
# and remove it. Runs as root (sudo) on a throwaway machine or container; CI only.
#
# Usage: test-shared-install.sh <A.deb|A.rpm> [<B.deb|B.rpm>]
# The service is started as the calling user ($SUDO_USER, else root) through systemd's user
# manager when there is one (a CI VM, after enable-linger), else directly (a container).
set -euo pipefail
A="$(realpath "$1")"
B="$(realpath "${2:-/dev/null}")"
[ "$B" = /dev/null ] && B=
case "$A" in
  *.deb) KIND=deb ;;
  *.rpm) KIND=rpm ;;
  *) echo "error: $A is not a .deb or .rpm" >&2; exit 2 ;;
esac
U="${SUDO_USER:-root}"
UHOME="$(getent passwd "$U" | cut -d: -f6)"
PORT=47100
fail() { echo "FAIL: $*" >&2; exit 1; }
step() { echo; echo "== $*"; }
name_of() { if [ $KIND = deb ]; then dpkg-deb -f "$1" Package; else rpm -qp --qf '%{NAME}' "$1"; fi; }
install_pkg() { if [ $KIND = deb ]; then apt-get install -y -qq "$1" >/dev/null; else dnf install -y -q "$1" >/dev/null; fi; }
remove_pkg() { if [ $KIND = deb ]; then apt-get remove -y -qq "$1" >/dev/null; else dnf remove -y -q "$1" >/dev/null; fi; }
as_user() { if [ "$U" = root ]; then "$@"; else sudo -u "$U" HOME="$UHOME" XDG_RUNTIME_DIR="/run/user/$(id -u "$U")" "$@"; fi; }

USER_SYSTEMD=
if [ "$U" != root ] && command -v loginctl >/dev/null && [ -d /run/systemd/system ]; then
  loginctl enable-linger "$U"
  for _ in $(seq 1 20); do [ -S "/run/user/$(id -u "$U")/bus" ] && break; sleep 0.5; done
  if as_user systemctl --user show-environment >/dev/null 2>&1; then USER_SYSTEMD=1; fi
fi
echo "packages: $KIND; user $U; user systemd: ${USER_SYSTEMD:-no}"

answers() { as_user /usr/bin/kopanod call --api-port $PORT --json '{"op":"hello"}' 2>/dev/null | grep -q signed_in; }
wait_answers() { for _ in $(seq 1 30); do answers && return 0; sleep 1; done; return 1; }
start_service() {
  if [ -n "$USER_SYSTEMD" ]; then
    as_user systemctl --user daemon-reload
    as_user systemctl --user start thabangvision-collab.service
  else
    as_user sh -c "/usr/bin/kopanod run --api-port $PORT >/tmp/kopanod-test.log 2>&1 &"
  fi
  wait_answers || { cat /tmp/kopanod-test.log 2>/dev/null; as_user journalctl --user -u thabangvision-collab -n 30 2>/dev/null || true; fail "kopanod does not answer"; }
}
running() { pgrep -f '^/usr/(bin|lib/[a-z-]+)/kopanod( |$)' >/dev/null; }
check_shared() { # <package expected to provide it>
  [ -L /usr/bin/kopanod ] || fail "/usr/bin/kopanod is not a link"
  t="$(readlink -f /usr/bin/kopanod)"
  [ "$t" = "/usr/lib/$1/kopanod" ] || fail "/usr/bin/kopanod -> $t, expected /usr/lib/$1/kopanod"
  for u in /usr/lib/systemd/user/thabangvision-collab.service /usr/lib/systemd/user/default.target.wants/thabangvision-collab.service; do
    [ -e "$u" ] || fail "$u missing"
    grep -q '^ExecStart=/usr/bin/kopanod run' "$u" || fail "$u is not the kopanod unit"
  done
  echo "ok: /usr/bin/kopanod -> $t, start-at-sign-in unit in place"
}
check_gone() {
  [ ! -e /usr/bin/kopanod ] && [ ! -L /usr/bin/kopanod ] || fail "/usr/bin/kopanod still there"
  [ ! -e /usr/lib/systemd/user/thabangvision-collab.service ] || fail "unit still there"
  [ ! -L /usr/lib/systemd/user/default.target.wants/thabangvision-collab.service ] || fail "unit still enabled"
  sleep 1
  ! running || { pgrep -af kopanod; fail "kopanod still running"; }
  [ -d "$UHOME/.local/share/kopano" ] || fail "the data folder $UHOME/.local/share/kopano was removed"
  echo "ok: kopanod, its unit and the service are gone; the data folder stays"
}

NA="$(name_of "$A")"
step "install $NA"
install_pkg "$A"
check_shared "$NA"
start_service
echo "ok: the service answers"
if [ -z "$B" ]; then
  step "remove $NA"
  remove_pkg "$NA"
  check_gone
  echo; echo "PASS ($KIND, one app)"
  exit 0
fi
NB="$(name_of "$B")"
step "install $NB as well (no file conflict)"
install_pkg "$B"
[ -L /usr/bin/kopanod ] || fail "/usr/bin/kopanod is not a link"
echo "ok: both installed; /usr/bin/kopanod -> $(readlink -f /usr/bin/kopanod) (the newer, or either when equal)"
answers || fail "installing $NB stopped the service"
step "remove $NA: $NB keeps collaboration"
remove_pkg "$NA"
check_shared "$NB"
answers || fail "removing $NA stopped the service $NB uses"
running || fail "kopanod is not running after removing $NA"
echo "ok: the service still answers"
if [ -n "$USER_SYSTEMD" ]; then
  as_user systemctl --user daemon-reload
  as_user systemctl --user restart thabangvision-collab.service
  wait_answers || fail "the unit does not start from $NB's copy"
  echo "ok: the unit restarts from $NB's copy"
fi
step "remove $NB: the last app"
remove_pkg "$NB"
check_gone
step "install $NB then $NA (the other order), remove $NB, then $NA"
install_pkg "$B"
install_pkg "$A"
remove_pkg "$NB"
check_shared "$NA"
remove_pkg "$NA"
check_gone
echo; echo "PASS ($KIND, $NA + $NB)"
