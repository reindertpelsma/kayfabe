#!/usr/bin/env bash
# POST_CAPTURE_HOOK for boot_capture.sh (fat guest on kf3): run the apps in $APPS inside the
# guest, one process each, and for EACH app persist
#   $APPS_OUT/<app>.guest.log         the app's own output (from inside the guest)
#   $APPS_OUT/<app>.guest_dmesg.log   the guest dmesg lines this app added (NVRM / Xid)
#   $APPS_OUT/<app>.kf3.log           the kf3 device's stderr lines this app added
# and append its APPRES line (plus KF3/XID counters) to $APPS_OUT/guest.res.
# Stops at the first app after which the guest no longer answers (verdict GUEST_DEAD);
# apps_matrix.sh reboots and continues with the rest.
set -uo pipefail
TAG=${1:?tag}
HERE="$(cd "$(dirname "$0")" && pwd)"; G="$HERE/../bench/gssh_nv"
OUT=${APPS_OUT:?APPS_OUT}; mkdir -p "$OUT"
QLOG=${BENCH_DIR:-/workspace/bench}/run_${TAG}_qemu.log
$G true >/dev/null 2>&1 || { echo "APPS_HOOK guest unreachable at start"; exit 0; }
$G 'sudo tee /opt/apps/bundle/run_apps.sh >/dev/null' < "$HERE/run_apps.sh"
# the tree's python drivers, so a harness fix does not need a re-provisioned image
# host-built probes added after the image was provisioned (same binaries the host lane ran)
tar -C /workspace/apps/bundle -cf - bin | $G 'sudo tar -C /opt/apps/bundle -xf -'
for f in "$HERE"/src/*.py; do $G "sudo tee /opt/apps/bundle/share/$(basename "$f") >/dev/null" < "$f"; done
$G 'sudo rm -rf /opt/apps/out/guest'
echo "APPS_HOOK tag=$TAG apps=[$APPS] guest=$($G 'nvidia-smi --query-gpu=name,driver_version,persistence_mode --format=csv,noheader' 2>&1 | head -1)"
[ "${APPS_GUEST_PM:-0}" = 1 ] && echo "APPS_GUEST_PM=$($G 'sudo nvidia-smi -pm 1 2>&1 | tail -1')"
for app in $APPS; do
  q0=$(wc -l < "$QLOG" 2>/dev/null || echo 0)
  d0=$($G 'sudo dmesg | wc -l' 2>/dev/null | tr -d '\r'); d0=${d0:-0}
  res=$(timeout "${APPS_APP_TMO:-2400}" "$G" "sudo bash /opt/apps/bundle/run_apps.sh guest $app" 2>&1 | tr -d '\r')
  src=$?
  alive=1; $G true >/dev/null 2>&1 || { sleep 20; $G true >/dev/null 2>&1 || alive=0; }
  line=$(grep -a '^APPRES ' <<<"$res" | tail -1)
  if [ -z "$line" ]; then
    v=HANG; [ $alive = 0 ] && v=GUEST_DEAD
    line="APPRES side=guest app=$app verdict=$v rc=ssh$src secs=- quiet=- note=no-result-line(guest_alive=$alive)"
  fi
  if [ $alive = 1 ]; then
    $G "sudo cat /opt/apps/out/guest/$app.log" > "$OUT/$app.guest.log" 2>&1
    $G "sudo dmesg | tail -n +$((d0+1))" > "$OUT/$app.guest_dmesg.log" 2>&1
  fi
  tail -n +"$((q0+1))" "$QLOG" 2>/dev/null | tail -3000 > "$OUT/$app.kf3.log"
  nx=$(grep -c 'Xid' "$OUT/$app.guest_dmesg.log" 2>/dev/null); nx=${nx:-0}
  nr=$(grep -ciE 'refus' "$OUT/$app.kf3.log" 2>/dev/null); nr=${nr:-0}
  nk=$(wc -l < "$OUT/$app.kf3.log")
  echo "$line boot=$TAG guest_xid=$nx kf3_lines=$nk kf3_refusals=$nr" | tee -a "$OUT/guest.res"
  grep -a '^APPDIG ' <<<"$res" | tee -a "$OUT/guest.dig"
  [ $alive = 0 ] && { echo "APPS_HOOK guest dead after $app — stopping this boot"; break; }
done
echo "APPS_HOOK_DONE"
