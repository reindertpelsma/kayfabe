#!/bin/bash
# diag.sh TAG [GRPS] [WORDS] -- one bounded diagnostic boot of the gate's enabled image (bisection of the
# caps page). GRPS/WORDS empty and OVERLAY=1: the whole own-die page; OVERLAY unset and both empty: no overlay.
set -u
TAG=$1 GRPS=${2:-} WORDS=${3:-}
cd /root/kayfabe || exit 2
export WINVM_ROOT=/workspace/winvm
if [ -n "$GRPS$WORDS" ] || [ "${OVERLAY:-0}" = 1 ]; then
  export KF3_DISPLAY_CAPS_OVERLAY_FILE=${CAPSFILE:-/root/bwwin/caps_read_box.txt}
  [ -n "$GRPS" ] && export KF3_DISPLAY_CAPS_OVERLAY_GROUPS=$GRPS
  [ -n "$WORDS" ] && export KF3_DISPLAY_CAPS_OVERLAY_WORDS=$WORDS
fi
echo "DIAG_START tag=$TAG groups=$GRPS words=$WORDS $(date -u -Is)"
bash scripts/bench/windows/win_vm.sh run --overlay gate_bwwin1 --guest-driver 580.65.06 \
  --kf3-extra gpu-uuid=GPU-fa4efa4e-fa4e-4fa4-8fa4-fa4efa4e0016 \
  --env-file scripts/bench/windows/kf3_flags.sample.env --tag "$TAG" --detach > /dev/null 2>&1
L=$WINVM_ROOT/kfwin/logs/${TAG}_b1_qemu.log
t0=$(date +%s)
while [ $(( $(date +%s) - t0 )) -lt ${WAIT_S:-240} ]; do
  grep -aqE "console shows head|ACTIVE raster|armed its first head" "$L" 2>/dev/null && { sleep 20; break; }
  sleep 10
  pgrep -x qemu-system-x86 > /dev/null || { echo "QEMU_GONE"; tail -2 "$WINVM_ROOT/kfwin/logs/${TAG}_run.log"; break; }
done
grep -a "DIAGNOSTIC caps overlay" "$L" | cut -c1-200
grep -aE "armed its first head|ACTIVE raster|updates completed|console shows|STOPPED" "$L" | cut -c1-160 | sort | uniq -c | head -8
echo "CTRL $(grep -ao 'cmd=0x00731152\|cmd=0x0073011d\|cmd=0x0073010b\|cmd=0x20800af1\|cmd=0x20800af2' "$L" | sort | uniq -c | tr '\n' ' ')"
grep -a "disp\[" "$L" | tail -1 | grep -o "disp\[[^]]*\]"
timeout 100 bash scripts/bench/windows/win_vm.sh stop > /dev/null 2>&1
pgrep -x qemu-system-x86 > /dev/null && echo "QEMU_STILL_UP" || echo "DIAG_END tag=$TAG $(date -u -Is)"
gzip -c "$L" > /root/bwwin/${TAG}_b1_qemu.log.gz
