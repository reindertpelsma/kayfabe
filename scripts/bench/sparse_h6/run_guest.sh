#!/bin/bash
# E3 guest launcher (EXPERIMENT claude/sparse-hw-20261026). Mirrors LX2's phase-3 QEMU argv (K arm).
#   run_guest.sh <tag> <qemu-binary> <h6run list, comma separated> [VAR=VAL ...]
# Holds /tmp/kayfabe-fastguest.lock for the boot only; polls `flock -n` every 300 s; stops with QMP
# system_powerdown only (this launcher's own QEMU). Writes logs/e3guest/<tag>.* and the Xid delta.
set -u
R=/data/claude-sparse-20261026
TAG=$1; BIN=$2; RUN=$3; shift 3
O=$R/logs/e3guest; mkdir -p "$O"
exec 9>/tmp/kayfabe-fastguest.lock
waited=0
until flock -n -o 9; do sleep 300; waited=$((waited+300)); echo "lock busy, waited ${waited}s"; [ $waited -gt 7200 ] && { echo "GAVE_UP_LOCK"; exit 9; }; done
x0=$(dmesg | grep -c 'NVRM: Xid')
t0=$(date -u +%FT%TZ)
echo "RUN tag=$TAG bin=$BIN sha=$(sha256sum "$BIN" | cut -c1-16) run=$RUN env='$*' start=$t0 xid_before=$x0" | tee "$O/$TAG.meta"
rm -f "$O/$TAG.qmp"
TRIG="$O/$TAG.vahist-trigger"; rm -f "$TRIG"
envs=(env __GL_SHADER_DISK_CACHE=0 CUDA_CACHE_DISABLE=1 "KF3_DIAG_VAHIST_TRIGGER=$TRIG")
for kv in "$@"; do envs+=("$kv"); done
"${envs[@]}" taskset -c 0-7 "$BIN" -name "h6-$TAG" \
  -object memory-backend-memfd,id=ram0,size=8192M,share=on -machine q35,accel=kvm,memory-backend=ram0 \
  -m 8192 -cpu host -smp 8 -kernel $R/guest/vmlinuz -initrd $R/guest/initrd.img \
  -append "console=ttyS0 panic=0 loglevel=4 h6run=$RUN" \
  -device kf3-gpu,fb-mb=8192,bar1-size=134217728,bar2-size=33554432,id=kf0 \
  -display none -monitor none -serial stdio -qmp "unix:$O/$TAG.qmp,server=on,wait=off" \
  > "$O/$TAG.serial" 2> "$O/$TAG.qemu.err" &
QP=$!
echo "qemu pid $QP" | tee -a "$O/$TAG.meta"
# watcher: write the newest Xid text to the VAHIST trigger file (only read when KF3_DIAG_VAHIST=1)
( last=$x0; while kill -0 $QP 2>/dev/null; do
    n=$(dmesg | grep -c 'NVRM: Xid')
    if [ "$n" -gt "$last" ]; then dmesg | grep 'NVRM: Xid' | tail -1 > "$TRIG.tmp" && mv "$TRIG.tmp" "$TRIG"; last=$n; fi
    sleep 0.5; done ) &
WP=$!
deadline=$((SECONDS+${H6_TIMEOUT:-420}))
while kill -0 $QP 2>/dev/null && [ $SECONDS -lt $deadline ]; do sleep 2; done
if kill -0 $QP 2>/dev/null; then
  echo "TIMEOUT: asking own guest to power down (QMP system_powerdown)" | tee -a "$O/$TAG.meta"
  python3 - "$O/$TAG.qmp" <<'PY'
import socket,sys,json,time
s=socket.socket(socket.AF_UNIX); s.connect(sys.argv[1]); f=s.makefile('rw')
f.readline(); f.write(json.dumps({"execute":"qmp_capabilities"})+"\n"); f.flush(); f.readline()
f.write(json.dumps({"execute":"system_powerdown"})+"\n"); f.flush(); print(f.readline().strip())
PY
  d2=$((SECONDS+60)); while kill -0 $QP 2>/dev/null && [ $SECONDS -lt $d2 ]; do sleep 2; done
fi
if kill -0 $QP 2>/dev/null; then echo "STILL_RUNNING_AFTER_POWERDOWN pid=$QP" | tee -a "$O/$TAG.meta"; else wait $QP; echo "qemu exit rc=$?" | tee -a "$O/$TAG.meta"; fi
kill $WP 2>/dev/null
x1=$(dmesg | grep -c 'NVRM: Xid')
dmesg | grep 'NVRM: Xid' | tail -n $((x1-x0)) > "$O/$TAG.xids" 2>/dev/null
echo "END tag=$TAG xid_after=$x1 delta=$((x1-x0)) end=$(date -u +%FT%TZ)" | tee -a "$O/$TAG.meta"
