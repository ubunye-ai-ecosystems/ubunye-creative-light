#!/bin/sh
# Ubunye: before the Ubunye Light .deb/.rpm is removed (not upgraded), stop ThabangVision
# collaboration (kopanod) and remove the start-at-sign-in unit Light made for each user.
# deb prerm gets "remove" (or "upgrade" ...); rpm %preun gets the number of versions left
# (0 = removed). The service folder (~/.local/share/kopano) stays, like Light's library.
case "$1" in
  remove | purge | 0) ;;
  *) exit 0 ;;
esac
for unit in /home/*/.config/systemd/user/kopano.service /root/.config/systemd/user/kopano.service; do
  [ -f "$unit" ] || continue
  if grep -q '/usr/bin/kopanod' "$unit"; then
    rm -f "$unit" "$(dirname "$unit")/default.target.wants/kopano.service"
  fi
done
if command -v pkill >/dev/null 2>&1; then
  pkill -f '^/usr/bin/kopanod ' || true
fi
exit 0
