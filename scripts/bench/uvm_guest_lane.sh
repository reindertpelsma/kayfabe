#!/usr/bin/env bash
# ★★★ The guest fault-plane lane (`docs/design/V3_UVM_GUEST_FAULT_PLANE.md` §8, E-M2) — ONE boot of
# the kf3 fat guest running `uvm_guest_hook.sh`, with the host nvidia-uvm in the state named by
# MODE, at THIS checkout's revision (boot_capture.sh selects kf3-bins/<rev8>/).
#
#   MODE=efs    the b3 patched module, uvm_efs_enable=1, and KF3_UVM_EFS=1 in the VMM (the plane ON)
#   MODE=off    the b3 patched module loaded, but KF3_UVM_EFS unset (the default: plane OFF)
#   MODE=stock  the stock module, KF3_UVM_EFS=1 (the probe must REFUSE by name; plane OFF)
#
# Writes /root/prov/uvmg_<tag>.log with a START line and an EXIT rc= line (a detached run is
# waited on by its EXIT line, never by absence of output).
#   usage: MODE=efs bash scripts/bench/uvm_guest_lane.sh <tag>
set -uo pipefail
TAG=${1:?tag}
[[ "$TAG" =~ ^[a-zA-Z0-9][a-zA-Z0-9._-]*$ ]] || { echo "invalid tag" >&2; exit 2; }
HERE="$(cd "$(dirname "$0")" && pwd)"; SRC="$(cd "$HERE/../.." && pwd)"
MODE=${MODE:-efs}
KO=${EFS_KO:-/root/efs/patched/nvidia-uvm.ko}
OUT=/root/prov/uvmg_${TAG}.log
mkdir -p /root/prov
[ ! -e "$OUT" ] || { echo "refusing to overwrite $OUT" >&2; exit 2; }
exec >"$OUT" 2>&1
trap 'rc=$?; echo "EXIT rc=$rc $(date -Is)"' EXIT
echo "START $(date -Is) tag=$TAG mode=$MODE rev=$(git -C "$SRC" rev-parse HEAD)"
if pgrep -x qemu-system-x86 >/dev/null; then echo "a QEMU is running — refusing"; exit 2; fi
if ss -tln | grep -q ':2223 '; then echo "port 2223 in use — refusing"; exit 2; fi
case $MODE in
  efs|off)
    rmmod nvidia_uvm 2>/dev/null
    insmod "$KO" uvm_efs_enable=1 uvm_efs_timeout_ms="${EFS_TIMEOUT_MS:-30000}" || { echo "insmod failed"; exit 3; }
    echo "loaded $KO sha=$(sha256sum "$KO" | cut -c1-16) enable=$(cat /sys/module/nvidia_uvm/parameters/uvm_efs_enable) timeout_ms=$(cat /sys/module/nvidia_uvm/parameters/uvm_efs_timeout_ms)"
    ;;
  stock)
    rmmod nvidia_uvm 2>/dev/null
    modprobe nvidia_uvm || { echo "modprobe failed"; exit 3; }
    echo "stock nvidia_uvm: efs params present=$(ls /sys/module/nvidia_uvm/parameters/ | grep -c uvm_efs)"
    ;;
  *) echo "unknown MODE=$MODE"; exit 2 ;;
esac
if [ ! -x /workspace/bench/um_probe ]; then
  /usr/local/cuda/bin/nvcc -O2 -arch=sm_86 -cudart static -o /workspace/bench/um_probe \
      "$SRC/traces/v3_uvm_research/um_probe.cu" || { echo "nvcc failed"; exit 3; }
fi
echo "um_probe md5=$(md5sum < /workspace/bench/um_probe | cut -d' ' -f1)"
if [ "$MODE" = off ]; then unset KF3_UVM_EFS; else export KF3_UVM_EFS=1; fi
# HOOK selects the workload: uvm_guest_hook.sh (um_probe modes, default) or uvm_apps_hook.sh (the
# four managed-memory apps; build them first with uvm_apps_build.sh).
export KF_DEVICE=kf3 POST_CAPTURE_HOOK="$HERE/${HOOK:-uvm_guest_hook.sh}"
bash "$HERE/boot_capture.sh" "$TAG"
rc=$?
echo "BOOT_CAPTURE_RC=$rc"
Q=/workspace/bench/run_${TAG}_qemu.log
echo "--- kf3 fault-plane / EFS lines ($Q):"
grep -E 'UVM EFS|fault plane|EFS mirror|EFS:|EFS space' "$Q" | head -60
echo "--- the LAST fault-plane heartbeat:"
grep 'kf3: fault plane: regs=' "$Q" | tail -1
echo "--- hook verdicts:"
grep -hE '^CHECK |UM_RC=|^UVMAPP ' /workspace/bench/run_${TAG}_probe.log 2>/dev/null | head -40
exit $rc
