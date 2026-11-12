#!/usr/bin/env bash
# Glue only: shared-PC lock + log around the Python stress run. Usage: run_stress.sh ROOT [mc_stress.py run args]
set -u
ROOT=${1:?root}; shift
HERE=$(cd "$(dirname "$0")" && pwd)
LOG=/root/prov/PC_ACTIVE.log
flock /root/prov/pc_gpu.lock bash -c '
  echo "$(date -u +%FT%TZ) minecraft-stress START MC client+server, GNOME Xwayland :0 as ubuntu, dir $0" >> '"$LOG"'
  python3 -B -I "$1" run --root "$0" "${@:2}"; rc=$?
  echo "$(date -u +%FT%TZ) minecraft-stress END rc=$rc" >> '"$LOG"'; exit $rc' "$ROOT" "$HERE/mc_stress.py" "$@"
