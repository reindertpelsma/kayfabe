#!/bin/bash
# lane trio, extra phase H (2026-11-05): host CUDA loop + the Linux fast suite, no Windows guest (the owner's
# second criterion, "1 guest and the host simultaneously"). Runs while the Windows install (CPU only, no GPU) runs.
# Uses trio_stage.sh's host loop format. Log: /root/prov/trioH/phaseH.log, last line PHASEH_EXIT.
set -uo pipefail
EV=/root/prov/trioH
mkdir -p "$EV/bin"
exec >> "$EV/phaseH.log" 2>&1
trap 'echo "PHASEH_EXIT rc=$? $(date -u -Is)"' EXIT
cd /root/kayfabe || exit 1
REV=$(git rev-parse --short=8 HEAD)
echo "PHASEH_START rev=$REV $(date -u -Is) install_qemu=$(pgrep -x qemu-system-x86 | tr '\n' ' ')"
gcc -O0 -I scripts/bench/cuda_min -o "$EV/bin/cup8" scripts/bench/cup8.c -lcuda -lm || exit 2
gcc -O2 -o "$EV/bin/cup8bench" scripts/bench/cup8bench.c -lcuda -lm || exit 2
loop() { # PHASE N STOP
  local ph=$1 n=$2 stop=$3 i=0 t rc ok
  while [ ! -e "$stop" ]; do
    t=$(date +%s.%N); CUP8_N=$n timeout 900 "$EV/bin/cup8" > "$EV/cur_$ph" 2>&1; rc=$?; ok=0
    [ $rc = 0 ] && grep -aq 'CUP8 VERDICT: PASS' "$EV/cur_$ph" && ok=1; [ $ok = 1 ] || cp "$EV/cur_$ph" "$EV/fail_${ph}_cup8_$i.txt"
    echo "TRIO_HOST_ITER phase=$ph i=$i tool=cup8 n=$n rc=$rc ok=$ok secs=$(awk -v a=$t -v b=$(date +%s.%N) 'BEGIN{printf "%.2f",b-a}') utc=$(date -u -Is)" >> "$EV/host_$ph.log"
    [ -e "$stop" ] && break
    t=$(date +%s.%N); BENCH_SIZES=1024,2048 BENCH_ITERS=10 BENCH_BATCH=10 timeout 900 "$EV/bin/cup8bench" > "$EV/cur_$ph" 2>&1; rc=$?; ok=0
    [ $rc = 0 ] && grep -aq 'BENCH_VERDICT: PASS' "$EV/cur_$ph" && ok=1; [ $ok = 1 ] || cp "$EV/cur_$ph" "$EV/fail_${ph}_bench_$i.txt"
    echo "TRIO_HOST_ITER phase=$ph i=$i tool=cup8bench sizes=1024,2048 rc=$rc ok=$ok secs=$(awk -v a=$t -v b=$(date +%s.%N) 'BEGIN{printf "%.2f",b-a}') utc=$(date -u -Is)" >> "$EV/host_$ph.log"
    i=$((i+1))
  done
}
snap() { echo "SNAP $1 $(date -u -Is) $(nvidia-smi --query-gpu=memory.used,memory.total --format=csv,noheader,nounits) bar1_used=$(nvidia-smi -q -d MEMORY | awk '/BAR1/{f=1} f&&/Used/{print $3; exit}') apps=$(nvidia-smi --query-compute-apps=pid,used_memory --format=csv,noheader,nounits | tr -d ' ' | tr '\n' ';')" >> "$EV/snap.log"; }
xid0=$(dmesg | grep -c 'NVRM: Xid')
snap idle
# 1) host solo, 120 s (beside the CPU-only install)
rm -f "$EV/stopS"; (sleep 120; touch "$EV/stopS") & loop solo 8192 "$EV/stopS"
echo "SOLO units_ok=$(grep -c ' ok=1 ' "$EV/host_solo.log") bad=$(grep -c ' ok=0 ' "$EV/host_solo.log")"
# 2) host loop + fast suite
rm -f "$EV/stopH"; loop H 8192 "$EV/stopH" & LP=$!
(while [ ! -e "$EV/stopH" ]; do snap H; sleep 15; done) &
t0=$(date +%s)
KF_LOCK=/tmp/kf-trio-linux.lock KF_GPU_IDLE_MIB=999999 KF_DEVICE=kf3 BENCH_DIR=/workspace/bench \
  bash scripts/fastguest/fast_suite.sh trioH 180 > "$EV/fast_H.run" 2>&1
echo "SUITE rc=$? secs=$(( $(date +%s) - t0 ))"
touch "$EV/stopH"; wait
grep '^FAST_SUITE_PASS' /workspace/bench/trioH_suite.out
cp /workspace/bench/trioH_suite.out "$EV/"
echo "HOST_H units_ok=$(grep -c ' ok=1 ' "$EV/host_H.log") bad=$(grep -c ' ok=0 ' "$EV/host_H.log")"
dmesg | grep 'NVRM: Xid' | tail -n "+$((xid0+1))" > "$EV/xid_H.txt"
echo "XID new=$(grep -c . "$EV/xid_H.txt")"
rm -rf "$EV/bin" "$EV"/cur_*
