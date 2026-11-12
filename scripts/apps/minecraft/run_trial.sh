#!/usr/bin/env bash
# Glue only: shared-PC lock + log, then the Python trial. Usage: run_trial.sh ROOT [extra mc_trial.py args]
set -u
ROOT=${1:?root}; shift
HERE=$(cd "$(dirname "$0")" && pwd)
LOG=/root/prov/PC_ACTIVE.log
XA=$(ls /run/user/1000/.mutter-Xwaylandauth.* | head -1)
echo "$(date -u +%FT%TZ) minecraft-trial START MC client 1.21.8 on GNOME Xwayland :0 as ubuntu, dir $ROOT" >> "$LOG"
flock /root/prov/pc_gpu.lock runuser -u ubuntu -- env DISPLAY=:0 XAUTHORITY="$XA" XDG_RUNTIME_DIR=/run/user/1000 \
  python3 -B -I "$HERE/mc_trial.py" run --root "$ROOT" "$@"
rc=$?
echo "$(date -u +%FT%TZ) minecraft-trial END rc=$rc" >> "$LOG"
exit $rc
